# SPDX-License-Identifier: GPL-3.0-only
#
# Barometer - a system monitor for the Windows taskbar
# Copyright (c) 2026 David Brustein
#
# Reads or sets the version, in the one place it is defined and the one place
# it is duplicated. Called by scripts\build.ps1 and by the build workflow;
# usable on its own.
#
#   version.ps1              print the current version
#   version.ps1 1.0.1        set it
#   version.ps1 -Bump patch  1.0.0 -> 1.0.1
#   version.ps1 -Bump minor  1.0.0 -> 1.1.0
#   version.ps1 -Bump major  1.1.0 -> 2.0.0
#
# The Windows app has its own numbering, starting at 1.0.0, and the macOS app
# has its own. They were kept in step while one repository held both, along
# with a `windows-v` tag prefix to tell the releases apart; the two have a
# repository each now, the tags here are plain `v1.0.0`, and forcing one
# ordering on two programs with different features only ever confused somebody.

param(
    [Parameter(Position = 0)][string]$Version,
    [ValidateSet('major', 'minor', 'patch')][string]$Bump
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path

$cargo = Join-Path $root 'Cargo.toml'
$iss = Join-Path $root 'installer\barometer.iss'

# Anchored to [workspace.package] rather than taking the first `version =` in
# the file. The two happen to be the same line today, and would stop being the
# same line the moment anybody adds a key above it - which is exactly the kind
# of edit nobody thinks to re-check the release script after.
$section = '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"'

function Get-Version {
    $text = [IO.File]::ReadAllText($cargo)
    $match = [Regex]::Match($text, $section)
    if (-not $match.Success) { throw "no [workspace.package] version found in $cargo" }

    return $match.Groups[1].Value
}

$current = Get-Version

if ($Bump) {
    if ($current -notmatch '^(\d+)\.(\d+)\.(\d+)') {
        throw "current version '$current' is not major.minor.patch, cannot bump"
    }

    $maj = [int]$Matches[1]; $min = [int]$Matches[2]; $pat = [int]$Matches[3]

    switch ($Bump) {
        'major' { $maj++; $min = 0; $pat = 0 }
        'minor' { $min++; $pat = 0 }
        'patch' { $pat++ }
    }

    $Version = "$maj.$min.$pat"
}

if (-not $Version) {
    Write-Host $current
    exit 0
}

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "'$Version' is not major.minor.patch"
}

# The workspace is the source of truth; every crate inherits from it, and the
# executable's own version resource comes from the same string by way of
# CARGO_PKG_VERSION. That matters more than it sounds: the updater compares the
# running version against a release tag, and a stale resource makes it offer an
# update the user already installed.
$text = [IO.File]::ReadAllText($cargo)
$match = [Regex]::Match($text, $section)
$span = $match.Groups[1]
$text = $text.Remove($span.Index, $span.Length).Insert($span.Index, $Version)
[IO.File]::WriteAllText($cargo, $text)

# Inno Setup cannot read Cargo.toml. The build passes /DAppVersion so the
# packaged version comes from Cargo.toml either way, but the script carries a
# fallback for anyone running ISCC by hand, and a fallback that says something
# different from the truth is worse than no fallback at all.
if (Test-Path $iss) {
    $text = [IO.File]::ReadAllText($iss)
    $text = [Regex]::Replace(
        $text,
        '(#define\s+AppVersion\s+)"[^"]+"',
        "`${1}`"$Version`""
    )
    [IO.File]::WriteAllText($iss, $text)
}

Write-Host "$current -> $Version"
