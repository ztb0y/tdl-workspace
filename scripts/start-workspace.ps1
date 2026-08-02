[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$buildScript = Join-Path $PSScriptRoot "build-client.ps1"
$installRoot = Join-Path $workspaceRoot "bin\client"
$launcher = Join-Path $installRoot "tdl-web-launcher.exe"

& $buildScript
if ($LASTEXITCODE -ne 0) {
    throw "Client build failed"
}

$previousInstallRoot = $env:TDL_WEB_INSTALL_ROOT
try {
    $env:TDL_WEB_INSTALL_ROOT = $installRoot
    & $launcher
    if ($LASTEXITCODE -ne 0) {
        throw "Launcher exited with code $LASTEXITCODE"
    }
} finally {
    $env:TDL_WEB_INSTALL_ROOT = $previousInstallRoot
}
