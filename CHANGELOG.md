# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing has been tagged/released yet — everything below is the initial,
in-progress implementation.

### Added

- `sc-protocol`: pure decoding logic for the new (2026) Steam Controller's
  raw USB HID input reports (report ID `0x42`), reverse-engineered against
  real hardware — all 20 digital buttons, both trigger axes, two sticks,
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
- `scripts/install-linux.sh` and `scripts/install-windows.ps1`: idempotent
  setup scripts (build deps, `uinput` udev rule + group membership on
  Linux; ViGEmBus install on Windows).
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

[Unreleased]: https://github.com/s3govesus/steam-controller-x/commits/main
