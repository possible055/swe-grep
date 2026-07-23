[CmdletBinding()]
param(
    [ValidateSet('agents', 'codex', 'cursor', 'claude', 'custom')]
    [string]$Client,

    [ValidateSet('user', 'project')]
    [string]$Scope = 'user',

    [string]$InstallRoot,
    [string]$ReleaseBaseUrl = 'https://github.com/possible055/swe-grep/releases/latest/download',
    [switch]$Uninstall,
    [switch]$Force,
    [switch]$Yes
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$archiveName = 'swe-grep-windows-x86_64.zip'
$scopeWasProvided = $PSBoundParameters.ContainsKey('Scope')

function Stop-Install {
    param([string]$Message)
    throw $Message
}

function Get-NormalizedPath {
    param([string]$Value)

    if ([string]::IsNullOrWhiteSpace($Value)) {
        return $null
    }

    $expanded = [Environment]::ExpandEnvironmentVariables($Value.Trim().Trim('"'))
    try {
        $fullPath = [IO.Path]::GetFullPath($expanded)
    } catch {
        return $null
    }

    $root = [IO.Path]::GetPathRoot($fullPath)
    if ([string]::Equals($fullPath, $root, [StringComparison]::OrdinalIgnoreCase)) {
        return $fullPath
    }
    return $fullPath.TrimEnd('\', '/')
}

function Test-SamePath {
    param(
        [string]$Left,
        [string]$Right
    )

    $normalizedLeft = Get-NormalizedPath $Left
    $normalizedRight = Get-NormalizedPath $Right
    if ($null -eq $normalizedLeft -or $null -eq $normalizedRight) {
        return $false
    }
    return [string]::Equals($normalizedLeft, $normalizedRight, [StringComparison]::OrdinalIgnoreCase)
}

function Test-PathListContains {
    param(
        [AllowNull()]
        [string]$Value,
        [string]$Candidate
    )

    if ([string]::IsNullOrWhiteSpace($Value)) {
        return $false
    }
    foreach ($entry in $Value.Split(';')) {
        if (Test-SamePath $entry $Candidate) {
            return $true
        }
    }
    return $false
}

function Assert-WindowsEnvironment {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        Stop-Install 'This installer supports Windows only.'
    }
    if ($PSVersionTable.PSVersion -lt [Version]'5.1') {
        Stop-Install 'PowerShell 5.1 or later is required.'
    }

    $nativeArchitecture = $env:PROCESSOR_ARCHITECTURE
    if (-not [string]::IsNullOrWhiteSpace($env:PROCESSOR_ARCHITEW6432)) {
        $nativeArchitecture = $env:PROCESSOR_ARCHITEW6432
    }
    if ([string]::IsNullOrWhiteSpace($nativeArchitecture)) {
        Stop-Install 'Could not determine the native Windows architecture.'
    }
    if (-not [string]::Equals($nativeArchitecture, 'AMD64', [StringComparison]::OrdinalIgnoreCase)) {
        Stop-Install "No release artifact is available for architecture: $nativeArchitecture"
    }
}

function Get-VerifiedSpecialFolder {
    param(
        [Environment+SpecialFolder]$Folder,
        [string]$EnvironmentName
    )

    $knownPath = [Environment]::GetFolderPath($Folder)
    if ([string]::IsNullOrWhiteSpace($knownPath) -or -not [IO.Path]::IsPathRooted($knownPath)) {
        Stop-Install "Could not resolve the Windows $Folder folder."
    }

    $environmentPath = [Environment]::GetEnvironmentVariable($EnvironmentName, 'Process')
    if (-not [string]::IsNullOrWhiteSpace($environmentPath) -and
        -not (Test-SamePath $environmentPath $knownPath)) {
        Write-Warning "$EnvironmentName does not match the Windows known folder. Using: $knownPath"
    }
    return $knownPath
}

function Get-ProjectRoot {
    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($null -ne $git) {
        $candidate = & git rev-parse --show-toplevel 2>$null
        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($candidate)) {
            return [IO.Path]::GetFullPath(($candidate | Select-Object -First 1))
        }
    }
    return [IO.Path]::GetFullPath((Get-Location).ProviderPath)
}

function Resolve-CustomRoot {
    param(
        [string]$Value,
        [string]$UserProfilePath
    )

    if ([string]::IsNullOrWhiteSpace($Value)) {
        Stop-Install 'A custom installation requires -InstallRoot.'
    }

    $resolved = $Value.Trim()
    if ($resolved -eq '~') {
        $resolved = $UserProfilePath
    } elseif ($resolved.StartsWith('~\') -or $resolved.StartsWith('~/')) {
        $resolved = Join-Path $UserProfilePath $resolved.Substring(2)
    } elseif ($resolved.StartsWith('~')) {
        Stop-Install 'Custom paths only support ~, ~\, or ~/ as home-directory shortcuts.'
    }
    return [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($resolved))
}

function Select-Client {
    Write-Host 'Choose an Agent Skill client:'
    Write-Host '  1) Agent Skills, cross-client  (~\.agents\skills)'
    Write-Host '  2) Cursor  (~\.cursor\skills)'
    Write-Host '  3) Claude Code  (~\.claude\skills)'
    Write-Host '  4) Custom path'
    $selection = Read-Host 'Selection [1]'
    switch ($selection) {
        '' { return 'agents' }
        '1' { return 'agents' }
        '2' { return 'cursor' }
        '3' { return 'claude' }
        '4' { return 'custom' }
        default { Stop-Install "Unknown client selection: $selection" }
    }
}

function Select-Scope {
    Write-Host 'Choose an installation scope:'
    Write-Host '  1) User'
    Write-Host '  2) Project'
    $selection = Read-Host 'Selection [1]'
    switch ($selection) {
        '' { return 'user' }
        '1' { return 'user' }
        '2' { return 'project' }
        default { Stop-Install "Unknown scope selection: $selection" }
    }
}

function Invoke-Download {
    param(
        [string]$Uri,
        [string]$Destination
    )

    $source = [Uri]$Uri
    if ($source.IsFile) {
        Copy-Item -LiteralPath $source.LocalPath -Destination $Destination
        return
    }
    Invoke-WebRequest -Uri $Uri -OutFile $Destination -UseBasicParsing
}

function Get-BackupPath {
    param([string]$OriginalPath)

    $candidate = "$OriginalPath.backup.$(Get-Date -Format 'yyyyMMddHHmmss')"
    while (Test-Path -LiteralPath $candidate) {
        $candidate = "$candidate.new"
    }
    return $candidate
}

function Move-ToBackup {
    param(
        [string]$OriginalPath,
        [string]$Label
    )

    $backupPath = Get-BackupPath $OriginalPath
    Move-Item -LiteralPath $OriginalPath -Destination $backupPath
    Write-Host "$Label preserved at $backupPath"
}

function Get-LauncherContent {
    param([string]$ExecutablePath)

    $escapedPath = $ExecutablePath.Replace('%', '%%')
    return "@echo off`r`nchcp 65001 >nul`r`n`"$escapedPath`" %*`r`n"
}

function Test-LauncherMatches {
    param(
        [string]$LauncherPath,
        [string]$ExpectedContent
    )

    if (-not (Test-Path -LiteralPath $LauncherPath -PathType Leaf)) {
        return $false
    }
    return [string]::Equals(
        [IO.File]::ReadAllText($LauncherPath),
        $ExpectedContent,
        [StringComparison]::Ordinal
    )
}

function Get-PathExtensions {
    if ([string]::IsNullOrWhiteSpace($env:PATHEXT)) {
        Stop-Install 'PATHEXT is not set; .CMD command discovery cannot be verified.'
    }
    $extensions = @($env:PATHEXT -split ';' | ForEach-Object { $_.Trim().ToUpperInvariant() })
    if ($extensions -notcontains '.CMD') {
        Stop-Install 'PATHEXT must contain .CMD before installing the swe-grep launcher.'
    }
    return $extensions
}

function Assert-NoExternalCommand {
    param(
        [string]$LauncherPath,
        [string[]]$PathExtensions
    )

    foreach ($command in @(Get-Command swe-grep -All -ErrorAction SilentlyContinue)) {
        $commandPath = $null
        if ($null -ne $command.PSObject.Properties['Path']) {
            $commandPath = $command.Path
        } elseif ($null -ne $command.PSObject.Properties['Source']) {
            $commandPath = $command.Source
        }
        if ([string]::IsNullOrWhiteSpace($commandPath)) {
            Stop-Install "Another swe-grep $($command.CommandType) command is already defined. Remove that command conflict before installing."
        }
        if (-not (Test-SamePath $commandPath $LauncherPath)) {
            Stop-Install "Another swe-grep command is already available at $commandPath. Remove that PATH conflict before installing."
        }
    }

    $pathValues = @(
        [Environment]::GetEnvironmentVariable('Path', 'Process'),
        [Environment]::GetEnvironmentVariable('Path', 'User'),
        [Environment]::GetEnvironmentVariable('Path', 'Machine')
    )
    foreach ($pathValue in $pathValues) {
        if ([string]::IsNullOrWhiteSpace($pathValue)) {
            continue
        }
        foreach ($directory in $pathValue.Split(';')) {
            if ([string]::IsNullOrWhiteSpace($directory)) {
                continue
            }
            $expandedDirectory = [Environment]::ExpandEnvironmentVariables($directory.Trim().Trim('"'))
            foreach ($extension in $PathExtensions) {
                $candidate = Join-Path $expandedDirectory "swe-grep$($extension.ToLowerInvariant())"
                if ((Test-Path -LiteralPath $candidate -PathType Leaf) -and
                    -not (Test-SamePath $candidate $LauncherPath)) {
                    Stop-Install "Another swe-grep command is available through PATH at $candidate. Remove that conflict before installing."
                }
            }
        }
    }
}

function Assert-LauncherPreflight {
    param(
        [string]$LauncherPath,
        [string]$ExpectedContent,
        [string]$LauncherDirectory,
        [bool]$AllowReplacement
    )

    foreach ($extension in @('.com', '.exe', '.bat', '.cmd')) {
        $candidate = Join-Path $LauncherDirectory "swe-grep$extension"
        if (-not (Test-Path -LiteralPath $candidate)) {
            continue
        }
        if ((Test-SamePath $candidate $LauncherPath) -and
            (Test-LauncherMatches $LauncherPath $ExpectedContent)) {
            continue
        }
        if (-not $AllowReplacement) {
            Stop-Install "Launcher already exists: $candidate (pass -Force to preserve it as a backup)"
        }
    }
}

function Test-RuntimeMatches {
    param(
        [string]$RuntimePath,
        [string]$CandidatePath
    )

    foreach ($configPath in @('.env', 'include.txt', 'exclude.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $RuntimePath $configPath) -PathType Leaf)) {
            return $false
        }
    }
    foreach ($relativePath in @('SKILL.md', 'bin\swe-grep.exe', 'bin\rg.exe')) {
        $installed = Join-Path $RuntimePath $relativePath
        $candidate = Join-Path $CandidatePath $relativePath
        if (-not (Test-Path -LiteralPath $installed -PathType Leaf)) {
            return $false
        }
        if ((Get-FileHash -LiteralPath $installed -Algorithm SHA256).Hash -ne
            (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash) {
            return $false
        }
    }
    return $true
}

function Copy-PreservedConfig {
    param(
        [string]$SourceRoot,
        [string]$CandidateRoot
    )

    foreach ($fileName in @('.env', 'include.txt', 'exclude.txt')) {
        $source = Join-Path $SourceRoot $fileName
        if (Test-Path -LiteralPath $source -PathType Leaf) {
            Copy-Item -LiteralPath $source -Destination (Join-Path $CandidateRoot $fileName) -Force
        }
    }
}

function Get-InstallationRecordPath {
    param(
        [string]$RegistryDirectory,
        [string]$DestinationPath
    )

    $normalized = (Get-NormalizedPath $DestinationPath).ToUpperInvariant()
    $bytes = [Text.Encoding]::UTF8.GetBytes($normalized)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha256.ComputeHash($bytes)
    } finally {
        $sha256.Dispose()
    }
    $identifier = -join ($digest | ForEach-Object { $_.ToString('x2') })
    return Join-Path $RegistryDirectory $identifier
}

function Assert-RegistryPath {
    param([string]$DestinationPath)

    if ($DestinationPath.Contains("`r") -or $DestinationPath.Contains("`n")) {
        Stop-Install 'Skill paths cannot contain carriage returns or newlines.'
    }
}

function Write-InstallationRecord {
    param(
        [string]$RegistryDirectory,
        [string]$DestinationPath
    )

    Assert-RegistryPath $DestinationPath
    $entry = Get-InstallationRecordPath $RegistryDirectory $DestinationPath
    $skillPath = Join-Path $DestinationPath 'SKILL.md'
    $skillHash = (Get-FileHash -LiteralPath $skillPath -Algorithm SHA256).Hash.ToLowerInvariant()
    New-Item -ItemType Directory -Force -Path $RegistryDirectory | Out-Null
    $utf8WithoutBom = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllLines(
        $entry,
        @((Get-NormalizedPath $DestinationPath), $skillHash),
        $utf8WithoutBom
    )
}

function Remove-ExactPathEntry {
    param(
        [AllowNull()]
        [string]$Value,
        [string]$Target
    )

    if ($null -eq $Value) {
        return $null
    }
    $kept = New-Object 'System.Collections.Generic.List[string]'
    foreach ($entry in $Value.Split(';')) {
        if (-not (Test-SamePath $entry $Target)) {
            $kept.Add($entry)
        }
    }
    return [string]::Join(';', $kept.ToArray())
}

function Invoke-Uninstall {
    param(
        [string]$DestinationPath,
        [string]$RuntimePath,
        [string]$RegistryDirectory,
        [string]$LauncherPath,
        [string]$LauncherDirectory,
        [string]$ExpectedLauncherContent,
        [string]$ApplicationRoot,
        [bool]$SkipConfirmation
    )

    Assert-RegistryPath $DestinationPath
    $entry = Get-InstallationRecordPath $RegistryDirectory $DestinationPath
    if (-not (Test-Path -LiteralPath $entry -PathType Leaf)) {
        Stop-Install "The selected Skill is not recorded as a swe-grep managed installation: $DestinationPath"
    }

    $record = @([IO.File]::ReadAllLines($entry))
    if ($record.Count -lt 2 -or -not (Test-SamePath $record[0] $DestinationPath)) {
        Stop-Install "The installation record does not match: $DestinationPath"
    }
    $hasOtherInstallations = $null -ne (
        Get-ChildItem -LiteralPath $RegistryDirectory -File |
            Where-Object { -not (Test-SamePath $_.FullName $entry) } |
            Select-Object -First 1
    )
    if (-not $SkipConfirmation) {
        $confirmation = Read-Host "Remove the managed Agent Skill at $DestinationPath? [y/N]"
        if ($confirmation -notin @('y', 'Y', 'yes', 'YES')) {
            Stop-Install 'Uninstallation cancelled.'
        }
    }

    $skillPath = Join-Path $DestinationPath 'SKILL.md'
    if (Test-Path -LiteralPath $skillPath) {
        if (-not (Test-Path -LiteralPath $skillPath -PathType Leaf)) {
            Stop-Install "Managed SKILL.md is not a regular file: $skillPath"
        }
        $currentHash = (Get-FileHash -LiteralPath $skillPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($currentHash -ne $record[1].ToLowerInvariant()) {
            Stop-Install "Managed SKILL.md was modified; leaving it and the installation record unchanged: $skillPath"
        }
        Remove-Item -LiteralPath $skillPath
    }
    if (Test-Path -LiteralPath $DestinationPath -PathType Container) {
        if ($null -eq (Get-ChildItem -LiteralPath $DestinationPath | Select-Object -First 1)) {
            Remove-Item -LiteralPath $DestinationPath
        } else {
            Write-Host "Kept non-installer files at $DestinationPath"
        }
    }
    if ($hasOtherInstallations) {
        Remove-Item -LiteralPath $entry
        Write-Host "Removed Agent Skill at $DestinationPath"
        Write-Host 'Shared runtime and launcher were kept because other managed Skills remain.'
        return
    }

    if (Test-LauncherMatches $LauncherPath $ExpectedLauncherContent) {
        Remove-Item -LiteralPath $LauncherPath
    } elseif (Test-Path -LiteralPath $LauncherPath) {
        Write-Warning "Launcher was modified and was left unchanged: $LauncherPath"
    }
    if (Test-Path -LiteralPath $RuntimePath) {
        Remove-Item -LiteralPath $RuntimePath -Recurse -Force
    }

    $pathMarker = Join-Path $ApplicationRoot 'user-path-added'
    if (Test-Path -LiteralPath $pathMarker -PathType Leaf) {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        [Environment]::SetEnvironmentVariable(
            'Path',
            (Remove-ExactPathEntry $userPath $LauncherDirectory),
            'User'
        )
        Remove-Item -LiteralPath $pathMarker
    }

    Remove-Item -LiteralPath $entry
    if ((Test-Path -LiteralPath $RegistryDirectory -PathType Container) -and
        $null -eq (Get-ChildItem -LiteralPath $RegistryDirectory | Select-Object -First 1)) {
        Remove-Item -LiteralPath $RegistryDirectory
    }

    if ((Test-Path -LiteralPath $LauncherDirectory -PathType Container) -and
        $null -eq (Get-ChildItem -LiteralPath $LauncherDirectory | Select-Object -First 1)) {
        Remove-Item -LiteralPath $LauncherDirectory
    }
    if ((Test-Path -LiteralPath $ApplicationRoot -PathType Container) -and
        $null -eq (Get-ChildItem -LiteralPath $ApplicationRoot | Select-Object -First 1)) {
        Remove-Item -LiteralPath $ApplicationRoot
    }

    Write-Host "Removed Agent Skill at $DestinationPath"
    Write-Host 'Removed the shared swe-grep runtime and managed launcher.'
    Write-Host 'Restart terminals and agent applications to unload the PATH change from existing processes.'
}

Assert-WindowsEnvironment

$userProfilePath = Get-VerifiedSpecialFolder ([Environment+SpecialFolder]::UserProfile) 'USERPROFILE'
$localAppDataPath = Get-VerifiedSpecialFolder ([Environment+SpecialFolder]::LocalApplicationData) 'LOCALAPPDATA'

$clientWasInteractive = [string]::IsNullOrWhiteSpace($Client)
if ($clientWasInteractive) {
    if ($Yes) {
        $Client = 'agents'
    } else {
        $Client = Select-Client
    }
}
if ([string]::Equals($Client, 'codex', [StringComparison]::OrdinalIgnoreCase)) {
    $Client = 'agents'
}
if ($clientWasInteractive -and -not $scopeWasProvided -and -not $Yes) {
    $Scope = Select-Scope
}

if ([string]::Equals($Client, 'custom', [StringComparison]::OrdinalIgnoreCase)) {
    if ([string]::IsNullOrWhiteSpace($InstallRoot) -and -not $Yes) {
        $InstallRoot = Read-Host 'Custom skill parent directory'
    }
    $skillParent = Resolve-CustomRoot $InstallRoot $userProfilePath
} else {
    if (-not [string]::IsNullOrWhiteSpace($InstallRoot)) {
        Stop-Install '-InstallRoot can only be used with -Client custom.'
    }
    if ([string]::Equals($Scope, 'project', [StringComparison]::OrdinalIgnoreCase)) {
        $basePath = Get-ProjectRoot
    } else {
        $basePath = $userProfilePath
    }
    switch ($Client.ToLowerInvariant()) {
        'agents' { $skillParent = Join-Path $basePath '.agents\skills' }
        'cursor' { $skillParent = Join-Path $basePath '.cursor\skills' }
        'claude' { $skillParent = Join-Path $basePath '.claude\skills' }
        default { Stop-Install "Unknown client: $Client" }
    }
}

$destination = Join-Path $skillParent 'swe-grep'
$applicationRoot = Join-Path $localAppDataPath 'swe-grep'
$runtime = Join-Path $applicationRoot 'runtime'
$registryDirectory = Join-Path $applicationRoot 'installations'
$launcherDirectory = Join-Path $applicationRoot 'bin'
$launcherPath = Join-Path $launcherDirectory 'swe-grep.cmd'
$executablePath = Join-Path $runtime 'bin\swe-grep.exe'
$launcherContent = Get-LauncherContent $executablePath

if ($Uninstall) {
    Invoke-Uninstall `
        $destination `
        $runtime `
        $registryDirectory `
        $launcherPath `
        $launcherDirectory `
        $launcherContent `
        $applicationRoot `
        ([bool]$Yes)
    exit 0
}

$pathExtensions = Get-PathExtensions

if ((Test-Path -LiteralPath $destination) -and -not $Force) {
    Stop-Install "Destination already exists: $destination (pass -Force to preserve it as a backup)"
}
Assert-NoExternalCommand $launcherPath $pathExtensions
Assert-LauncherPreflight $launcherPath $launcherContent $launcherDirectory ([bool]$Force)

if (-not $Yes) {
    $confirmation = Read-Host "Install $archiveName runtime at $runtime and Skill at $destination? [y/N]"
    if ($confirmation -notin @('y', 'Y', 'yes', 'YES')) {
        Stop-Install 'Installation cancelled.'
    }
}

$releaseRoot = $ReleaseBaseUrl.TrimEnd('/')
$temporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) "swe-grep-install-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null

try {
    if (([Enum]::GetNames([Net.SecurityProtocolType])) -contains 'Tls12') {
        [Net.ServicePointManager]::SecurityProtocol =
            [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    }

    $archivePath = Join-Path $temporaryDirectory $archiveName
    $checksumPath = "$archivePath.sha256"
    Invoke-Download "$releaseRoot/$archiveName" $archivePath
    Invoke-Download "$releaseRoot/$archiveName.sha256" $checksumPath

    $digestMatch = [regex]::Match([IO.File]::ReadAllText($checksumPath), '(?i)\b[a-f0-9]{64}\b')
    if (-not $digestMatch.Success) {
        Stop-Install "Could not find a SHA-256 digest in $archiveName.sha256"
    }
    $expectedDigest = $digestMatch.Value.ToLowerInvariant()
    $actualDigest = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualDigest -ne $expectedDigest) {
        Stop-Install "Checksum mismatch for $archiveName"
    }
    Write-Host "$archiveName`: SHA-256 OK"

    $packageRoot = Join-Path $temporaryDirectory 'package'
    Expand-Archive -LiteralPath $archivePath -DestinationPath $packageRoot
    $candidate = Join-Path $packageRoot 'swe-grep'
    foreach ($requiredPath in @(
        (Join-Path $candidate 'SKILL.md'),
        (Join-Path $candidate '.env'),
        (Join-Path $candidate 'include.txt'),
        (Join-Path $candidate 'exclude.txt'),
        (Join-Path $candidate 'bin\swe-grep.exe'),
        (Join-Path $candidate 'bin\rg.exe')
    )) {
        if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
            Stop-Install "Release archive is missing: $requiredPath"
        }
    }

    $runtimeMatches = Test-RuntimeMatches $runtime $candidate
    if (-not $runtimeMatches) {
        if (Test-Path -LiteralPath $runtime) {
            if (-not $Force) {
                Stop-Install "A different swe-grep runtime is already installed at $runtime (pass -Force to preserve and upgrade it)"
            }
            Copy-PreservedConfig $runtime $candidate
            Move-ToBackup $runtime 'Existing runtime'
        } elseif ($Force -and (Test-Path -LiteralPath $destination)) {
            Copy-PreservedConfig $destination $candidate
        }
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $runtime) | Out-Null
        Move-Item -LiteralPath $candidate -Destination $runtime
    }

    New-Item -ItemType Directory -Force -Path $skillParent | Out-Null
    if (Test-Path -LiteralPath $destination) {
        Move-ToBackup $destination 'Existing skill'
    }
    New-Item -ItemType Directory -Path $destination | Out-Null
    Copy-Item -LiteralPath (Join-Path $runtime 'SKILL.md') -Destination (Join-Path $destination 'SKILL.md')

    New-Item -ItemType Directory -Force -Path $launcherDirectory | Out-Null
    foreach ($extension in @('.com', '.exe', '.bat', '.cmd')) {
        $candidateLauncher = Join-Path $launcherDirectory "swe-grep$extension"
        if (-not (Test-Path -LiteralPath $candidateLauncher)) {
            continue
        }
        if ((Test-SamePath $candidateLauncher $launcherPath) -and
            (Test-LauncherMatches $launcherPath $launcherContent)) {
            continue
        }
        Move-ToBackup $candidateLauncher 'Existing launcher'
    }
    $utf8WithoutBom = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText($launcherPath, $launcherContent, $utf8WithoutBom)

    $processPath = [Environment]::GetEnvironmentVariable('Path', 'Process')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $persistentPathChanged = $false
    $processPathChanged = $false

    $persisted =
        (Test-PathListContains $userPath $launcherDirectory) -or
        (Test-PathListContains $machinePath $launcherDirectory)
    if (-not $persisted) {
        if ([string]::IsNullOrEmpty($userPath)) {
            $newUserPath = $launcherDirectory
        } else {
            $newUserPath = "$userPath;$launcherDirectory"
        }
        try {
            [Environment]::SetEnvironmentVariable('Path', $newUserPath, 'User')
            [IO.File]::WriteAllText((Join-Path $applicationRoot 'user-path-added'), '')
        } catch {
            [Environment]::SetEnvironmentVariable('Path', $userPath, 'User')
            throw
        }
        $persistentPathChanged = $true
    }

    if (-not (Test-PathListContains $processPath $launcherDirectory)) {
        if ([string]::IsNullOrEmpty($processPath)) {
            $env:Path = $launcherDirectory
        } else {
            $env:Path = "$processPath;$launcherDirectory"
        }
        $processPathChanged = $true
    }

    Write-InstallationRecord $registryDirectory $destination
    Write-Host "Installed swe-grep runtime at $runtime"
    Write-Host "Installed Agent Skill at $destination"
    Write-Host "Launcher available at $launcherPath"
    if ($persistentPathChanged) {
        Write-Host 'Restart terminals and agent applications to load the User PATH change.'
    } else {
        Write-Host "$launcherDirectory is already persisted through the User or Machine PATH."
    }
    if ($processPathChanged) {
        Write-Host 'The launcher is also available in the current PowerShell process.'
    }
} finally {
    if (Test-Path -LiteralPath $temporaryDirectory) {
        Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
    }
}
