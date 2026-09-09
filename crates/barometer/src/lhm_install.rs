// SPDX-License-Identifier: GPL-3.0-only
//
// Barometer - a system monitor for the Windows taskbar
// Copyright (c) 2026 David Brustein
//
// Fetching LibreHardwareMonitor, when somebody asks for it.
//
// AGENTS.md's decision stands unchanged: Barometer redistributes none of
// LibreHardwareMonitor. Nothing of it is in this binary, nothing of it is in
// the installer, and no MPL obligation is taken on. This module is the other
// half of that decision rather than a retreat from it - a person who wants
// temperatures should not have to go and find a zip, so pressing a button
// downloads the same release from the same place they would have downloaded
// it from themselves, into a directory that belongs to them.
//
// What that costs is that the download has to be trustworthy without a
// signature to lean on, which is why everything here is pinned. The release,
// the asset, its size and its digest are compile-time constants; nothing that
// arrives over the network chooses what is fetched, where it is written, or
// whether it is kept. That is AGENTS.md's rule for the updater, and it applies
// with more force here because the bytes are somebody else's.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use barometer_core::net;

use crate::update::verify_digest;

/// Upstream's repository, and the release this was pinned against.
///
/// **Not "latest".** Resolving the newest release at run time would mean the
/// digest could not be pinned, and an unpinned digest is no check at all: the
/// only thing standing between a user and whatever arrived over the wire would
/// be TLS and a hope that the release had not been re-cut. Moving to a newer
/// LibreHardwareMonitor is an edit to these four constants, made by somebody
/// who has looked at what they are moving to.
const OWNER: &str = "LibreHardwareMonitor";
const REPOSITORY: &str = "LibreHardwareMonitor";
const HOST: &str = "github.com";
pub const TAG: &str = "v0.9.6";

/// The asset to fetch, by name, size and SHA-256.
///
/// **`LibreHardwareMonitor.NET.10.zip`, and not `LibreHardwareMonitor.zip`.**
/// The release carries both. The plain one is built against .NET Framework
/// 4.7.2, and `barometer-sensors.exe` is .NET 10: loading that library into it
/// dies with a `TypeLoadException`. Both were tested against the real helper,
/// so this is measurement rather than preference. If a later release renames
/// the asset, the answer is a new pin against a build that was tried - never a
/// fallback to whichever asset happens to still be there.
pub const ASSET: &str = "LibreHardwareMonitor.NET.10.zip";
pub const ASSET_BYTES: u64 = 8_889_121;
pub const ASSET_SHA256: &str = "29739c4959b01b348fddad87664066634bcfd4f46e9bf41e4e916c318bcfdb99";

/// What the download is allowed to grow to.
///
/// The pinned size and a small margin. The ceiling is there so a wrong or
/// hostile response cannot fill the disk; the margin is there so a response
/// that is not the pinned asset is reported as the wrong size, which is a
/// thing the settings pane can explain, rather than as a transport failure,
/// which is not.
const DOWNLOAD_LIMIT: usize = ASSET_BYTES as usize + 64 * 1024;

/// The file the helper resolves everything else from.
///
/// The same name `helper/LibraryLocator.cs` requires. The two have to agree:
/// an install this module called successful and the helper then could not load
/// would be the worst of both, and there is nothing at run time to catch it.
pub const LIBRARY: &str = "LibreHardwareMonitorLib.dll";

/// How the helper is told where the library went.
///
/// `LibraryLocator` reads this from its environment, and the helper inherits
/// Barometer's, so exporting it before spawning is all the wiring an install
/// needs. Named here rather than in the caller because the string has to match
/// the helper's, and a constant in the module that knows the directory is the
/// place that will be looked at when it stops matching.
pub const HELPER_LIBRARY_VARIABLE: &str = "BAROMETER_LHM_DIR";

/// Barometer's own folder under the user's local application data, and the
/// installation inside it.
const APP_FOLDER: &str = "Barometer";
const INSTALL_FOLDER: &str = "LibreHardwareMonitor";

/// The unpack in progress, and the installation being replaced.
///
/// Both sit beside the target rather than in the system temp directory, for
/// two reasons. A rename is only atomic within a volume, and the user's temp
/// directory is not reliably on the same one; and a half-unpacked tree under
/// Barometer's own folder is something `uninstall` can clean up, which a
/// half-unpacked tree in `%TEMP%` is not.
const STAGING_FOLDER: &str = "LibreHardwareMonitor.incoming";
const SUPERSEDED_FOLDER: &str = "LibreHardwareMonitor.previous";

/// How far along an install is.
///
/// 8.9 MB on a hotel connection is long enough that a dialog with nothing
/// moving in it reads as hung, and the cancel button people reach for then is
/// Task Manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Bytes that have arrived, against the pinned size.
    ///
    /// Reported twice, at nothing and at everything, because `net::get_bytes`
    /// reads the whole body before it returns. A moving figure needs a
    /// callback inside that function, which is another file; until it has one
    /// the honest thing is a determinate bar that fills in one step rather
    /// than an animation that claims to know something it does not.
    Downloading { received: u64, total: u64 },
    Verifying,
    Extracting,
}

/// Why an install did not happen.
///
/// Separate cases rather than one string, because the settings pane says a
/// different thing to each of them: no network is "try again later", a failed
/// checksum is "this did not come from where it said it did", and a missing
/// tar.exe is a Windows too old for any of this to work at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Windows did not say where this account's data lives.
    NoInstallLocation,
    /// The download did not happen. Carries what the transport said.
    Unreachable(String),
    /// Something arrived, but not the pinned asset.
    WrongSize { expected: u64, received: u64 },
    /// The right number of bytes, and not the right bytes.
    Checksum,
    /// Windows' own archiver is not where it should be.
    NoArchiver(PathBuf),
    /// tar ran and refused the archive. Carries what it said.
    Unpack(String),
    /// It unpacked, and the library was not in it.
    NoLibrary,
    /// The disk refused something, with what was being attempted at the time.
    Filesystem { action: &'static str, why: String },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::NoInstallLocation => f.write_str(
                "Windows did not say where this account's application data is kept, \
                 so there is nowhere to install to",
            ),
            InstallError::Unreachable(why) => {
                write!(f, "LibreHardwareMonitor could not be downloaded from github.com: {why}")
            }
            InstallError::WrongSize { expected, received } => write!(
                f,
                "the download was {received} bytes, not the {expected} this release publishes, \
                 so nothing was installed"
            ),
            InstallError::Checksum => f.write_str(
                "the download did not match the checksum it was pinned to, \
                 so it was discarded and nothing was installed",
            ),
            InstallError::NoArchiver(path) => write!(
                f,
                "Windows' own archiver is missing: there is no tar.exe at {}. \
                 Windows has included one since Windows 10 1803",
                path.display()
            ),
            InstallError::Unpack(why) => write!(f, "the download could not be unpacked: {why}"),
            InstallError::NoLibrary => write!(
                f,
                "the download unpacked, but there was no {LIBRARY} in it"
            ),
            InstallError::Filesystem { action, why } => write!(f, "{action} failed: {why}"),
        }
    }
}

/// Where an install goes, if this account has anywhere to put one.
pub fn install_directory() -> Option<PathBuf> {
    Some(target_in(&local_app_data()?))
}

/// The installed library's directory, when there is one.
///
/// Presence of the DLL is the test, not presence of the directory: an install
/// interrupted before it finished, or half deleted afterwards, leaves a folder
/// that the helper would find nothing in. What the caller wants to know is
/// whether the helper can load it, so that is what is asked.
pub fn installed() -> Option<PathBuf> {
    let directory = install_directory()?;
    directory.join(LIBRARY).is_file().then_some(directory)
}

/// Whether the button should say Install or Reinstall.
pub fn is_installed() -> bool {
    installed().is_some()
}

/// The URL the download comes from.
///
/// Built from the pinned constants, so the address shown to a person who wants
/// to fetch it themselves is by construction the address Barometer would fetch
/// from, rather than a second copy of it that can drift.
pub fn download_url() -> String {
    format!("https://{HOST}{}", download_path())
}

/// Downloads, verifies and unpacks LibreHardwareMonitor.
///
/// Blocking, and long: it is a nine megabyte download and belongs on a worker
/// thread, never on the thread that owns the strip. On success the directory
/// returned holds the library and is what the helper wants as `--library`.
pub fn install(progress: Option<&dyn Fn(Progress)>) -> Result<PathBuf, InstallError> {
    let root = local_app_data().ok_or(InstallError::NoInstallLocation)?;
    install_into(&root, progress)
}

/// Removes the installation, and anything a previous attempt left beside it.
///
/// Already absent is success. The caller is asking for a state, not for an
/// event, and reporting a failure to delete what was not there would make
/// "remove it" fail on the second press.
pub fn uninstall() -> Result<(), InstallError> {
    let root = local_app_data().ok_or(InstallError::NoInstallLocation)?;
    // The helper has the library mapped into a live process for as long as it
    // runs, and Windows will not delete a file in that state. So it is closed
    // first, and reopened when this returns - by which time the directory is
    // gone, so it reopens against whatever installation the user made
    // themselves, or reports no library at all. Either way the strip is
    // telling the truth a second later, and nobody had to be told to turn the
    // Sensors module off first.
    let _helper_closed = barometer_core::sensors::library::hold();
    for directory in [target_in(&root), staging_in(&root), superseded_in(&root)] {
        match fs::remove_dir_all(&directory) {
            Ok(()) => {}
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => {
                return Err(InstallError::Filesystem {
                    action: "removing LibreHardwareMonitor",
                    why: why.to_string(),
                })
            }
        }
    }
    Ok(())
}

/// The whole flow, against a given application-data root.
///
/// Split from `install` so every path in it is expressed in terms of a root
/// that a test can supply, and so the one call that reads the environment
/// happens in exactly one place.
fn install_into(root: &Path, progress: Option<&dyn Fn(Progress)>) -> Result<PathBuf, InstallError> {
    let report = |stage: Progress| {
        if let Some(watcher) = progress {
            watcher(stage);
        }
    };

    // Asked before the download rather than after it. A machine with no
    // tar.exe cannot be unpacked to whatever happens next, and spending
    // somebody's connection on nine megabytes before telling them something
    // that was knowable in a syscall is rude.
    let archiver = archiver();
    if !archiver.is_file() {
        return Err(InstallError::NoArchiver(archiver));
    }

    report(Progress::Downloading { received: 0, total: ASSET_BYTES });
    let archive = net::get_bytes(HOST, &download_path(), DOWNLOAD_LIMIT)
        .map_err(|why| InstallError::Unreachable(why.to_string()))?;
    report(Progress::Downloading { received: archive.len() as u64, total: ASSET_BYTES });

    // Before a single byte of it reaches the disk. A zip that failed its
    // checksum must never exist anywhere something could open it, which
    // includes the staging directory: the file is the payload, and tar is the
    // thing that would run against it.
    report(Progress::Verifying);
    check_bytes(&archive, ASSET_BYTES, ASSET_SHA256)?;

    let staging = staging_in(root);
    let unpacked = staging.join("unpacked");

    // Whatever a previous attempt was killed in the middle of.
    let _ = fs::remove_dir_all(&staging);
    doing("creating the staging directory", fs::create_dir_all(&unpacked))?;

    let file = staging.join(ASSET);
    doing("writing the download", fs::write(&file, &archive))?;

    report(Progress::Extracting);
    // Held from here and not a moment earlier. Nine megabytes on a slow
    // connection is minutes, and there is no reason for temperatures to stop
    // while bytes are arriving - it is replacing the files that needs the
    // helper out of the way, not fetching them.
    //
    // A reinstall is the case that makes this necessary rather than tidy: the
    // running helper has the library it is about to be handed a new copy of,
    // and `place` renames the old directory aside. Without this the rename
    // leaves a `.previous` folder nothing can delete, and the helper carries
    // on reading the superseded copy from its new path, so a user who pressed
    // Reinstall gets no error and no new version either.
    let _helper_closed = barometer_core::sensors::library::hold();
    let placed = unpack_and_place(&archiver, &file, &unpacked, root);

    // Whatever happened. A half-unpacked tree that looks enough like an
    // installation to be found by something is the failure this staging
    // directory exists to prevent, so it does not outlive the attempt.
    let _ = fs::remove_dir_all(&staging);

    let directory = placed?;
    if !directory.join(LIBRARY).is_file() {
        return Err(InstallError::NoLibrary);
    }
    Ok(directory)
}

/// Unpacks the verified archive and moves the result into place.
///
/// Separate from `install_into` only so that its single caller can clean the
/// staging directory up on every path out of it, successful or not, without
/// repeating the cleanup at each `?`.
fn unpack_and_place(
    archiver: &Path,
    archive: &Path,
    unpacked: &Path,
    root: &Path,
) -> Result<PathBuf, InstallError> {
    unpack(archiver, archive, unpacked)?;
    let source = library_root(unpacked).ok_or(InstallError::NoLibrary)?;
    place(&source, root)
}

/// Moves an unpacked library into place, replacing whatever was there.
///
/// A rename rather than a copy, and that is the whole design: until this
/// succeeds the target directory is either the previous installation or
/// nothing at all, never a partial one. Windows will not rename over an
/// existing directory, so the old installation is moved aside first and only
/// deleted once the new one has landed - which also means a reinstall that
/// fails at the last step puts back what it displaced instead of leaving the
/// user with less than they started with.
fn place(source: &Path, root: &Path) -> Result<PathBuf, InstallError> {
    let target = target_in(root);
    let superseded = superseded_in(root);
    let _ = fs::remove_dir_all(&superseded);

    let replacing = target.exists();
    if replacing {
        doing("moving the previous installation aside", fs::rename(&target, &superseded))?;
    }
    if let Err(why) = fs::rename(source, &target) {
        if replacing {
            let _ = fs::rename(&superseded, &target);
        }
        return Err(InstallError::Filesystem {
            action: "moving the new library into place",
            why: why.to_string(),
        });
    }
    let _ = fs::remove_dir_all(&superseded);
    Ok(target)
}

/// Runs Windows' own tar over the archive.
fn unpack(archiver: &Path, archive: &Path, into: &Path) -> Result<(), InstallError> {
    // CREATE_NO_WINDOW, and it is not cosmetic. tar is a console program, so
    // spawning it plainly opens a black window on top of whatever the user was
    // doing. This project has had that bug once already, from the sensor
    // helper, and it is worse here: it happens while a dialog the user is
    // watching is on screen, and it steals the focus from it.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Extract, from this file, into that directory, and nothing else. There is
    // no flag here that would let the archive decide where its contents land -
    // and the digest check above is what makes even that safe, because tar is
    // only ever pointed at the exact bytes the pinned release published.
    let outcome = Command::new(archiver)
        .arg("-x")
        .arg("-f")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|why| InstallError::Unpack(why.to_string()))?;

    if outcome.status.success() {
        return Ok(());
    }
    // bsdtar explains itself on stderr, and passing that through is the
    // difference between "unpacking failed" and something somebody can act on.
    let complaint = String::from_utf8_lossy(&outcome.stderr).trim().to_string();
    Err(InstallError::Unpack(if complaint.is_empty() {
        format!("tar exited with {}", outcome.status)
    } else {
        complaint
    }))
}

/// The directory in the unpacked tree that actually holds the library.
///
/// Upstream's zip has its files at the root, but a zip that wraps everything
/// in a folder named after the release is common enough that following one
/// level down costs a `read_dir` and saves an install that unpacked perfectly
/// well and then reported nothing found. It also means the installed directory
/// is the one with the DLL in it either way, rather than sometimes being a
/// directory whose only content is another directory.
fn library_root(unpacked: &Path) -> Option<PathBuf> {
    if unpacked.join(LIBRARY).is_file() {
        return Some(unpacked.to_path_buf());
    }
    fs::read_dir(unpacked)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|child| child.join(LIBRARY).is_file())
}

/// Checks a download against what was pinned for it.
///
/// Size first, because it is free and because it separates "the server sent
/// something else entirely" - a proxy's login page, a truncated response -
/// from "these are the right number of bytes and the wrong ones", which is the
/// case that actually means something is wrong.
///
/// Pure, and takes its expectations as arguments, so the decision this whole
/// module turns on can be tested against known digests rather than only
/// against a nine megabyte file nobody can put in a test.
fn check_bytes(bytes: &[u8], expected: u64, digest: &str) -> Result<(), InstallError> {
    let received = bytes.len() as u64;
    if received != expected {
        return Err(InstallError::WrongSize { expected, received });
    }
    // The updater's, deliberately. One SHA-256 in this program, through
    // BCrypt, already reviewed - a second implementation would be a second
    // thing to get right in the one place where being wrong is silent.
    if !verify_digest(bytes, digest) {
        return Err(InstallError::Checksum);
    }
    Ok(())
}

/// The path part of the download URL. Split from the host because that is how
/// WinHTTP wants it, and because a path built from constants is one fewer
/// place a string could point the download somewhere else.
fn download_path() -> String {
    format!("/{OWNER}/{REPOSITORY}/releases/download/{TAG}/{ASSET}")
}

fn target_in(root: &Path) -> PathBuf {
    root.join(APP_FOLDER).join(INSTALL_FOLDER)
}

fn staging_in(root: &Path) -> PathBuf {
    root.join(APP_FOLDER).join(STAGING_FOLDER)
}

fn superseded_in(root: &Path) -> PathBuf {
    root.join(APP_FOLDER).join(SUPERSEDED_FOLDER)
}

fn local_app_data() -> Option<PathBuf> {
    let value = std::env::var_os("LOCALAPPDATA")?;
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// Windows' own tar, by absolute path.
///
/// bsdtar has been in System32 since Windows 10 1803 and reads zip, which is
/// why there is no zip crate in the dependency list and no unpacking code in
/// this file. Addressed absolutely and never through PATH: this is a program
/// Barometer is about to run, and PATH is writable by whoever controls the
/// session that started it.
fn archiver() -> PathBuf {
    archiver_in(&std::env::var("SystemRoot").unwrap_or_default())
}

fn archiver_in(system_root: &str) -> PathBuf {
    let root = system_root.trim();
    let root = if root.is_empty() { r"C:\Windows" } else { root };
    Path::new(root).join("System32").join("tar.exe")
}

fn doing<T>(action: &'static str, result: std::io::Result<T>) -> Result<T, InstallError> {
    result.map_err(|why| InstallError::Filesystem { action, why: why.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-256 of "abc", which is where a digest check is wrong if it is wrong
    /// at all.
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn the_pinned_asset_is_the_net_10_build_and_not_the_framework_one() {
        // The release carries both. The other one loads into a .NET 10 helper
        // and throws TypeLoadException, so this constant is load bearing and
        // this test is here to make a "simplification" of it fail loudly.
        assert_eq!(ASSET, "LibreHardwareMonitor.NET.10.zip");
        assert_ne!(ASSET, "LibreHardwareMonitor.zip");
    }

    #[test]
    fn the_pin_is_a_well_formed_sha_256() {
        assert_eq!(ASSET_SHA256.len(), 64);
        assert!(ASSET_SHA256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // The size the GitHub API reports for the pinned asset, spelled
        // out for the same reason the URL is: a wrong one here would turn
        // every download into a size mismatch and nothing would say why.
        assert_eq!(ASSET_BYTES, 8_889_121);
    }

    #[test]
    fn the_download_url_is_built_from_the_pin_and_nothing_else() {
        // The URL verified against the GitHub API, spelled out once so that
        // changing any constant it is made of has to be deliberate.
        assert_eq!(
            download_url(),
            "https://github.com/LibreHardwareMonitor/LibreHardwareMonitor\
             /releases/download/v0.9.6/LibreHardwareMonitor.NET.10.zip"
        );
        // And that the two halves the transport is handed reassemble into it,
        // since only the path is what actually gets requested.
        assert_eq!(download_url(), format!("https://{HOST}{}", download_path()));
        assert!(download_path().starts_with('/'));
        assert!(download_path().ends_with(ASSET));
        assert!(download_path().contains(TAG));
    }

    #[test]
    fn the_download_ceiling_admits_the_pinned_asset_and_little_more() {
        assert!(DOWNLOAD_LIMIT as u64 > ASSET_BYTES);
        assert!((DOWNLOAD_LIMIT as u64) < ASSET_BYTES * 2);
    }

    #[test]
    fn a_download_that_fails_its_checksum_is_never_unpacked() {
        // The right length and the wrong bytes: the only case where anything
        // downstream could have been fooled.
        assert_eq!(check_bytes(b"abd", 3, ABC), Err(InstallError::Checksum));
        assert_eq!(check_bytes(b"abc", 3, ABC), Ok(()));
    }

    #[test]
    fn a_download_of_the_wrong_size_is_rejected_before_it_is_hashed() {
        assert_eq!(
            check_bytes(b"ab", 3, ABC),
            Err(InstallError::WrongSize { expected: 3, received: 2 })
        );
    }

    #[test]
    fn the_real_pin_rejects_anything_that_is_not_the_asset() {
        // A proxy login page, a 404 body, an empty response.
        for body in [b"".as_slice(), b"<html>Sign in</html>".as_slice()] {
            assert!(matches!(
                check_bytes(body, ASSET_BYTES, ASSET_SHA256),
                Err(InstallError::WrongSize { .. })
            ));
        }
    }

    #[test]
    fn the_installation_lives_under_barometers_own_folder() {
        let root = Path::new(r"C:\Users\someone\AppData\Local");
        assert_eq!(
            target_in(root),
            Path::new(r"C:\Users\someone\AppData\Local\Barometer\LibreHardwareMonitor")
        );
    }

    #[test]
    fn the_staging_directory_sits_beside_the_installation_not_inside_it() {
        // Beside, because a rename is only atomic within a volume and only
        // possible at all if the source is not under the destination.
        let root = Path::new(r"C:\Users\someone\AppData\Local");
        let target = target_in(root);
        for other in [staging_in(root), superseded_in(root)] {
            assert_eq!(other.parent(), target.parent());
            assert_ne!(other, target);
            assert!(!other.starts_with(&target));
        }
        assert_ne!(staging_in(root), superseded_in(root));
    }

    #[test]
    fn the_archiver_is_windows_own_and_is_never_looked_up_on_path() {
        assert_eq!(archiver_in(r"C:\Windows"), Path::new(r"C:\Windows\System32\tar.exe"));
        // An environment with no SystemRoot still gets an absolute path, and
        // in particular never a bare "tar.exe" that PATH would resolve.
        for empty in ["", "   "] {
            let found = archiver_in(empty);
            assert!(found.is_absolute(), "{}", found.display());
            assert_eq!(found, Path::new(r"C:\Windows\System32\tar.exe"));
        }
    }

    #[test]
    fn the_helper_is_told_about_the_library_in_the_terms_it_looks_for_it() {
        // Both strings are helper/LibraryLocator.cs's, and an install this
        // module calls successful is one the helper has to be able to load.
        assert_eq!(LIBRARY, "LibreHardwareMonitorLib.dll");
        assert_eq!(HELPER_LIBRARY_VARIABLE, "BAROMETER_LHM_DIR");
    }

    #[test]
    fn each_failure_says_a_different_thing() {
        // The reason this is an enum: the settings pane has three different
        // things to say and cannot say them from one string.
        let failures = [
            InstallError::NoInstallLocation,
            InstallError::Unreachable("could not reach the service: WinHttpConnect".into()),
            InstallError::WrongSize { expected: ASSET_BYTES, received: 12 },
            InstallError::Checksum,
            InstallError::NoArchiver(PathBuf::from(r"C:\Windows\System32\tar.exe")),
            InstallError::Unpack("Damaged tar archive".into()),
            InstallError::NoLibrary,
            InstallError::Filesystem { action: "writing the download", why: "denied".into() },
        ];
        let said: Vec<String> = failures.iter().map(ToString::to_string).collect();
        for (index, message) in said.iter().enumerate() {
            assert!(!message.is_empty());
            assert!(!said[index + 1..].contains(message), "two failures say {message:?}");
        }
        // And that the three the pane distinguishes name what went wrong.
        assert!(said[1].contains("could not be downloaded"));
        assert!(said[3].contains("checksum"));
        assert!(said[4].contains("tar.exe"));
    }

    #[test]
    fn a_zip_that_wraps_everything_in_a_folder_still_installs_the_right_directory() {
        let scratch = Scratch::new("wrapped");
        let inner = scratch.path().join("LibreHardwareMonitor-0.9.6");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join(LIBRARY), b"not really a dll").unwrap();
        assert_eq!(library_root(scratch.path()), Some(inner));
    }

    #[test]
    fn a_zip_with_its_files_at_the_root_installs_that_root() {
        let scratch = Scratch::new("flat");
        fs::write(scratch.path().join(LIBRARY), b"not really a dll").unwrap();
        assert_eq!(library_root(scratch.path()), Some(scratch.path().to_path_buf()));
    }

    #[test]
    fn a_tree_with_no_library_in_it_is_not_an_installation() {
        let scratch = Scratch::new("empty");
        // Something that unpacked fine and is not what we asked for, including
        // a library two levels down, which is deeper than we will look.
        let deep = scratch.path().join("a").join("b");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join(LIBRARY), b"too deep").unwrap();
        assert_eq!(library_root(scratch.path()), None);
        assert_eq!(library_root(&scratch.path().join("nothing here")), None);
    }

    /// A directory of its own that goes away when the test does.
    ///
    /// The tests that need one are about what is on the disk after an unpack,
    /// and there is no way to ask that question without a disk. Nothing here
    /// touches the real installation directory or the network.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);

            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "barometer-lhm-{name}-{}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
