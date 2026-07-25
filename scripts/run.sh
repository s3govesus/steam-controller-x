#!/usr/bin/env bash
# Auto-detect the Steam Controller's USB product id and launch sc-adapter
# with it, instead of needing to run `--list` and pass `--pid` by hand.
# Any extra arguments (e.g. --web, --grab-phantom-inputs) are forwarded to
# sc-adapter as-is.
#
# Detection just scopes to Valve's vendor id (0x28de) and reads the PIDs
# `sc-adapter --list` reports back -- a single physical device can show up
# as more than one row (one per USB HID interface), so this dedupes by PID
# rather than assuming exactly one row means exactly one device.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
BIN="$REPO_ROOT/target/release/sc-adapter"

if [[ ! -x "$BIN" ]]; then
    echo "sc-adapter binary not found at $BIN -- run scripts/install-linux.sh (or 'cargo build --release --workspace') first." >&2
    exit 1
fi

list_output="$("$BIN" --list)"

unique_pids=()
while IFS= read -r pid; do
    unique_pids+=("$pid")
done < <(grep -oE 'pid=0x[0-9a-fA-F]+' <<<"$list_output" | cut -d= -f2 | sort -u)

case "${#unique_pids[@]}" in
    0)
        echo "No Valve HID device found (vid 0x28de). Is the controller plugged in?" >&2
        exit 1
        ;;
    1)
        pid="${unique_pids[0]}"
        echo "Detected controller: pid=$pid"
        exec "$BIN" --pid "$pid" "$@"
        ;;
    *)
        echo "Multiple distinct Valve devices found -- pass --pid explicitly to pick one:" >&2
        echo "$list_output" >&2
        exit 1
        ;;
esac
