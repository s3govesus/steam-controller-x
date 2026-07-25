# Build sc-adapter and check/install its Windows prerequisites:
#   - a Rust toolchain
#   - the ViGEmBus driver (virtual Xbox 360 controller support)
#
# Safe to re-run. Installing ViGEmBus needs winget (or a manual download);
# you'll be asked before anything is installed.

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot

function Confirm-Action {
    param([string]$Prompt)
    $reply = Read-Host "$Prompt [y/N]"
    return $reply -match '^[Yy]$'
}

Write-Host "== Rust toolchain =="
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargo) {
    Write-Host "cargo found: $(cargo --version)"
} else {
    Write-Host "cargo not found. Install a Rust toolchain first: https://rustup.rs"
    exit 1
}

Write-Host ""
Write-Host "== ViGEmBus driver =="
$vigemService = Get-Service -Name "ViGEmBus" -ErrorAction SilentlyContinue
if ($vigemService) {
    Write-Host "ViGEmBus service found (status: $($vigemService.Status))."
} else {
    Write-Host "ViGEmBus not found. sc-adapter needs it to create the virtual Xbox 360 controller."
    $winget = Get-Command winget -ErrorAction SilentlyContinue
    if ($winget) {
        if (Confirm-Action "Install it now via winget (ViGEm.ViGEmBus)?") {
            winget install --id ViGEm.ViGEmBus -e
        } else {
            Write-Host "Skipping. Install manually from: https://github.com/ViGEm/ViGEmBus/releases"
        }
    } else {
        Write-Host "winget not available. Install manually from: https://github.com/ViGEm/ViGEmBus/releases"
    }
}

Write-Host ""
Write-Host "== Building (cargo build --release --workspace) =="
Push-Location $repoRoot
try {
    cargo build --release --workspace
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "== Done =="
Write-Host "Run it (auto-detects the PID):  .\scripts\run.ps1"
Write-Host "With the web UI:                .\scripts\run.ps1 --web"
Write-Host "See README.md for more."
