[CmdletBinding()]
param(
    [switch]$NoPause,
    [switch]$NoOpen
)

# PowerShell 5.1 compatible; the .bat owns the interactive pause.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$appRoot = $repoRoot
$releaseRoot = Join-Path $repoRoot 'release\kanban'
$targetRoot = Join-Path $repoRoot 'release\kanban-build'
$sourceRoot = Join-Path $repoRoot 'release\kanban-source'
$target = 'x86_64-pc-windows-msvc'
$buildLock = $null
$transcribing = $false
$exitCode = 1

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments)
    Write-Host ("> {0} {1}" -f $Command, ($Arguments -join ' '))
    # Native build tools write normal progress to stderr in Windows PowerShell.
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & $Command @Arguments 2>&1 | ForEach-Object {
            $line = if ($_ -is [System.Management.Automation.ErrorRecord]) { $_.Exception.Message } else { [string]$_ }
            if (-not [string]::IsNullOrWhiteSpace($line)) { Write-Host $line }
        }
        $nativeExit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($nativeExit -ne 0) { throw "$Command failed (exit $nativeExit)." }
}

try {
    New-Item -ItemType Directory -Force -Path $releaseRoot | Out-Null
    # Keep concurrent double-clicks from rebuilding/copying the same outputs.
    $buildLock = [IO.File]::Open((Join-Path $releaseRoot '.build.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
    $logRoot = Join-Path $releaseRoot 'logs'
    New-Item -ItemType Directory -Force -Path $logRoot | Out-Null
    $logPath = Join-Path $logRoot ("build-{0}-{1}.log" -f (Get-Date -Format 'yyyyMMdd-HHmmss'), $PID)
    Start-Transcript -Path $logPath | Out-Null
    $transcribing = $true
    Write-Host "Herdr Kanban - Windows x64 release"
    Write-Host "Log: $logPath"
    # Mirror the repository-root app; exclude release to avoid copying the destination into itself.
    # Never npm ci in the live dev tree.
    $expectedSource = [IO.Path]::GetFullPath((Join-Path $repoRoot 'release\kanban-source'))
    if ([IO.Path]::GetFullPath($sourceRoot) -ne $expectedSource) { throw 'Unsafe build workspace path.' }
    foreach ($directory in @((Join-Path $repoRoot 'release'), $sourceRoot)) {
        if ((Test-Path -LiteralPath $directory) -and ((Get-Item -LiteralPath $directory -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Build workspace must not be a junction or symbolic link: $directory"
        }
    }
    & robocopy.exe $appRoot $sourceRoot /MIR /XJ /XD node_modules dist target .git release .scratch /XF '*.log' /NFL /NDL /NJH /NJS /NP
    if ($LASTEXITCODE -gt 7) { throw "Source snapshot failed (robocopy exit $LASTEXITCODE)." }
    Set-Location -LiteralPath $sourceRoot

    foreach ($command in @('node.exe', 'npm.cmd', 'cargo.exe', 'rustup.exe')) {
        if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
            throw "Missing $command. Install Node.js LTS and Rust MSVC, then reopen this script. See docs/kanban-packaging.md."
        }
    }
    $nodeVersion = [version]((& node.exe --version).TrimStart('v'))
    if (-not (($nodeVersion.Major -eq 20 -and $nodeVersion -ge [version]'20.19.0') -or $nodeVersion -ge [version]'22.12.0')) {
        throw "Node.js $nodeVersion is unsupported. Use Node.js 22.12+ LTS."
    }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) {
        throw 'Visual Studio Build Tools not found. Install Desktop development with C++, including a Windows SDK.'
    }
    $vsInstall = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vsInstall) { throw 'MSVC x64 build tools not found. Install the Desktop development with C++ workload.' }
    $sdk = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue
    if (-not $sdk -or -not $sdk.KitsRoot10 -or -not (Test-Path -LiteralPath (Join-Path $sdk.KitsRoot10 'Include'))) {
        throw 'Windows 10/11 SDK not found. Add it with Visual Studio Installer.'
    }
    $installedTargets = & rustup.exe target list --installed
    if ($LASTEXITCODE -ne 0 -or $target -notin $installedTargets) {
        throw "Rust target missing. Run: rustup target add $target"
    }

    $config = Get-Content 'src-tauri\tauri.conf.json' -Raw | ConvertFrom-Json
    $package = Get-Content 'package.json' -Raw | ConvertFrom-Json
    # package-lock has an empty property name that ConvertFrom-Json in PS 5.1 rejects.
    $npmLockVersions = & node.exe -p "JSON.stringify([require('./package-lock.json').version, require('./package-lock.json').packages[''].version])"
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read package-lock.json.' }
    $npmLockVersions = $npmLockVersions | ConvertFrom-Json
    $cargoToml = Get-Content 'src-tauri\Cargo.toml' -Raw
    $cargoVersion = [regex]::Match($cargoToml, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
    $cargoVersion = [regex]::Match($cargoVersion.Groups[1].Value, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    $version = $config.version
    if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Use a stable numeric version such as 0.1.0 in tauri.conf.json.' }
    if ($package.version -ne $version -or $cargoVersion -ne $version -or $npmLockVersions[0] -ne $version -or $npmLockVersions[1] -ne $version) {
        throw 'Version mismatch: package.json, package-lock.json, Cargo.toml and tauri.conf.json must agree. See docs/kanban-packaging.md.'
    }
    # NSIS restores its path using publisher + productName, not identifier alone.
    if ($config.productName -ne 'kanban' -or $config.identifier -ne 'com.administrator.kanban' -or $config.bundle.publisher -ne 'administrator' -or $config.bundle.windows.nsis.installMode -ne 'currentUser') {
        throw 'Installer identity changed. Keep the existing productName, identifier, publisher and currentUser mode to preserve upgrades.'
    }

    Write-Host '[1/3] Restoring locked frontend dependencies...'
    Invoke-Checked 'npm.cmd' @('ci', '--no-audit', '--no-fund')
    Write-Host '[2/3] Building release executable and NSIS installer...'
    # Explicit target directory isolates release output from a running dev build.
    $env:CARGO_TARGET_DIR = $targetRoot
    if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = '4' }
    Invoke-Checked 'npm.cmd' @('run', 'tauri', '--', 'build', '--ci', '--target', $target, '--bundles', 'nsis', '--', '--locked')

    Write-Host '[3/3] Publishing this successful build locally...'
    $installer = Join-Path $targetRoot "$target\release\bundle\nsis\kanban_${version}_x64-setup.exe"
    if (-not (Test-Path -LiteralPath $installer)) { throw "Expected installer missing: $installer" }
    $hasher = [Security.Cryptography.SHA256]::Create()
    $installerStream = [IO.File]::OpenRead($installer)
    try {
        $hash = [BitConverter]::ToString($hasher.ComputeHash($installerStream)).Replace('-', '').ToLowerInvariant()
    } finally {
        $installerStream.Dispose()
        $hasher.Dispose()
    }
    $output = Join-Path $releaseRoot $version
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    $destination = Join-Path $output "Herdr-Kanban-${version}-windows-x64-setup.exe"
    Copy-Item -LiteralPath $installer -Destination $destination -Force
    "$hash  $([IO.Path]::GetFileName($destination))" | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.txt') -Encoding ASCII
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\kanban-distribution.txt') -Destination (Join-Path $output 'README.txt') -Force
    Write-Host "SUCCESS: $destination"
    Write-Host "SHA256: $hash"
    if (-not $NoOpen) { Start-Process explorer.exe -ArgumentList ('"{0}"' -f $output) | Out-Null }
    $exitCode = 0
} catch {
    Write-Host "ERROR: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host $_.ScriptStackTrace
    if ($null -eq $buildLock) { Write-Host 'Another build may be running, or the release folder is not writable.' }
} finally {
    if ($transcribing) { Stop-Transcript | Out-Null }
    if ($null -ne $buildLock) { $buildLock.Dispose() }
}
exit $exitCode
