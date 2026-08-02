[CmdletBinding()]
param(
    [string]$Version = "0.1.0",
    [switch]$RequireOfficialTdl
)

$ErrorActionPreference = "Stop"
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$goWorkPath = Join-Path $workspaceRoot "go.work"
$tdlDir = Join-Path $workspaceRoot "tdl"
$pluginDir = Join-Path $workspaceRoot "tdl-web"
$uiDir = Join-Path $workspaceRoot "tdl-web-ui"
$clientDir = Join-Path $workspaceRoot "client"
$binDir = Join-Path $workspaceRoot "bin"
$stageDir = Join-Path $binDir "client"
$runtimeDir = Join-Path $stageDir "runtime"
$bundleDir = Join-Path $clientDir "bundle"
$bundlePath = Join-Path $bundleDir "runtime.zip"

function Invoke-Checked([string]$workingDirectory, [string]$executable, [string[]]$arguments) {
    Push-Location $workingDirectory
    try {
        & $executable @arguments
        if ($LASTEXITCODE -ne 0) {
            throw "Command failed: $executable $($arguments -join ' ')"
        }
    } finally {
        Pop-Location
    }
}

function Invoke-RustRelease([string[]]$arguments) {
    $separator = [char]0x1f
    $flags = @(
        "-C",
        "target-feature=+crt-static",
        "--remap-path-prefix=$workspaceRoot=."
    )
    if ($env:USERPROFILE) {
        $flags += "--remap-path-prefix=$env:USERPROFILE=~"
    }
    $previous = $env:CARGO_ENCODED_RUSTFLAGS
    $encoded = $flags -join $separator
    if ($previous) {
        $encoded = "$previous$separator$encoded"
    }
    try {
        $env:CARGO_ENCODED_RUSTFLAGS = $encoded
        Invoke-Checked $clientDir "cargo.exe" $arguments
    } finally {
        $env:CARGO_ENCODED_RUSTFLAGS = $previous
    }
}

function Reset-GeneratedDirectory([string]$path) {
    $fullPath = [IO.Path]::GetFullPath($path)
    $fullRoot = [IO.Path]::GetFullPath($workspaceRoot) + [IO.Path]::DirectorySeparatorChar
    if (-not $fullPath.StartsWith($fullRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Generated path escaped workspace: $fullPath"
    }
    if (Test-Path -LiteralPath $fullPath) {
        Remove-Item -LiteralPath $fullPath -Recurse -Force
    }
    New-Item -ItemType Directory -Path $fullPath -Force | Out-Null
}

function Assert-NoMsvcRedistributable([string[]]$paths) {
    Push-Location $clientDir
    try {
        $targetLibDir = (rustc.exe --print target-libdir).Trim()
    } finally {
        Pop-Location
    }
    $readObj = Join-Path (Split-Path -Parent $targetLibDir) "bin\llvm-readobj.exe"
    if (-not (Test-Path -LiteralPath $readObj)) {
        throw "llvm-readobj.exe not found; install the llvm-tools-preview Rust component"
    }
    foreach ($path in $paths) {
        $imports = (& $readObj --coff-imports $path | Out-String)
        if ($LASTEXITCODE -ne 0) {
            throw "Unable to inspect PE imports: $path"
        }
        if ($imports -match '(?i)\b(?:VCRUNTIME|MSVCP)[^\s]*\.dll\b') {
            throw "Dynamic MSVC runtime dependency found in $path`: $($Matches[0])"
        }
    }
}

function Assert-GoReleasePrivacy([string[]]$paths) {
    foreach ($path in $paths) {
        $metadata = (& go.exe version -m $path | Out-String)
        if ($LASTEXITCODE -ne 0 -or $metadata -match '(?m)^\s*build\s+vcs\.') {
            throw "Go VCS metadata found in $path"
        }
        $previous = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            $symbols = (& go.exe tool nm $path 2>&1 | Out-String)
            $exitCode = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $previous
        }
        if ($exitCode -eq 0 -or $symbols -notmatch 'no symbols|no symbol section') {
            throw "Go symbol table found in $path"
        }
    }
}

function Assert-NoBuildPaths([string[]]$paths) {
    $prefixes = @($workspaceRoot, $env:USERPROFILE) | Where-Object { $_ }
    foreach ($path in $paths) {
        $content = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($path))
        foreach ($prefix in $prefixes) {
            if ($content.IndexOf($prefix, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                throw "Build path $prefix found in $path"
            }
        }
    }
}

foreach ($command in "go", "npm.cmd", "cargo") {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        throw "Command not found: $command"
    }
}
foreach ($path in $goWorkPath, $tdlDir, $pluginDir, $uiDir, $clientDir) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Workspace path not found: $path"
    }
}
$clientMetadata = cargo.exe metadata --manifest-path (Join-Path $clientDir "Cargo.toml") --no-deps --format-version 1 |
    ConvertFrom-Json
if ($LASTEXITCODE -ne 0) {
    throw "Unable to read client package version"
}
$clientVersion = $clientMetadata.packages[0].version
if ($Version -ne $clientVersion) {
    throw "Build version $Version does not match client version $clientVersion"
}

Reset-GeneratedDirectory $stageDir
New-Item -ItemType Directory -Path $runtimeDir -Force | Out-Null

if (-not (Test-Path -LiteralPath (Join-Path $uiDir "node_modules"))) {
    Invoke-Checked $uiDir "npm.cmd" @("ci")
}
Invoke-Checked $uiDir "node.exe" @("--test", "test/*.test.ts")
Invoke-Checked $uiDir "npm.cmd" @("run", "build")

$previousGoWork = $env:GOWORK
try {
    $env:GOWORK = $goWorkPath
    $goReleaseArgs = @("build", "-trimpath", "-buildvcs=false", "-ldflags", "-s -w -buildid=")
    Invoke-Checked $tdlDir "go.exe" ($goReleaseArgs + @("-o", (Join-Path $runtimeDir "tdl.exe"), "."))
    Invoke-Checked $pluginDir "go.exe" ($goReleaseArgs + @("-o", (Join-Path $runtimeDir "tdl-web.exe"), "."))
} finally {
    $env:GOWORK = $previousGoWork
}
Assert-GoReleasePrivacy @(
    (Join-Path $runtimeDir "tdl.exe"),
    (Join-Path $runtimeDir "tdl-web.exe")
)

Copy-Item -LiteralPath (Join-Path $uiDir "dist") -Destination (Join-Path $runtimeDir "ui") -Recurse
Invoke-Checked $clientDir "cargo.exe" @("test", "--locked")
Invoke-RustRelease @("build", "--locked", "--release", "--bin", "tdl-web-launcher", "--bin", "tdl-web-uninstaller")
Copy-Item -LiteralPath (Join-Path $clientDir "target\release\tdl-web-launcher.exe") -Destination $stageDir
Copy-Item -LiteralPath (Join-Path $clientDir "target\release\tdl-web-uninstaller.exe") -Destination $stageDir
Assert-NoMsvcRedistributable (Get-ChildItem -LiteralPath $stageDir -Filter "*.exe" -File -Recurse).FullName

$tdlCommit = (git -C $tdlDir rev-parse HEAD).Trim()
$pluginCommit = (git -C $pluginDir rev-parse HEAD).Trim()
$uiCommit = (git -C $uiDir rev-parse HEAD).Trim()
$workspaceCommit = (git -C $workspaceRoot rev-parse HEAD).Trim()
$tdlGitlink = ((git -C $workspaceRoot ls-tree HEAD tdl) -split '\s+')[2]
if ($RequireOfficialTdl -and ($tdlGitlink -ne "9359aa28ed7b423a9350f61e87ec38d7030e1e9c" -or $tdlCommit -ne $tdlGitlink)) {
    throw "Release tdl must be the official v0.20.3 gitlink 9359aa2"
}
$components = @(
    Get-ChildItem -LiteralPath $stageDir -File -Recurse | ForEach-Object {
        $relative = $_.FullName.Substring($stageDir.Length).TrimStart('\').Replace('\', '/')
        $name = if ($relative -eq "runtime/tdl.exe") { "tdl" }
            elseif ($relative -eq "runtime/tdl-web.exe") { "tdl-web" }
            elseif ($relative.StartsWith("runtime/ui/")) { "tdl-web-ui" }
            elseif ($relative -eq "tdl-web-launcher.exe") { "launcher" }
            else { "uninstaller" }
        $commit = switch ($name) {
            "tdl" { $tdlCommit }
            "tdl-web" { $pluginCommit }
            "tdl-web-ui" { $uiCommit }
            default { $workspaceCommit }
        }
        [ordered]@{
            name = $name
            version = $Version
            commit = $commit
            path = $relative
            size = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
)
$componentManifest = [ordered]@{
    schema_version = 1
    version = $Version
    platform = "windows-x64"
    components = $components
}
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[IO.File]::WriteAllText(
    (Join-Path $stageDir "components.json"),
    ($componentManifest | ConvertTo-Json -Depth 6),
    $utf8NoBom
)

if (Test-Path -LiteralPath $bundleDir) {
    Remove-Item -LiteralPath $bundleDir -Recurse -Force
}
New-Item -ItemType Directory -Path $bundleDir -Force | Out-Null
Compress-Archive -Path (Join-Path $stageDir "*") -DestinationPath $bundlePath -CompressionLevel Optimal
Invoke-RustRelease @("build", "--locked", "--release", "--features", "bundled-setup", "--bin", "tdl-web-setup")
Assert-NoMsvcRedistributable @((Join-Path $clientDir "target\release\tdl-web-setup.exe"))
Assert-NoBuildPaths @(
    (Join-Path $runtimeDir "tdl.exe"),
    (Join-Path $runtimeDir "tdl-web.exe"),
    (Join-Path $stageDir "tdl-web-launcher.exe"),
    (Join-Path $stageDir "tdl-web-uninstaller.exe"),
    (Join-Path $clientDir "target\release\tdl-web-setup.exe")
)

$setupPath = Join-Path $binDir "tdl-web-setup-windows-x64.exe"
Copy-Item -LiteralPath (Join-Path $clientDir "target\release\tdl-web-setup.exe") -Destination $setupPath -Force
$setupInfo = Get-Item -LiteralPath $setupPath
$releaseManifest = [ordered]@{
    schema_version = 1
    version = $Version
    platform = "windows-x64"
    setup = [ordered]@{
        filename = "tdl-web-setup-windows-x64.exe"
        size = $setupInfo.Length
        sha256 = (Get-FileHash -LiteralPath $setupPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    components = $components
}
[IO.File]::WriteAllText(
    (Join-Path $binDir "release-manifest.json"),
    ($releaseManifest | ConvertTo-Json -Depth 6),
    $utf8NoBom
)

Write-Host "Built offline setup: $setupPath" -ForegroundColor Green
