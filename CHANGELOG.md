# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3] - 2026-07-29

### Added

- `sc-hid`: two scratch diagnostic examples for the grip-squeeze
  investigation below — `grip_probe.rs` (watches one byte over time,
  distinguishing a real sensor reading from a free-running counter) and
  `isolate_grip_byte.rs` (diffs every byte's value range across an
  idle/squeeze/release cycle to isolate a real touch-correlated signal).

### Fixed

- `sc-protocol`: report byte 32 was mislabeled `grip` (analog grip-squeeze
  pressure, "49 unsqueezed to 130+ squeezed") based on a single before/after
  squeeze test. A dedicated probe found it actually free-runs on its own at
  a steady ~14Hz regardless of touch, sweeping its full `0..=255` range and
  wrapping every ~18s — this is also what was making the web UI's "Grip"
  meter climb to 100%, plateau for several seconds, and reset in a loop
  even with nobody touching the controller. Renamed to
  `PadState::unknown_counter_32` and pulled the misleading meter from the
  web UI entirely; see "Investigated" below for the full write-up.

### Investigated

- Grip-squeeze pressure (analog): re-investigated (2026-07-29) after the
  fix above. A proper isolation test (idle baseline / sustained squeeze /
  release, diffed per-byte) found no byte in report `0x42` that shifts to a
  new stable range during a squeeze and returns to baseline after release —
  so no analog pressure signal was found in this report at all. Only the
  already-confirmed *digital* squeezed/not-squeezed bit remains reliable.
  See the "Protocol status" section of the README for the full write-up.

### Documentation

- Noted in the README that an unloaded `uinput` kernel module makes
  `sc-adapter` fail immediately with "Error: Device not found." — from the
  virtual-pad creation step, before it ever tries the physical controller,
  so it reads like a controller-detection failure (and can make
  `scripts/run.sh` look like the culprit) but isn't one.

## [0.1.1] - 2026-07-25

Initial tagged release. (Retroactively added to the changelog — this
content sat undocumented under a stale "Unreleased" heading through the
0.1.2 release; see 0.1.3's cleanup above.)

### Added

- `sc-protocol`: pure decoding logic for the new (2026) Steam Controller's
  raw USB HID input reports (report ID `0x42`), reverse-engineered against
  real hardware — all 21 digital buttons, both trigger axes, two sticks,
  two touchpads, 6-axis IMU, capacitive grip/stick-touch sensing.
- `sc-hid`: USB HID device discovery and I/O via `hidapi`, including
  wireless-puck interface probing and haptic rumble output reports for the
  controller's 4 motors.
- `sc-xinput`: platform-agnostic `VirtualPad`/`XInputState` types, plus
  concrete backends — `windows_vigem` (ViGEmBus) and `linux_uinput`
  (kernel `uinput`), each emulating an Xbox 360 controller.
- `sc-config`: deadzone/response-curve axis shaping, a remappable
  physical-control-to-XInput-button table (`LogicalButton` -> `XInputButton`),
  and `Profile`/`ProfileStore` for saving/loading named TOML configs.
- `sc-adapter`: the CLI binary — device discovery (`--list`), the main
  HID-to-XInput adapter loop with auto-reconnect, `--dump-raw` and
  `--replay` for protocol reverse-engineering, `--test-rumble`, and
  `--grab-phantom-inputs` (Linux) to exclusively grab the kernel's phantom
  mouse/keyboard `evdev` devices this controller's HID interfaces trigger.
- Optional local web UI (`--web`/`--web-bind`/`--web-port`): live
  button/stick/touchpad/IMU monitoring over WebSocket, deadzone/curve
  tuning, button remapping, profile management, and rumble-test buttons.
  Buttons are grouped by physical location (face/D-pad/shoulders &
  paddles/sticks & pads/system) rather than dumped in one flat grid; stick
  diagrams overlay a live deadzone ring; the touchpad diagrams reflect
  touch/press state (the left pad's press is inferred from the D-pad
  buttons, since it doubles as the D-pad and has no dedicated click flag);
  unsaved profile edits are tracked with a dirty indicator and a
  confirm-before-discard prompt on Load; and errors/save confirmations
  surface as toasts instead of `alert()` popups.
- `scripts/install-linux.sh` and `scripts/install-windows.ps1`: idempotent
  setup scripts (build deps, loading the `uinput` kernel module and
  persisting that across reboots, the `uinput` udev rule + group
  membership on Linux; ViGEmBus install on Windows).
- `scripts/run.sh` / `scripts/run.ps1`: auto-detect the controller's USB
  PID and launch `sc-adapter` without needing to pass `--pid` by hand.
- `tools/pygame_test.py`: standalone SDL2-based script for exercising the
  virtual pad independent of this project's own code.

### Changed

- Default web UI port changed from `8080` to `61302` (IANA dynamic/private
  range) to avoid clashing with other local dev servers/proxies that
  commonly default to `8080`/"http-alt".

### Documentation

- Added a disclaimer to the README: this project speaks raw,
  reverse-engineered HID reports (including motor-driving output reports)
  and is provided with no warranty — used at your own risk.
- Documented a known issue where the Steam client itself (not just
  SDL-based games) misidentifies this controller as the 2015 Steam
  Controller and can pop its on-screen keyboard on a face-button press;
  no fix found yet.

## [0.1.2] - 2026-07-26

### Added

- `sc-protocol`: the real right-*stick* click (report byte 2, bit `0x20`),
  distinct from the right *touchpad*'s press-force click — reverse-engineered
  against real hardware via a dedicated capture plus a touchpad-click
  negative control. `sc-config`'s default profile now binds XInput's RTHUMB
  to this real click instead of the touchpad-click stand-in used previously.
- `sc-protocol`/`sc-adapter`: wireless RF signal strength, in dBm
  (`Telemetry::signal_dbm`), decoded from a separate HID report (`0x7b`)
  found by reading the device's HID report descriptor directly rather than
  by pressing controls. Shown live in the web UI header, color-coded.
- `sc-hid`/`sc-adapter`: the paired controller's serial number, read via
  HID feature report `0x02` (`GET_FEATURE_REPORT`) rather than the OS-level
  HID string descriptor, which over the wireless puck reports the puck's
  own serial instead of the controller's. Shown live in the web UI header.

### Fixed

- `sc-adapter`'s web UI: `ProfileStore`'s synchronous file I/O
  (save/load/list/delete/set-active) now runs via `tokio::task::spawn_blocking`
  instead of directly on the async request-handling task, so a slow/blocked
  disk no longer stalls every other in-flight web UI request.
- `sc-adapter`'s main read loop only ever recognized report id `0x42`,
  silently discarding every other report type as if it were a read
  timeout — this is what was hiding the `0x7b` telemetry report above.
  Now dispatches by report id.

### Investigated

- Re-verified the right bumper's previously-noted intermittent contact with
  two fresh continuous-hold captures: confirmed it's a single clean
  contiguous dropout (not per-sample flicker), consistent with the
  tester's finger briefly lifting off the button rather than a hardware or
  decoding fault, per the hardware owner. Closed out — not an open issue.
- Extensively investigated battery-level reporting and did not find it —
  see the "Battery level" entry under README's "Protocol status" for the
  full write-up. Summary: report `0x42` bytes 46-53 never move under any
  tested condition (idle, every control, ~50 minutes of continuous
  max-rumble drain, a full wired charge cycle); bytes 30-31 are a
  "charging active" clock, not a percentage; the newly-found `0x7b` and
  `0x43` reports (see "Added" above) carry more charging-state flags and
  noisy telemetry but nothing that trends like a battery percentage; a
  feature-report `0x01` response (`01 8e 00...00`) fired exactly once and
  never reproduced across 21 follow-up attempts. Closed out as
  unresolved — documented rather than guessed at.

[0.1.3]: https://github.com/s3govesus/steam-controller-x/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/s3govesus/steam-controller-x/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/s3govesus/steam-controller-x/releases/tag/v0.1.1
