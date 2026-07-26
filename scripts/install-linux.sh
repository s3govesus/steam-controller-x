#!/usr/bin/env bash
# Build sc-adapter and set up the system prerequisites it needs on Linux:
#   - a Rust toolchain and build dependencies (pkg-config, libudev)
#   - the udev rule granting /dev/uinput access to the `input` group
#   - your user account being a member of that group
#
# Safe to re-run; each step is skipped if already satisfied. System-wide
# changes (installing packages, writing the udev rule, group membership)
# each ask for confirmation before using sudo.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
UDEV_RULE_SRC="$SCRIPT_DIR/60-steam-controller-x-uinput.rules"
UDEV_RULE_DST="/etc/udev/rules.d/60-steam-controller-x-uinput.rules"

confirm() {
    # confirm "prompt text" -- returns 0 (yes) or 1 (no)
    local reply
    read -r -p "$1 [y/N] " reply
    [[ "$reply" =~ ^[Yy]$ ]]
}

echo "== Rust toolchain =="
if command -v cargo >/dev/null 2>&1; then
    echo "cargo found: $(cargo --version)"
else
    echo "cargo not found. Install a Rust toolchain first: https://rustup.rs"
    exit 1
fi

echo
echo "== Build dependencies (pkg-config, libudev) =="
if pkg-config --exists libudev 2>/dev/null; then
    echo "libudev found: $(pkg-config --modversion libudev)"
else
    echo "libudev development files not found via pkg-config."
    if command -v pacman >/dev/null 2>&1; then
        install_cmd="sudo pacman -S --needed pkgconf systemd-libs"
    elif command -v apt-get >/dev/null 2>&1; then
        install_cmd="sudo apt-get install -y pkg-config libudev-dev"
    elif command -v dnf >/dev/null 2>&1; then
        install_cmd="sudo dnf install -y pkgconf-pkg-config systemd-devel"
    elif command -v zypper >/dev/null 2>&1; then
        install_cmd="sudo zypper install -y pkgconf-pkg-config libudev-devel"
    else
        install_cmd=""
    fi

    if [[ -n "$install_cmd" ]]; then
        echo "Suggested: $install_cmd"
        if confirm "Run that now?"; then
            eval "$install_cmd"
        else
            echo "Skipping -- the build below may fail without it."
        fi
    else
        echo "Unrecognized package manager -- install a libudev development"
        echo "package (provides libudev.pc) manually, then re-run this script."
    fi
fi

echo
echo "== Building (cargo build --release --workspace) =="
(cd "$REPO_ROOT" && cargo build --release --workspace)

echo
echo "== /dev/uinput udev rule =="
if [[ -f "$UDEV_RULE_DST" ]]; then
    echo "Already installed: $UDEV_RULE_DST"
else
    echo "This lets the 'input' group (which you'll also be added to below)"
    echo "access /dev/uinput without running sc-adapter as root."
    if confirm "Install $UDEV_RULE_DST and reload udev rules now?"; then
        sudo cp "$UDEV_RULE_SRC" "$UDEV_RULE_DST"
        sudo udevadm control --reload-rules
        sudo udevadm trigger
        echo "Installed."
    else
        echo "Skipping -- you'll need to run sc-adapter as root instead, or"
        echo "install $UDEV_RULE_SRC to $UDEV_RULE_DST yourself later."
    fi
fi

echo
echo "== 'input' group membership =="
if id -nG "$USER" | tr ' ' '\n' | grep -qx input; then
    echo "$USER is already in the 'input' group."
else
    if confirm "Add $USER to the 'input' group now?"; then
        sudo usermod -aG input "$USER"
        echo "Added. This takes effect on your next login (or run 'newgrp input' in this shell)."
    else
        echo "Skipping."
    fi
fi

echo
echo "== Done =="
echo "Run it (auto-detects the PID):  ./scripts/run.sh"
echo "With the web UI:                ./scripts/run.sh --web"
echo
echo "Common ./scripts/run.sh flags (forwarded to sc-adapter):"
echo "  --web                    serve the local web UI (monitoring + remap/deadzone config)"
echo "  --web-bind <addr>        web UI bind address (default 127.0.0.1)"
echo "  --web-port <port>        web UI port (default 61302)"
echo "  --grab-phantom-inputs    exclusively grab this controller's phantom mouse/keyboard devices"
echo "  --test-rumble a|b|c|d    trigger one haptic pulse on the given motor and exit"
echo "  --dump-raw               print raw HID reports instead of driving the virtual pad"
echo "  --pid <pid>              override the auto-detected controller PID"
echo "  -v, -vv                  increase log verbosity"
echo "See README.md for more."
