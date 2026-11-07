#!/usr/bin/env powershell

param (
  [Parameter(Position = 0, Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$BuildRoot,
  [Parameter(Position = 1, Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$SourceRoot,
  [Parameter(Position = 2)]
  [ValidateSet('x64', 'arm64')]
  [string]$Architecture = 'x64',
  [ValidateNotNullOrEmpty()]
  [string]$DepCtrlVersion = "0.8.1",
  # Skip compiling translations, to package without a Meson build tree
  [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'

$InstallerDir = Join-Path $SourceRoot "packages\win_installer" | Resolve-Path
$DepsDir = Join-Path $BuildRoot "installer-deps"
if (!(Test-Path $DepsDir)) {
	New-Item -ItemType Directory -Path $DepsDir | Out-Null
}

$Env:BUILD_ROOT = $BuildRoot
$Env:SOURCE_ROOT = $SourceRoot

$GitHeaders = @{}
if (Test-Path 'Env:GITHUB_TOKEN') {
	$GitHeaders = @{ 'Authorization' = 'Bearer ' + $Env:GITHUB_TOKEN }
}

# DependencyControl
$DepCtrlDir = Join-Path $DepsDir "DependencyControl"

function Get-CachedDepCtrlVersion {
	param([Parameter(Mandatory = $true)][string]$Root)
	$module = Join-Path $Root "automation\include\l0\DependencyControl.moon"
	if (!(Test-Path -LiteralPath $module -PathType Leaf)) {
		return $null
	}
	$marker = Select-String -LiteralPath $module -Pattern 'version:\s*"([^"]+)".*--\s*@\{l0\.DependencyControl:version\}' |
		Select-Object -First 1
	if (!$marker) {
		return $null
	}
	$marker.Matches[0].Groups[1].Value
}

if ((Get-CachedDepCtrlVersion $DepCtrlDir) -ne $DepCtrlVersion) {
	$depCtrlUrl = "https://github.com/TypesettingTools/DependencyControl/releases/download/v$DepCtrlVersion/DependencyControl-v$DepCtrlVersion.zip"
	$depCtrlStagingDir = Join-Path $DepsDir ("DependencyControl-{0}" -f [guid]::NewGuid().ToString("N"))
	$depCtrlZip = Join-Path $depCtrlStagingDir "DependencyControl.zip"

	try {
		New-Item -ItemType Directory -Path $depCtrlStagingDir | Out-Null
		Invoke-WebRequest $depCtrlUrl -OutFile $depCtrlZip -UseBasicParsing
		7z x $depCtrlZip "-o$depCtrlStagingDir"
		if ($LASTEXITCODE -ne 0) {
			throw "Failed to extract DependencyControl (7z exited with code $LASTEXITCODE)"
		}

		$stagedVersion = Get-CachedDepCtrlVersion $depCtrlStagingDir
		if (!$stagedVersion) {
			throw "DependencyControl archive did not contain a versioned automation\include\l0\DependencyControl.moon"
		}
		if ($stagedVersion -ne $DepCtrlVersion) {
			throw "DependencyControl archive contains version $stagedVersion, expected $DepCtrlVersion"
		}

		Remove-Item -LiteralPath $depCtrlZip
		if (Test-Path -LiteralPath $DepCtrlDir) {
			Remove-Item -LiteralPath $DepCtrlDir -Recurse -Force
		}
		Rename-Item -LiteralPath $depCtrlStagingDir -NewName (Split-Path $DepCtrlDir -Leaf)
		Write-Host "DependencyControl v$DepCtrlVersion has been downloaded to $DepCtrlDir"
	} finally {
		if (Test-Path -LiteralPath $depCtrlStagingDir) {
			Remove-Item -LiteralPath $depCtrlStagingDir -Recurse -Force -ErrorAction SilentlyContinue
		}
	}
} else {
	Write-Host "DependencyControl v$DepCtrlVersion already cached at $DepCtrlDir"
}

# Avisynth
# $AviSynthDir = Join-Path $DepsDir "AviSynthPlus64"
# if (!(Test-Path $AviSynthDir)) {
# 	$avsReleases = Invoke-WebRequest "https://api.github.com/repos/AviSynth/AviSynthPlus/releases/latest" -Headers $GitHeaders -UseBasicParsing | ConvertFrom-Json
# 	$avsUrl = $avsReleases.assets[0].browser_download_url
# 	$avsArchive = Join-Path $DepsDir "AviSynthPlus.7z"
# 	Invoke-WebRequest $avsUrl -OutFile $avsArchive -UseBasicParsing
# 	7z x $avsArchive "-o$DepsDir"
# 	Rename-Item (Join-Path $DepsDir (Get-ChildItem -Path $DepsDir -Filter "AviSynthPlus_*" -Directory).Name) $AviSynthDir
# 	Remove-Item $avsArchive
# }

# libmpv: LGPL build of mpv (zhongfly/mpv-winbuild, derived from shinchiro/mpv-winbuild-cmake).
# Aegisub loads libmpv-2.dll at run time, so it ships beside the executables. Pins are
# GitHub release asset digests (live API, 2026-11-07); the archive SHA256 is re-verified on download.
$LibmpvPins = @{
    x64   = @{ Url = 'https://github.com/zhongfly/mpv-winbuild/releases/download/2026-11-07-55d3feb45c/mpv-dev-lgpl-x86_64-20261107-git-55d3feb45c.7z'; Name = 'mpv-dev-lgpl-x86_64-20261107-git-55d3feb45c.7z'; Sha256 = '17bff1d2e90ad06a9d98ce0fae738feb64feaf10ddd804870e0144111a9715df'; DllSha256 = '6d9a3a97b7efa71476d2bc4717944a6d8ae6f6b769149796af08fb055d7b665a'; Machine = 0x8664 }
    arm64 = @{ Url = 'https://github.com/zhongfly/mpv-winbuild/releases/download/2026-11-07-55d3feb45c/mpv-dev-lgpl-aarch64-20261107-git-55d3feb45c.7z'; Name = 'mpv-dev-lgpl-aarch64-20261107-git-55d3feb45c.7z'; Sha256 = '6210785d28c4d48e44dc7de083fe69c49411e0bf30a8a65b5bd4b659aadae891'; DllSha256 = 'f7d49de86a0a019ccd8c6a5baf16734ec5158760e9dc618ea10533441266858b'; Machine = 0xAA64 }
}

function Get-PeMachine {
    param([Parameter(Mandatory = $true)][string]$Path)
    $data = [IO.File]::ReadAllBytes($Path)
    $peOffset = [BitConverter]::ToInt32($data, 0x3C)
    if (-not ($data[$peOffset] -eq 0x50 -and $data[$peOffset + 1] -eq 0x45 -and $data[$peOffset + 2] -eq 0 -and $data[$peOffset + 3] -eq 0)) {
        throw "Invalid PE header: $Path"
    }
    [BitConverter]::ToUInt16($data, $peOffset + 4)
}

$LibmpvDir = Join-Path $DepsDir "libmpv\$Architecture"
$LibmpvDll = Join-Path $LibmpvDir "libmpv-2.dll"
$LibmpvPin = $LibmpvPins[$Architecture]
$LicenseSha = '72b672113d642cbb8ef5dcc76938db801983c56e50b1400ab930f1a64d6dc8d9'
$LicenseFile = Join-Path $LibmpvDir "LICENSE.LGPL"
$libmpvReady = (Test-Path -LiteralPath $LibmpvDll -PathType Leaf) -and
    (Test-Path -LiteralPath $LicenseFile -PathType Leaf) -and
    (Test-Path -LiteralPath (Join-Path $LibmpvDir "PROVENANCE.txt") -PathType Leaf)
if ($libmpvReady) {
    $libmpvReady = (Get-FileHash -LiteralPath $LibmpvDll -Algorithm SHA256).Hash.ToLowerInvariant() -eq $LibmpvPin.DllSha256 -and
        (Get-FileHash -LiteralPath $LicenseFile -Algorithm SHA256).Hash.ToLowerInvariant() -eq $LicenseSha
}
if (!$libmpvReady) {
    New-Item -ItemType Directory -Path $LibmpvDir -Force | Out-Null
    $LibmpvArchive = Join-Path $LibmpvDir $LibmpvPin.Name
    $LibmpvStage = Join-Path $LibmpvDir "stage"
    try {
        Invoke-WebRequest $LibmpvPin.Url -OutFile $LibmpvArchive -UseBasicParsing
        $archiveSha = (Get-FileHash -LiteralPath $LibmpvArchive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($archiveSha -ne $LibmpvPin.Sha256) {
            throw "libmpv archive SHA256 is $archiveSha, expected $($LibmpvPin.Sha256)"
        }
        7z x $LibmpvArchive "-o$LibmpvStage" -y | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to extract libmpv (7z exited with code $LASTEXITCODE)"
        }
        $libmpvFound = Get-ChildItem -LiteralPath $LibmpvStage -Recurse -Filter "libmpv-2.dll" -File | Select-Object -First 1
        if (!$libmpvFound) {
            throw "libmpv archive does not contain libmpv-2.dll"
        }
        $libmpvMachine = Get-PeMachine $libmpvFound.FullName
        if ($libmpvMachine -ne $LibmpvPin.Machine) {
            throw "libmpv-2.dll machine type 0x$('{0:X4}' -f $libmpvMachine) does not match $Architecture (0x$('{0:X4}' -f $LibmpvPin.Machine))"
        }
        $dllSha = (Get-FileHash -LiteralPath $libmpvFound.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($dllSha -ne $LibmpvPin.DllSha256) {
            throw "libmpv-2.dll SHA256 is $dllSha, expected $($LibmpvPin.DllSha256)"
        }
        Set-Content -LiteralPath (Join-Path $LibmpvDir "PROVENANCE.txt") -Value @(
            "libmpv ($Architecture)"
            "Source: $($LibmpvPin.Url)"
            "Archive SHA256: $archiveSha"
            "libmpv-2.dll SHA256: $dllSha"
            "Upstream: https://github.com/zhongfly/mpv-winbuild (LGPL build, tag 2026-11-07-55d3feb45c), based on https://github.com/shinchiro/mpv-winbuild-cmake"
            "License: LGPL-2.1 text in LICENSE.LGPL (https://raw.githubusercontent.com/mpv-player/mpv/master/LICENSE.LGPL, SHA256 72b672113d642cbb8ef5dcc76938db801983c56e50b1400ab930f1a64d6dc8d9)"
        )
        # The archive ships no license text, so take mpv's LGPL-2.1 text, pinned by SHA256.
        Invoke-WebRequest 'https://raw.githubusercontent.com/mpv-player/mpv/master/LICENSE.LGPL' -OutFile $LicenseFile -UseBasicParsing
        $licenseActual = (Get-FileHash -LiteralPath $LicenseFile -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($licenseActual -ne $LicenseSha) {
            throw "LICENSE.LGPL SHA256 is $licenseActual, expected $LicenseSha"
        }
        Copy-Item -LiteralPath $libmpvFound.FullName -Destination $LibmpvDll
    } finally {
        Remove-Item -LiteralPath $LibmpvStage -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $LibmpvArchive -Force -ErrorAction SilentlyContinue
    }
    Write-Host "libmpv ($Architecture) staged at $LibmpvDll"
} else {
    Write-Host "libmpv ($Architecture) already cached at $LibmpvDll"
}

# VSFilter
$VSFilterDir = Join-Path $DepsDir "VSFilter"
if ($Architecture -eq 'x64' -and !(Test-Path $VSFilterDir)) {
	New-Item -ItemType Directory -Path $VSFilterDir | Out-Null
	$vsFilterReleases = Invoke-WebRequest "https://api.github.com/repos/pinterf/xy-VSFilter/releases/latest" -Headers $GitHeaders -UseBasicParsing | ConvertFrom-Json
	$vsFilterUrl = $vsFilterReleases.assets[0].browser_download_url
	$vsFilterArchive = Join-Path $VSFilterDir "VSFilter.7z"
	Invoke-WebRequest $vsFilterUrl -OutFile $vsFilterArchive -UseBasicParsing
	7z x $vsFilterArchive "-o$VSFilterDir"
	Remove-Item $vsFilterArchive
}

# VC++ redistributable
$RedistDir = Join-Path $DepsDir "VC_redist"
$RedistName = "VC_redist.$Architecture.exe"
$RedistPath = Join-Path $RedistDir $RedistName
if (!(Test-Path $RedistPath)) {
	New-Item -ItemType Directory -Path $RedistDir -Force | Out-Null
	Invoke-WebRequest "https://aka.ms/vs/17/release/$RedistName" -OutFile $RedistPath -UseBasicParsing
}

# Dictionaries
$DictionariesDir = Join-Path $DepsDir "dictionaries"
if (!(Test-Path $DictionariesDir)) {
	New-Item -ItemType Directory -Path $DictionariesDir | Out-Null
	Invoke-WebRequest https://raw.githubusercontent.com/TypesettingTools/Aegisub-dictionaries/master/dicts/en_US.aff -OutFile (Join-Path $DictionariesDir "en_US.aff") -UseBasicParsing
	Invoke-WebRequest https://raw.githubusercontent.com/TypesettingTools/Aegisub-dictionaries/master/dicts/en_US.dic -OutFile (Join-Path $DictionariesDir "en_US.dic") -UseBasicParsing
}

# Installer localization
$LangsDir = Join-Path $DepsDir "innosetup-langs"
if (!(Test-Path $LangsDir)) {
	New-Item -ItemType Directory -Path $LangsDir | Out-Null
	$LangBaseUrl = "https://raw.github.com/jrsoftware/issrc/is-6_7_3/Files/Languages/Unofficial"
	$Languages = @(
		'Greek', 'Basque', 'Galician', 'Indonesian',
		'SerbianCyrillic', 'SerbianLatin', 'ChineseSimplified', 'ChineseTraditional'
	)
	foreach ($lang in $Languages) {
		Invoke-WebRequest "$LangBaseUrl/$lang.isl" -OutFile (Join-Path $LangsDir "$lang.isl") -UseBasicParsing
	}
}

# Aegisub localization
if (!$NoBuild) {
	meson compile -C $BuildRoot aegisub-gmo
	if(!$?) { Exit $LASTEXITCODE }
}

# Invoke InnoSetup
$IssUrl = Join-Path $InstallerDir "aegisub_depctrl.iss"
if ($Architecture -eq 'arm64') {
	iscc /DARM64 $IssUrl
} else {
	iscc $IssUrl
}
if(!$?) { Exit $LASTEXITCODE }
