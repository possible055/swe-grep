[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ReleaseDirectory
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

function Test-PathContains {
    param(
        [AllowNull()]
        [string]$Value,
        [string]$Candidate
    )

    if ([string]::IsNullOrWhiteSpace($Value)) {
        return $false
    }
    $expected = [IO.Path]::GetFullPath($Candidate).TrimEnd('\')
    foreach ($entry in $Value.Split(';')) {
        if ([string]::IsNullOrWhiteSpace($entry)) {
            continue
        }
        $actual = [IO.Path]::GetFullPath($entry).TrimEnd('\')
        if ([string]::Equals($actual, $expected, [StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }
    return $false
}

$releaseDirectoryPath = (Resolve-Path $ReleaseDirectory).Path
$releaseBaseUrl = ([Uri]$releaseDirectoryPath).AbsoluteUri
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$installer = Join-Path $scriptDirectory 'install.ps1'
$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) "swe-grep-installer-test-$([Guid]::NewGuid().ToString('N'))"
$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$applicationRoot = Join-Path $localAppData 'swe-grep'
$savedApplicationRoot = Join-Path $temporaryRoot 'saved-application-root'
$originalUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$originalProcessPath = [Environment]::GetEnvironmentVariable('Path', 'Process')
$machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')

New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
if (Test-Path -LiteralPath $applicationRoot) {
    Move-Item -LiteralPath $applicationRoot -Destination $savedApplicationRoot
}

try {
    $conflictBin = Join-Path $temporaryRoot 'conflict-bin'
    $conflictSkills = Join-Path $temporaryRoot 'conflict-skills'
    New-Item -ItemType Directory -Path $conflictBin | Out-Null
    [IO.File]::WriteAllText(
        (Join-Path $conflictBin 'swe-grep.cmd'),
        "@echo off`r`nexit /b 0`r`n",
        [Text.Encoding]::ASCII
    )
    $env:Path = "$conflictBin;$originalProcessPath"
    $conflictDetected = $false
    try {
        & $installer -Client custom -InstallRoot $conflictSkills -ReleaseBaseUrl $releaseBaseUrl -Yes
    } catch {
        $conflictDetected = $_.Exception.Message.Contains('Another swe-grep')
    }
    if (-not $conflictDetected) {
        throw 'Expected an external swe-grep command conflict.'
    }
    if (Test-Path -LiteralPath $conflictSkills) {
        throw 'External command conflict changed the Skill destination.'
    }

    $env:Path = $originalProcessPath
    $sentinelOne = Join-Path $temporaryRoot 'sentinel-one'
    $sentinelTwo = Join-Path $temporaryRoot 'sentinel-two'
    New-Item -ItemType Directory -Path $sentinelOne, $sentinelTwo | Out-Null
    $controlledUserPath = "$sentinelOne;;$sentinelTwo;"
    [Environment]::SetEnvironmentVariable('Path', $controlledUserPath, 'User')
    $firstSkills = Join-Path $temporaryRoot 'skills-one'
    & $installer -Client custom -InstallRoot $firstSkills -ReleaseBaseUrl $releaseBaseUrl -Yes

    $runtime = Join-Path $applicationRoot 'runtime'
    $launcherDirectory = Join-Path $applicationRoot 'bin'
    $launcher = Join-Path $launcherDirectory 'swe-grep.cmd'
    $skill = Join-Path $firstSkills 'swe-grep'
    if (-not (Test-Path -LiteralPath (Join-Path $runtime 'bin\swe-grep.exe') -PathType Leaf)) {
        throw 'The shared runtime executable was not installed.'
    }
    if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
        throw 'The managed launcher was not installed.'
    }
    if (@(Get-ChildItem -LiteralPath $skill -File).Count -ne 1 -or
        -not (Test-Path -LiteralPath (Join-Path $skill 'SKILL.md') -PathType Leaf)) {
        throw 'The Skill destination must contain only SKILL.md.'
    }
    & $launcher --help
    if ($LASTEXITCODE -ne 0) {
        throw "Managed launcher --help failed with exit code $LASTEXITCODE"
    }
    $installedUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($installedUserPath -ne "$controlledUserPath;$launcherDirectory") {
        throw 'Installation changed unrelated User PATH text or separators.'
    }

    [Environment]::SetEnvironmentVariable('Path', $controlledUserPath, 'User')
    if (Test-PathContains $machinePath $launcherDirectory) {
        throw 'The test runner Machine PATH unexpectedly contains the managed launcher directory.'
    }
    $env:Path = "$launcherDirectory;$originalProcessPath"
    $secondSkills = Join-Path $temporaryRoot 'skills-two'
    & $installer -Client custom -InstallRoot $secondSkills -ReleaseBaseUrl $releaseBaseUrl -Yes
    $persistedUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not (Test-PathContains $persistedUserPath $launcherDirectory)) {
        throw 'A Process-only launcher path was not persisted to the User PATH.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $secondSkills 'swe-grep\SKILL.md') -PathType Leaf)) {
        throw 'The second Skill installation failed.'
    }

    $secondSkillFile = Join-Path $secondSkills 'swe-grep\SKILL.md'
    Add-Content -LiteralPath $secondSkillFile -Value '# user modification'
    $modifiedSkillBlocked = $false
    try {
        & $installer -Uninstall -Client custom -InstallRoot $secondSkills -Yes
    } catch {
        $modifiedSkillBlocked = $_.Exception.Message.Contains('was modified')
    }
    if (-not $modifiedSkillBlocked) {
        throw 'Expected a modified SKILL.md to block uninstallation.'
    }
    Copy-Item -LiteralPath (Join-Path $runtime 'SKILL.md') -Destination $secondSkillFile -Force

    Set-Alias -Name swe-grep -Value Get-Item
    $aliasConflictDetected = $false
    try {
        & $installer `
            -Client custom `
            -InstallRoot (Join-Path $temporaryRoot 'alias-conflict-skills') `
            -ReleaseBaseUrl $releaseBaseUrl `
            -Yes
    } catch {
        $aliasConflictDetected = $_.Exception.Message.Contains('Another swe-grep')
    } finally {
        Remove-Item Alias:swe-grep -ErrorAction SilentlyContinue
    }
    if (-not $aliasConflictDetected) {
        throw 'Expected a PowerShell alias conflict.'
    }

    [IO.File]::WriteAllText((Join-Path $runtime '.env'), 'CUSTOM_CONFIG=kept')
    $upgradeRelease = Join-Path $temporaryRoot 'upgrade-release'
    $upgradePackage = Join-Path $upgradeRelease 'package'
    New-Item -ItemType Directory -Path $upgradePackage -Force | Out-Null
    $archiveName = 'swe-grep-windows-x86_64.zip'
    Expand-Archive `
        -LiteralPath (Join-Path $releaseDirectoryPath $archiveName) `
        -DestinationPath $upgradePackage
    Add-Content -LiteralPath (Join-Path $upgradePackage 'swe-grep\SKILL.md') -Value '# installer upgrade test'
    $upgradeArchive = Join-Path $upgradeRelease $archiveName
    Compress-Archive -Path (Join-Path $upgradePackage 'swe-grep') -DestinationPath $upgradeArchive
    $upgradeHash = (Get-FileHash -LiteralPath $upgradeArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$upgradeHash  $archiveName" | Out-File "$upgradeArchive.sha256" -Encoding ascii
    $upgradeBaseUrl = ([Uri]$upgradeRelease).AbsoluteUri
    $upgradeSkills = Join-Path $temporaryRoot 'upgrade-skills'

    $upgradeRequiredForce = $false
    try {
        & $installer -Client custom -InstallRoot $upgradeSkills -ReleaseBaseUrl $upgradeBaseUrl -Yes
    } catch {
        $upgradeRequiredForce = $_.Exception.Message.Contains('pass -Force')
    }
    if (-not $upgradeRequiredForce) {
        throw 'Expected a different runtime to require -Force.'
    }

    & $installer -Client custom -InstallRoot $upgradeSkills -ReleaseBaseUrl $upgradeBaseUrl -Force -Yes
    if (-not ([IO.File]::ReadAllText((Join-Path $runtime '.env')).Contains('CUSTOM_CONFIG=kept'))) {
        throw 'Runtime upgrade did not preserve .env.'
    }
    if (@(Get-ChildItem -LiteralPath $applicationRoot -Filter 'runtime.backup.*').Count -eq 0) {
        throw 'Runtime upgrade did not preserve a backup.'
    }

    & $installer -Client custom -InstallRoot $upgradeSkills -ReleaseBaseUrl $upgradeBaseUrl -Force -Yes
    if (@(Get-ChildItem -LiteralPath $temporaryRoot -Filter 'swe-grep.backup.*' -Recurse).Count -eq 0) {
        throw 'Forced Skill replacement did not preserve a backup.'
    }

    & $installer -Uninstall -Client custom -InstallRoot $firstSkills -Yes
    if (-not (Test-Path -LiteralPath $runtime -PathType Container) -or
        -not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
        throw 'Partial uninstallation removed the shared runtime or launcher.'
    }

    [IO.File]::WriteAllText((Join-Path $secondSkills 'swe-grep\user-file.txt'), 'preserved')
    & $installer -Uninstall -Client custom -InstallRoot $secondSkills -Yes
    if (-not (Test-Path -LiteralPath (Join-Path $secondSkills 'swe-grep\user-file.txt') -PathType Leaf) -or
        (Test-Path -LiteralPath (Join-Path $secondSkills 'swe-grep\SKILL.md'))) {
        throw 'Uninstallation did not preserve non-installer files correctly.'
    }

    & $installer -Uninstall -Client custom -InstallRoot $upgradeSkills -Yes
    if (Test-Path -LiteralPath $runtime) {
        throw 'Final uninstallation did not remove the shared runtime.'
    }
    if (Test-Path -LiteralPath $launcher) {
        throw 'Final uninstallation did not remove the managed launcher.'
    }
    $restoredUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($restoredUserPath -ne $controlledUserPath) {
        throw 'Uninstallation changed unrelated User PATH entries or separators.'
    }
    if ($env:Path -ne "$launcherDirectory;$originalProcessPath") {
        throw 'Uninstallation changed the current Process PATH.'
    }

    Write-Host 'Windows installer functional tests passed'
} finally {
    [Environment]::SetEnvironmentVariable('Path', $originalUserPath, 'User')
    $env:Path = $originalProcessPath
    Remove-Item -LiteralPath $applicationRoot -Recurse -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $savedApplicationRoot) {
        Move-Item -LiteralPath $savedApplicationRoot -Destination $applicationRoot
    }
    Remove-Item -LiteralPath $temporaryRoot -Recurse -Force -ErrorAction SilentlyContinue
}
