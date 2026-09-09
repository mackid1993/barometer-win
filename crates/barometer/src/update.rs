// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// The GitHub Releases updater, following Yamato's and Clicker's flow.
//
// The check is quiet unless it finds a newer release. A person chooses whether
// to download it. The installer is verified against the digest GitHub itself
// published before it is written anywhere it could be run from, and Barometer
// exits so the installer can replace the running executable.
//
// Nothing new in the dependency tree for any of it. HTTPS is WinHTTP, which is
// already carrying the weather; SHA-256 is BCrypt, which is in Windows. The
// alternative is a TLS stack, an async runtime and a hashing crate for
// something that happens once a day at most, and each of them is one more
// thing that has to be audited before an unsigned binary asks somebody to run
// an installer.

use barometer_core::net;
use serde_json::Value;

pub const OWNER: &str = "mackid1993";
pub const REPOSITORY: &str = "barometer-win";
const API_HOST: &str = "api.github.com";
const DOWNLOAD_HOST: &str = "github.com";

/// The tag prefix that marks a release as this program's.
///
/// Plain `v1.2.3`, because this repository holds one program. It used to hold
/// two - the macOS app and this one, which share a name and nothing else -
/// and the Windows releases were tagged `windows-v1.2.3` to keep the updater
/// from offering a DMG. That worked, but it was a workaround for a repository
/// shape rather than something either program wanted: the two are written in
/// different languages, do not have the same features, and release on their
/// own schedules, so forcing their version numbers into one ordering was
/// always going to confuse somebody. They have a repository each now.
const TAG_PREFIX: &str = "v";

/// A ceiling on what will be pulled down, so a wrong or hostile URL cannot
/// fill the disk. The installer is a few megabytes; this is generous.
const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024;

/// A three-part release number, compared numerically rather than as text, so
/// 1.3.10 is newer than 1.3.9 instead of older.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version([u32; 3]);

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version([major, minor, patch])
    }

    /// Reads a version out of a tag, with or without a leading v.
    ///
    /// Trailing text on a field is trimmed rather than rejected, so a
    /// pre-release tag like 1.2.0-beta still compares as 1.2.0. A field with
    /// no digits at all ends the parse: "1.2" is a version, "v" is not.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix(TAG_PREFIX).unwrap_or(text);
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let mut fields = text.split('.');
        let mut version = [0u32; 3];
        for (index, slot) in version.iter_mut().enumerate() {
            let Some(field) = fields.next() else { break };
            let digits: String = field.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                return (index != 0).then_some(Version(version));
            }
            *slot = digits.parse().ok()?;
        }
        Some(Version(version))
    }

    pub fn running() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(0, 0, 0))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [major, minor, patch] = self.0;
        write!(f, "{major}.{minor}.{patch}")
    }
}

/// One downloadable file from a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    /// The path part of the download URL. Kept split from the host because
    /// WinHTTP wants them separately, and because it is one fewer place a
    /// remote string could redirect the download somewhere else.
    pub path: String,
    pub sha256: String,
}

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The Markdown body GitHub publishes, shown in the offer so the decision
    /// is informed rather than a version number with no explanation.
    pub notes: String,
    assets: Vec<Asset>,
}

impl Release {
    /// The installer for this release, by the name the release workflow gives
    /// it. Matched exactly rather than by pattern: an asset whose name does
    /// not match is not an installer to be run.
    pub fn installer(&self) -> Option<&Asset> {
        let expected = format!("Barometer-Setup-{}.exe", self.version);
        self.assets.iter().find(|asset| asset.name == expected)
    }
}

/// What a check found. The running version is carried in the up-to-date case
/// so an explicitly requested check has an honest answer to give.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    UpToDate(Version),
    Newer(Box<Release>),
}

/// Parses GitHub's latest-release response.
///
/// Assets without an API-provided SHA-256 digest are dropped rather than
/// offered. Barometer ships unsigned, so this digest is the *only* thing
/// standing between a user and running whatever arrived over the wire; an
/// undigested asset is one that cannot be checked and therefore one that will
/// never be run.
pub fn parse_release(json: &str) -> Option<Release> {
    let root: Value = serde_json::from_str(json).ok()?;
    let version = Version::parse(root.get("tag_name")?.as_str()?)?;

    // The body is prose from this repository, but it is still remote input and
    // must not be able to make an unbounded dialog. The GitHub page remains the
    // place for anything past a generous sixteen thousand characters.
    let mut notes: String = root
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .take(16_000)
        .collect();
    if notes.trim().is_empty() {
        notes = "No release notes were provided.".to_string();
    }

    let mut assets = Vec::new();
    for asset in root.get("assets").and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = asset.get("name").and_then(Value::as_str) else { continue };
        let Some(url) = asset.get("browser_download_url").and_then(Value::as_str) else { continue };
        let Some(digest) = asset.get("digest").and_then(Value::as_str) else { continue };
        let Some(sha256) = digest.strip_prefix("sha256:").map(str::to_ascii_lowercase) else {
            continue;
        };
        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        // The host is checked rather than trusted. A release asset that claims
        // to live anywhere but GitHub's own download host is not one of ours,
        // whatever the rest of the JSON says.
        let Some(path) = download_path(url) else { continue };
        assets.push(Asset { name: name.to_string(), path, sha256 });
    }

    Some(Release { version, notes, assets })
}

/// The path part of a GitHub release download URL, if that is what it is.
///
/// Anything else - another host, another scheme, another repository - returns
/// None. This is the check that keeps a rewritten release description from
/// pointing the updater at somebody else's executable.
fn download_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    if host != DOWNLOAD_HOST {
        return None;
    }
    let expected = format!("{OWNER}/{REPOSITORY}/releases/download/");
    path.starts_with(&expected).then(|| format!("/{path}"))
}

/// A week, which is what the settings pane promises.
pub const AUTOMATIC_INTERVAL: u64 = 7 * 24 * 60 * 60;

/// Whether the automatic check is switched on and has not run this week.
///
/// The switch and its caption - "Check for updates automatically. Once a
/// week." - described nothing before: `check` had two callers and both were a
/// person pressing a button, so the setting, the "Skipping 1.4.0" row and
/// `should_offer` were all machinery for a mechanism that did not exist.
///
/// Both times are Unix seconds, and both are optional because a clock set
/// before 1970 has no answer and a file that has never been checked against
/// has no stamp.
pub fn due(enabled: bool, last: Option<u64>, now: Option<u64>) -> bool {
    if !enabled {
        return false;
    }
    let Some(now) = now else { return false };
    match last {
        // A stamp in the future means the clock has moved - a machine that
        // booted with a dead battery and then found a time server. Due,
        // rather than waiting out a week that may never end.
        Some(last) => last > now || now - last >= AUTOMATIC_INTERVAL,
        None => true,
    }
}

/// Yamato's rule: a skipped version stays hidden from automatic checks, but a
/// check the person asked for always reports what it found.
pub fn should_offer(version: &str, skipped: Option<&str>, asked_for: bool) -> bool {
    asked_for || skipped != Some(version)
}

/// Asks GitHub for the newest Windows release.
///
/// The list rather than `releases/latest`, because "latest" is whatever was
/// published most recently and that is not necessarily the highest version -
/// a patch to an older line published after a newer release would take it.
/// Thirty is plenty of history to find the newest among.
pub fn check() -> Result<Outcome, String> {
    let path = format!("/repos/{OWNER}/{REPOSITORY}/releases?per_page=30");
    let body = net::get(API_HOST, &path).map_err(|why| why.to_string())?;
    let running = Version::running();
    match newest_windows_release(&body) {
        Some(release) if release.version > running => Ok(Outcome::Newer(Box::new(release))),
        Some(_) | None => Ok(Outcome::UpToDate(running)),
    }
}

/// Picks the newest release tagged for Windows out of a list response.
///
/// Newest by version rather than by position. GitHub returns them in creation
/// order, which is not the same thing the moment a patch is published for an
/// older line.
pub fn newest_windows_release(json: &str) -> Option<Release> {
    let releases = serde_json::from_str::<Value>(json).ok()?;
    let mut best: Option<Release> = None;
    for entry in releases.as_array()?.iter() {
        // Drafts have no assets worth having, and a prerelease is not
        // something to push at somebody who did not ask for one.
        if entry.get("draft").and_then(Value::as_bool) == Some(true)
            || entry.get("prerelease").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(tag) = entry.get("tag_name").and_then(Value::as_str) else { continue };
        if !tag.starts_with(TAG_PREFIX) {
            continue;
        }
        let Some(release) = parse_release(&entry.to_string()) else { continue };
        if best.as_ref().is_none_or(|found| release.version > found.version) {
            best = Some(release);
        }
    }
    best
}

/// Downloads an asset and checks it against the digest GitHub published.
///
/// Returns the bytes only when they hash to what was promised. A mismatch is
/// never written to disk: the point of the check is that the file does not
/// exist anywhere it could be double-clicked before it has passed.
pub fn download(asset: &Asset) -> Result<Vec<u8>, String> {
    let bytes = net::get_bytes(DOWNLOAD_HOST, &asset.path, MAX_DOWNLOAD_BYTES)
        .map_err(|why| why.to_string())?;
    if !verify_digest(&bytes, &asset.sha256) {
        return Err("the download did not match its published checksum".to_string());
    }
    Ok(bytes)
}

/// Checks bytes against a hex SHA-256.
pub fn verify_digest(bytes: &[u8], expected: &str) -> bool {
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let Some(actual) = sha256(bytes) else { return false };
    // Compared without early exit. The timing of a checksum comparison is not
    // a realistic attack here, but writing the loop this way costs nothing and
    // means nobody has to decide whether it is.
    let expected = expected.to_ascii_lowercase();
    let mut difference = 0u8;
    for (a, b) in actual.iter().zip(expected.as_bytes()) {
        difference |= a ^ b;
    }
    difference == 0 && actual.len() == expected.len()
}

/// SHA-256, as lowercase hex, through BCrypt.
///
/// Windows' own implementation rather than a hashing crate. It is the one the
/// operating system already trusts, and it keeps a security-relevant primitive
/// out of the dependency tree of a program that asks people to run installers.
fn sha256(bytes: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash,
        BCryptHashData, BCryptOpenAlgorithmProvider, BCRYPT_SHA256_ALGORITHM,
    };

    // SAFETY: the algorithm and hash handles are closed on every path out, and
    // the digest buffer is exactly the size SHA-256 produces.
    unsafe {
        let mut algorithm = std::ptr::null_mut();
        if BCryptOpenAlgorithmProvider(&mut algorithm, BCRYPT_SHA256_ALGORITHM, std::ptr::null(), 0)
            != 0
        {
            return None;
        }

        let mut hash = std::ptr::null_mut();
        if BCryptCreateHash(algorithm, &mut hash, std::ptr::null_mut(), 0, std::ptr::null(), 0, 0)
            != 0
        {
            BCryptCloseAlgorithmProvider(algorithm, 0);
            return None;
        }

        let ok = BCryptHashData(hash, bytes.as_ptr(), bytes.len() as u32, 0) == 0;
        let mut digest = [0u8; 32];
        let finished =
            ok && BCryptFinishHash(hash, digest.as_mut_ptr(), digest.len() as u32, 0) == 0;

        BCryptDestroyHash(hash);
        BCryptCloseAlgorithmProvider(algorithm, 0);
        if !finished {
            return None;
        }
        Some(digest.iter().flat_map(|b| format!("{b:02x}").into_bytes()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers_not_as_text() {
        // The whole reason this is not a string comparison.
        assert!(Version::parse("1.3.10").unwrap() > Version::parse("1.3.9").unwrap());
        assert!(Version::parse("2.0.0").unwrap() > Version::parse("1.99.99").unwrap());
        assert_eq!(Version::parse("v1.2.3"), Version::parse("1.2.3"));
    }

    #[test]
    fn a_short_or_decorated_tag_still_parses() {
        assert_eq!(Version::parse("1.2"), Some(Version::new(1, 2, 0)));
        assert_eq!(Version::parse("1"), Some(Version::new(1, 0, 0)));
        // A pre-release suffix compares as the release it precedes, which is
        // the conservative answer: it is never offered over the real one.
        assert_eq!(Version::parse("1.2.0-beta.1"), Some(Version::new(1, 2, 0)));
    }

    #[test]
    fn something_that_is_not_a_version_is_rejected() {
        assert_eq!(Version::parse("v"), None);
        assert_eq!(Version::parse("latest"), None);
        assert_eq!(Version::parse(""), None);
    }

    const GOOD: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000001";

    fn feed(digest: &str, host: &str) -> String {
        format!(
            concat!(
                r#"{{"tag_name":"v9.9.9","body":"Fixed a thing.","assets":[{{"#,
                r#""name":"Barometer-Setup-9.9.9.exe","#,
                r#""browser_download_url":"https://{host}/mackid1993/barometer-win"#,
                r#"/releases/download/v9.9.9/Barometer-Setup-9.9.9.exe","#,
                r#""digest":"{digest}"}}]}}"#
            ),
            host = host,
            digest = digest
        )
    }

    #[test]
    fn a_well_formed_release_yields_its_installer() {
        let release = parse_release(&feed(GOOD, "github.com")).unwrap();
        assert_eq!(release.version, Version::new(9, 9, 9));
        assert_eq!(release.notes, "Fixed a thing.");
        let installer = release.installer().unwrap();
        assert_eq!(installer.name, "Barometer-Setup-9.9.9.exe");
        assert!(installer.sha256.ends_with("01"));
        assert!(installer.path.starts_with("/mackid1993/barometer-win/releases/download/"));
    }

    #[test]
    fn an_asset_with_no_digest_is_never_offered() {
        // Barometer ships unsigned; the digest is the only check there is.
        let json = concat!(
            r#"{"tag_name":"v9.9.9","assets":[{"name":"Barometer-Setup-9.9.9.exe","#,
            r#""browser_download_url":"https://github.com/mackid1993/barometer-win"#,
            r#"/releases/download/v9.9.9/Barometer-Setup-9.9.9.exe"}]}"#
        );
        assert!(parse_release(json).unwrap().installer().is_none());
    }

    #[test]
    fn the_weekly_check_is_due_once_a_week_and_never_when_it_is_switched_off() {
        const WEEK: u64 = AUTOMATIC_INTERVAL;
        let now = Some(1_800_000_000);
        // Never checked, so there is nothing to have been too recent.
        assert!(due(true, None, now));
        assert!(!due(false, None, now));
        assert!(!due(true, Some(1_800_000_000 - WEEK + 1), now));
        assert!(due(true, Some(1_800_000_000 - WEEK), now));
        // A clock that has just been corrected forward leaves a stamp in the
        // future; waiting out a week from there could be a very long wait.
        assert!(due(true, Some(1_900_000_000), now));
        // No clock, no answer, rather than checking on every tick.
        assert!(!due(true, None, None));
    }

    #[test]
    fn a_malformed_digest_is_never_offered() {
        assert!(parse_release(&feed("sha256:nothex", "github.com")).unwrap().installer().is_none());
        assert!(parse_release(&feed("md5:0011", "github.com")).unwrap().installer().is_none());
    }

    #[test]
    fn an_asset_hosted_anywhere_else_is_never_offered() {
        // This is what stops an edited release description pointing the
        // updater at somebody else's executable.
        for host in ["evil.example", "github.com.evil.example", "raw.githubusercontent.com"] {
            let release = parse_release(&feed(GOOD, host)).unwrap();
            assert!(release.installer().is_none(), "accepted {host}");
        }
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    #[test]
    fn an_asset_from_another_repository_is_never_offered() {
        let json = concat!(
            r#"{"tag_name":"v9.9.9","assets":[{"name":"Barometer-Setup-9.9.9.exe","#,
            r#""browser_download_url":"https://github.com/someone/else"#,
            r#"/releases/download/v9.9.9/Barometer-Setup-9.9.9.exe","#,
            r#""digest":"sha256:"#,
            "0000000000000000000000000000000000000000000000000000000000000001",
            r#""}]}"#
        );
        assert!(parse_release(json).unwrap().installer().is_none());
    }

    #[test]
    fn an_installer_named_for_another_version_is_not_this_releases_installer() {
        let json = concat!(
            r#"{"tag_name":"v9.9.9","assets":[{"name":"Barometer-Setup-1.0.0.exe","#,
            r#""browser_download_url":"https://github.com/mackid1993/barometer-win"#,
            r#"/releases/download/v9.9.9/Barometer-Setup-1.0.0.exe","#,
            r#""digest":"sha256:"#,
            "0000000000000000000000000000000000000000000000000000000000000001",
            r#""}]}"#
        );
        assert!(parse_release(json).unwrap().installer().is_none());
    }

    #[test]
    fn an_empty_release_body_still_says_something() {
        let json = r#"{"tag_name":"v1.0.0","body":"   ","assets":[]}"#;
        assert_eq!(parse_release(json).unwrap().notes, "No release notes were provided.");
    }

    #[test]
    fn release_notes_cannot_grow_without_bound() {
        let long = "x".repeat(40_000);
        let json = format!(r#"{{"tag_name":"v1.0.0","body":"{long}","assets":[]}}"#);
        assert_eq!(parse_release(&json).unwrap().notes.chars().count(), 16_000);
    }

    #[test]
    fn a_skipped_version_hides_from_the_automatic_check_but_not_a_requested_one() {
        assert!(!should_offer("1.2.3", Some("1.2.3"), false));
        assert!(should_offer("1.2.3", Some("1.2.3"), true));
        assert!(should_offer("1.2.4", Some("1.2.3"), false));
        assert!(should_offer("1.2.3", None, false));
    }

    #[test]
    fn the_digest_check_agrees_with_the_published_vectors() {
        // The empty string and "abc", which is where a hash implementation is
        // wrong if it is wrong at all.
        assert!(verify_digest(
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(verify_digest(
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
    }

    #[test]
    fn the_digest_check_is_case_insensitive_but_not_length_insensitive() {
        assert!(verify_digest(
            b"abc",
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        ));
        assert!(!verify_digest(b"abc", "ba7816bf"));
        assert!(!verify_digest(b"abc", ""));
    }

    #[test]
    fn a_changed_byte_fails_the_digest() {
        assert!(!verify_digest(
            b"abd",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn entry(tag: &str, name: &str, draft: bool, pre: bool) -> String {
        format!(
            concat!(
                r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":{pre},"assets":[{{"#,
                r#""name":"{name}","browser_download_url":"#,
                r#""https://github.com/mackid1993/barometer-win/releases/download/{tag}/{name}","#,
                r#""digest":"sha256:"#,
                "0000000000000000000000000000000000000000000000000000000000000001",
                r#""}}]}}"#
            ),
            tag = tag,
            name = name,
            draft = draft,
            pre = pre
        )
    }

    fn feed(entries: &[String]) -> String {
        format!("[{}]", entries.join(","))
    }

    #[test]
    fn a_release_without_an_installer_for_this_program_is_not_offered() {
        // The repositories are separate now, so a macOS build is not expected
        // to turn up in this feed at all. The asset check is still what makes
        // an offer real: a release exists here that has no Windows installer
        // attached whenever one is published before its build finishes, or
        // when a tag is pushed for something else entirely, and offering it
        // would send somebody to a download that is not there.
        let json = feed(&[entry("v1.2.3", "Barometer-1.2.3.dmg", false, false)]);
        let found = newest_windows_release(&json).unwrap();
        assert_eq!(found.version, Version::new(1, 2, 3));
        assert!(found.installer().is_none());
    }

    #[test]
    fn a_tag_that_is_not_a_version_is_passed_over() {
        // Anything without the prefix, and anything whose remainder is not a
        // version: a repository accumulates tags that are not releases.
        let json = feed(&[
            entry("nightly", "Barometer-Setup-9.9.9.exe", false, false),
            entry("v1.2.3", "Barometer-Setup-1.2.3.exe", false, false),
        ]);
        assert_eq!(newest_windows_release(&json).unwrap().version, Version::new(1, 2, 3));
    }

    #[test]
    fn the_newest_is_by_version_not_by_position() {
        // GitHub lists releases in creation order, which stops being the same
        // thing the moment a patch ships for an older line.
        let json = feed(&[
            entry("v1.2.0", "Barometer-Setup-1.2.0.exe", false, false),
            entry("v2.0.0", "Barometer-Setup-2.0.0.exe", false, false),
            entry("v1.2.1", "Barometer-Setup-1.2.1.exe", false, false),
        ]);
        assert_eq!(newest_windows_release(&json).unwrap().version, Version::new(2, 0, 0));
    }

    #[test]
    fn drafts_and_prereleases_are_left_alone() {
        let json = feed(&[
            entry("v3.0.0", "Barometer-Setup-3.0.0.exe", true, false),
            entry("v2.9.0", "Barometer-Setup-2.9.0.exe", false, true),
            entry("v1.0.0", "Barometer-Setup-1.0.0.exe", false, false),
        ]);
        assert_eq!(newest_windows_release(&json).unwrap().version, Version::new(1, 0, 0));
    }

    #[test]
    fn an_empty_or_unreadable_feed_finds_nothing_rather_than_guessing() {
        assert!(newest_windows_release("[]").is_none());
        assert!(newest_windows_release("not json").is_none());
        // A repository with nothing but tags that are not releases.
        assert!(newest_windows_release(&feed(&[entry("nightly", "x.exe", false, false)])).is_none());
    }
}
