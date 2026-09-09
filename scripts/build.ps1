# SPDX-License-Identifier: GPL-3.0-only
#
# Barometer - a system monitor for the Windows taskbar
# Copyright (c) 2026 David Brustein

<#
.SYNOPSIS
    Build Barometer, stage it, and package the installer.

.DESCRIPTION
    Three steps, each skippable, in the order they depend on each other:
    build the Rust binary and the .NET sensor helper, stage everything the
    installer expects into dist\Barometer, then run ISCC over it.

    The version is read from Cargo.toml and passed to Inno Setup rather than
    written in two places. The executable's own version resource comes from the
    same string by way of CARGO_PKG_VERSION, so the installer, its filename and
    the binary inside it cannot disagree - which matters more than it sounds,
    because the updater compares the running version against a release tag and
    a stale resource makes it offer an update the user already has.
#>

[CmdletBinding()]
param(
    [switch]$SkipHelper,
    [switch]$SkipInstaller,
    [switch]$Run
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $root
# Set once the target directory is known - see below. Staging inside the
# repository would put it under Dropbox, which holds the directory open and
# makes the clean fail with "used by another process". Same reason the build
# output is redirected; see AGENTS.md.
$dist = $null

# The one place the version comes from.
$manifest = Get-Content (Join-Path $repo 'Cargo.toml') -Raw
if ($manifest -notmatch '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
    throw 'Could not read the version out of Cargo.toml.'
}
$version = $Matches[1]
Write-Host "Barometer $version" -ForegroundColor Cyan

# The icon, if it is missing or older than the artwork. build.rs warns rather
# than fails when it is absent, so without this a release could ship with the
# generic default icon and nothing would have said so.
$art = Join-Path $repo 'assets\AppIcon.png'
$ico = Join-Path $repo 'assets\barometer.ico'
if (-not (Test-Path $ico) -or (Get-Item $art).LastWriteTime -gt (Get-Item $ico).LastWriteTime) {
    Write-Host 'Rebuilding the icon...' -ForegroundColor Yellow
    & (Join-Path $root 'make-icon.ps1')
}

# Nothing that leaves this machine may carry the path it was built on. Rust
# embeds source paths in panic messages and debug info, and this repository
# lives under a directory named after its author, twice. Remapped on the way in
# rather than only checked afterwards: a check tells you the build is unusable,
# a remap means it never was.
# CARGO_ENCODED_RUSTFLAGS rather than CARGO_BUILD_RUSTFLAGS: the latter is
# split on spaces, and the path being remapped is "...\David Brustein
# Dropbox\..." - so the flag arrives at rustc in three pieces and the build
# fails with "--remap-path-prefix must contain '=' between FROM and TO". The
# encoded form is separated by U+001F and survives any path.
$remaps = @(
    "--remap-path-prefix=$repo=."
    "--remap-path-prefix=$env:USERPROFILE\.cargo=.cargo"
    "--remap-path-prefix=$env:USERPROFILE=~"
)
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
$env:CARGO_ENCODED_RUSTFLAGS = ($remaps -join [char]0x1F)

Write-Host 'Building...' -ForegroundColor Yellow
Push-Location $repo
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed.' }

    if (-not $SkipHelper) {
        Write-Host 'Building the sensor helper...' -ForegroundColor Yellow
        # Self-contained, so the user needs no .NET runtime. Published rather
        # than built: the build output beside it is missing the two native
        # libraries a single-file publish still cannot bundle, and a helper
        # staged from there starts and exits immediately.
        dotnet publish (Join-Path $repo 'helper\BarometerSensorsHelper.csproj') `
            -c Release -r win-x64 --nologo
        if ($LASTEXITCODE -ne 0) { throw 'dotnet publish failed.' }
    }
} finally {
    Pop-Location
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
}

# Where cargo actually put it. The target directory is redirected on this
# machine - see .cargo/config.toml - so it is asked for rather than assumed.
$metadata = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
# cargo reports this with forward slashes. Windows tolerates them almost
# everywhere, but ISCC does not: a mixed-separator path reaches it as
# "The filename, directory name, or volume label syntax is incorrect" with
# nothing to say which path it meant.
$targetDir = [System.IO.Path]::GetFullPath($metadata.target_directory)
$exe = Join-Path $targetDir 'release\barometer.exe'
$dist = Join-Path (Join-Path $targetDir 'dist') 'Barometer'
if (-not (Test-Path $exe)) { throw "Not found after building: $exe" }

Write-Host 'Staging...' -ForegroundColor Yellow
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force $dist | Out-Null

Copy-Item $exe $dist
Copy-Item (Join-Path $repo 'LICENSE.md') $dist
Copy-Item (Join-Path $repo 'NOTICE.md')  $dist
Copy-Item $ico $dist

$publish = Join-Path $repo 'helper\bin\Release\net10.0-windows\win-x64\publish'
if (Test-Path (Join-Path $publish 'barometer-sensors.exe')) {
    # The exe and the two native libraries beside it, and nothing else in that
    # directory: the .pdb is debug information nobody downloading an installer
    # wants, and it carries full source paths.
    Copy-Item (Join-Path $publish 'barometer-sensors.exe') $dist
    foreach ($native in 'MonoPosixHelper.dll', 'libMonoPosixHelper.dll') {
        $path = Join-Path $publish $native
        if (Test-Path $path) { Copy-Item $path $dist }
    }
} else {
    Write-Warning 'No sensor helper staged; the installed Barometer will have no temperatures.'
}

# And verified, because a remap that silently stopped working would be
# invisible. The bytes are decoded as Latin-1 in one call rather than joined a
# character at a time: the pipeline form is correct and takes minutes on a one
# megabyte executable, which is long enough that it reads as a hang.
$pattern = 'Dropbox|David Brustein|[A-Za-z]:[\/]Users[\/]'
foreach ($binary in Get-ChildItem $dist -Filter *.exe) {
    $bytes = [System.IO.File]::ReadAllBytes($binary.FullName)
    $text = [System.Text.Encoding]::GetEncoding(28591).GetString($bytes)
    $hits = [regex]::Matches($text, $pattern)
    if ($hits.Count -gt 0) {
        $hits | Select-Object -First 5 | ForEach-Object {
            $from = [Math]::Max(0, $_.Index - 40)
            $span = [Math]::Min(120, $text.Length - $from)
            Write-Host ("  ..." + $text.Substring($from, $span)) -ForegroundColor Red
        }
        throw "$($binary.Name) carries the path it was built on. See the strings above."
    }
    Write-Host ("  {0} carries no build path" -f $binary.Name)
}

Write-Host ("Staged {0} files into {1}" -f (Get-ChildItem $dist).Count, $dist) -ForegroundColor Green

if (-not $SkipInstaller) {
    $iscc = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) {
        Write-Warning 'Inno Setup 6 not found; skipping the installer.'
    } else {
        Write-Host 'Packaging...' -ForegroundColor Yellow
        # The installer is written beside the staging directory, outside the
        # repository. Inside it the compile fails at the last step with
        # "EndUpdateResource failed ... try excluding the Output folder from
        # your antivirus software (110)" - which here is not antivirus but
        # the synced folder grabbing the file as ISCC is still writing it.
        # Same root cause as the build directory; see AGENTS.md.
        $output = Join-Path $targetDir 'dist'
        & $iscc "/DAppVersion=$version" "/DStageDir=$dist" "/O$output" `
            (Join-Path $repo 'installer\barometer.iss')
        if ($LASTEXITCODE -ne 0) { throw 'ISCC failed.' }
        $setup = Join-Path $output "Barometer-Setup-$version.exe"
        if (Test-Path $setup) {
            # The digest the updater will check a download against, so it can
            # be compared with what GitHub publishes for the uploaded asset.
            $hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
            Write-Host ("{0}`nSHA-256 {1}" -f $setup, $hash) -ForegroundColor Green
        }
    }
}

if ($Run) {
    Write-Host 'Starting...' -ForegroundColor Yellow
    Start-Process (Join-Path $dist 'barometer.exe') -ArgumentList '--strip'
}
