# SPDX-License-Identifier: GPL-3.0-only
#
# Barometer - a system monitor for the Windows taskbar
# Copyright (c) 2026 David Brustein
#
# Checks a built binary for local paths and anything else identifying the
# machine it was built on. Run by hand, against one binary at a time; exits
# non-zero on a hit. scripts\build.ps1 carries the same check inline over
# everything it stages, so a release cannot be packaged with them still in it -
# this is the one to reach for when a single binary is in question.
#
#   scrub-check.ps1 target\release\barometer.exe
#   scrub-check.ps1 dist\Barometer\barometer-sensors.exe
#
# Both binaries have carried these at some point and for different reasons.
# rustc bakes absolute source paths into panic messages and debug info, which
# is what --remap-path-prefix is for. The .NET helper embedded its own PDB path
# in the executable, which no remap touches and which needed <DebugType>none.
# A remap that silently stopped working, or a csproj setting somebody reverted,
# would both be invisible without this.

param([Parameter(Mandatory = $true)][string]$Binary)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $Binary)) {
    Write-Host "  $Binary does not exist" -ForegroundColor Red
    exit 1
}

$bytes = [IO.File]::ReadAllBytes($Binary)

# Strings land in a binary as both ASCII and UTF-16, and a path present only in
# the wide form would slip past an ASCII-only scan. The bytes are decoded in
# one call each rather than joined a character at a time: the pipeline form is
# correct and takes minutes on a one megabyte executable, which is long enough
# to read as a hang.
$ascii = [Text.Encoding]::ASCII.GetString($bytes)
$wide = [Text.Encoding]::Unicode.GetString($bytes)
$hay = $ascii + "`n" + $wide

# Things that identify this machine or this person. Deliberately broader than
# the source directory alone: a stray temp path or the cargo registry gives the
# same information away.
#
# Every one of these is a PATH, and that distinction is the whole design. The
# author's name is supposed to be in the binary - the version resource carries
# "(c) 2026 David Brustein. GNU GPL v3." and that is a copyright notice, not a
# leak. What must not be there is his name as a directory: this repository has
# lived under a folder named after him, inside a synced folder also named after
# him. Likewise the registry, which is fine as the remapped relative
# ".cargo\registry" and not fine as the absolute path it came from.
#
# Testing the bare name instead of the path form fails a correct build, which
# is worse than useless: a check that cries wolf is a check somebody starts
# passing -Force to.
$patterns = @(
    @{ Name = 'user profile'; Value = $env:USERPROFILE },
    @{ Name = 'user name in a path'; Value = "\$($env:USERNAME)\" },
    @{ Name = 'source tree'; Value = (Get-Location).Path },
    @{ Name = 'cargo registry'; Value = "$env:USERPROFILE\.cargo\registry" },
    @{ Name = 'rustup toolchain'; Value = "$env:USERPROFILE\.rustup\toolchains" },
    @{ Name = 'computer name'; Value = $env:COMPUTERNAME },
    @{ Name = 'sync folder'; Value = 'Dropbox' },
    @{ Name = 'author name as a directory'; Value = 'David Brustein\' }
)

$found = @()

foreach ($p in $patterns) {
    if ([string]::IsNullOrWhiteSpace($p.Value)) { continue }

    if ($hay.Contains($p.Value)) {
        $found += $p
    }
}

# Any drive-letter path at all, as a catch-all for something the named patterns
# missed. Reported rather than failed on, because a few are legitimate: the
# helper names the PawnIO device path and Barometer names the Windows directory
# it looks for tar.exe in.
$paths = [Regex]::Matches($hay, '[A-Za-z]:\\[A-Za-z0-9_\-\\. ]{6,}') |
    ForEach-Object { $_.Value } |
    Where-Object { $_ -notmatch '^[A-Za-z]:\\Windows' } |
    Select-Object -Unique

if ($found.Count -gt 0) {
    Write-Host "  FAILED. $(Split-Path -Leaf $Binary) contains:" -ForegroundColor Red
    foreach ($f in $found) {
        Write-Host ("    {0}: {1}" -f $f.Name, $f.Value) -ForegroundColor Red
    }
    Write-Host "  Check the --remap-path-prefix flags, and <DebugType> in the helper's csproj." -ForegroundColor Yellow
    exit 1
}

Write-Host ("  {0}: clean, no local paths, user name or machine name" -f (Split-Path -Leaf $Binary)) -ForegroundColor Green

if ($paths.Count -gt 0) {
    Write-Host "  absolute paths present (expected, not identifying):" -ForegroundColor DarkGray
    foreach ($p in $paths | Select-Object -First 8) {
        Write-Host "    $p" -ForegroundColor DarkGray
    }
}

exit 0
