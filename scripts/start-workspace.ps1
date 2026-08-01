[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$workspaceRoot = Split-Path -Parent $PSScriptRoot
$goWorkPath = Join-Path $workspaceRoot "go.work"
$tdlDir = Join-Path $workspaceRoot "tdl"
$pluginDir = Join-Path $workspaceRoot "tdl-web"
$uiDir = Join-Path $workspaceRoot "tdl-web-ui"
$binDir = Join-Path $workspaceRoot "bin"
$tdlExe = Join-Path $binDir "tdl.exe"
$pluginExe = Join-Path $binDir "tdl-web.exe"

function Require-Command([string]$name) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        throw "Command not found: $name"
    }
}

function Invoke-WorkspaceCommand([string]$workingDirectory, [string]$executable, [string[]]$arguments) {
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

function Assert-PortAvailable([int]$port) {
    $ports = [System.Net.NetworkInformation.IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners() |
        ForEach-Object { $_.Port }
    if ($ports -contains $port) {
        throw "Port $port is already in use. Stop the existing workspace process first."
    }
}

function Wait-ForPort([int]$port, [int]$timeoutSeconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($timeoutSeconds)
    do {
        $client = New-Object System.Net.Sockets.TcpClient
        try {
            $client.Connect("127.0.0.1", $port)
            return
        } catch {
        } finally {
            $client.Dispose()
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for port $port"
}

function Stop-ChildProcess($process) {
    if ($null -eq $process) {
        return
    }
    $process.Refresh()
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
}

foreach ($path in @($goWorkPath, $tdlDir, $pluginDir, $uiDir)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Workspace path not found: $path"
    }
}

Require-Command go
Require-Command npm
Assert-PortAvailable 8080
Assert-PortAvailable 5173

New-Item -ItemType Directory -Force -Path $binDir | Out-Null

$previousGoWork = $env:GOWORK
$previousApiBase = $env:VITE_API_BASE
$apiProcess = $null
$uiProcess = $null

try {
    $env:GOWORK = $goWorkPath

    Invoke-WorkspaceCommand $tdlDir "go" @("build", "-o", $tdlExe, ".")
    Invoke-WorkspaceCommand $pluginDir "go" @("build", "-o", $pluginExe, ".")

    if (-not (Test-Path -LiteralPath (Join-Path $uiDir "node_modules"))) {
        Invoke-WorkspaceCommand $uiDir "npm" @("ci")
    }

    Invoke-WorkspaceCommand $workspaceRoot $tdlExe @("extension", "install", "--force", $pluginExe)

    $apiProcess = Start-Process -FilePath $tdlExe -ArgumentList @("web") -WorkingDirectory $workspaceRoot -PassThru -NoNewWindow
    $env:VITE_API_BASE = "http://127.0.0.1:8080"
    $npmPath = (Get-Command "npm.cmd" -ErrorAction Stop).Source
    $uiProcess = Start-Process -FilePath $npmPath -ArgumentList @("run", "dev", "--", "--host", "127.0.0.1", "--port", "5173", "--strictPort") -WorkingDirectory $uiDir -PassThru -NoNewWindow

    Wait-ForPort 8080 20
    Wait-ForPort 5173 20
    Start-Process "http://localhost:5173"
    Write-Host "tdl web is running. Press Ctrl+C or close this window to stop it." -ForegroundColor Green

    while ($true) {
        $apiProcess.Refresh()
        $uiProcess.Refresh()
        if ($apiProcess.HasExited) {
            throw "tdl web exited with code $($apiProcess.ExitCode)"
        }
        if ($uiProcess.HasExited) {
            throw "Vite exited with code $($uiProcess.ExitCode)"
        }
        Start-Sleep -Seconds 1
    }
} finally {
    Stop-ChildProcess $uiProcess
    Stop-ChildProcess $apiProcess
    $env:GOWORK = $previousGoWork
    $env:VITE_API_BASE = $previousApiBase
}
