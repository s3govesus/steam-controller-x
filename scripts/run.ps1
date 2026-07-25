# Auto-detect the Steam Controller's USB product id and launch sc-adapter
# with it, instead of needing to run --list and pass --pid by hand.
# Any extra arguments (e.g. --web) are forwarded to sc-adapter as-is.
# (--grab-phantom-inputs is Linux-only and a no-op here.)
#
# Detection just scopes to Valve's vendor id (0x28de) and reads the PIDs
# sc-adapter --list reports back -- a single physical device can show up
# as more than one row (one per USB HID interface), so this dedupes by PID
# rather than assuming exactly one row means exactly one device.

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $repoRoot "target\release\sc-adapter.exe"

if (-not (Test-Path $bin)) {
    Write-Error "sc-adapter.exe not found at $bin -- run scripts\install-windows.ps1 (or 'cargo build --release --workspace') first."
    exit 1
}

$listOutput = & $bin --list

$uniquePids = @()
foreach ($line in $listOutput) {
    if ($line -match 'pid=(0x[0-9a-fA-F]+)') {
        $uniquePids += $matches[1]
    }
}
$uniquePids = $uniquePids | Select-Object -Unique

if ($uniquePids.Count -eq 0) {
    Write-Host "No Valve HID device found (vid 0x28de). Is the controller plugged in?"
    exit 1
} elseif ($uniquePids.Count -eq 1) {
    $controllerPid = $uniquePids[0]
    Write-Host "Detected controller: pid=$controllerPid"
    & $bin --pid $controllerPid @args
} else {
    Write-Host "Multiple distinct Valve devices found -- pass --pid explicitly to pick one:"
    $listOutput | ForEach-Object { Write-Host $_ }
    exit 1
}
