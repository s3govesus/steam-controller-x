# steam-controller-x

A standalone Rust daemon that reads the **new (2026) Steam Controller's**
raw USB HID reports and drives a virtual XInput-compatible gamepad, so
games/emulators that only speak XInput can use it without going through
Steam. Includes an optional local web UI for live input monitoring/testing
and remap/deadzone configuration.

> **Not the 2015 Steam Controller.** This targets Valve's new Steam
> Controller (USB VID `0x28de`, wired PID `0x1302`, or PID `0x1304` when
> connected via its wireless receiver/"puck" — see
> [Wireless (dongle/puck)](#wireless-donglepuck) below), a different,
> unreleased-at-time-of-writing device with its own undocumented HID
> protocol. It does **not** work with the original 2015 Steam Controller
> (VID `0x28de`, PID `0x1102`), which already has mature Linux support via
> Steam Input and the kernel's `hid-steam` driver, and doesn't need this
> project at all.

- **Windows**: emulates an Xbox 360 controller via [ViGEmBus](https://github.com/ViGEm/ViGEmBus).
- **Linux**: creates a virtual `uinput` device shaped like an Xbox 360 pad (name, VID/PID, and evdev capabilities), which SDL2/Proton recognize as XInput-capable without needing the kernel `xpad` driver.

> **Disclaimer / use at your own risk.** This project talks to your
> controller over raw, reverse-engineered USB HID reports (see "Protocol
> status" below) — the wire format was derived empirically, not from any
> official spec, and includes output reports that drive the haptic
> motors directly. It is provided with no warranty of any kind. The
> author(s) are not responsible for any damage — to this controller, to
> any other hardware, or otherwise — that may result from building,
> running, or modifying this software. Use it at your own risk.

## Protocol status: verified against real hardware

`crates/sc-protocol/src/report.rs` implements the HID input report layout
for the new Steam Controller (USB VID `0x28de`, PID `0x1302`), determined
empirically across several sessions (2026-07-25) by capturing raw reports
from a real unit and isolating one control at a time. This hardware has
**two analog sticks and two touchpads** (like Steam Deck), plus capacitive
grip/stick-touch sensing, a 6-axis IMU, and haptic motors — all distinct
from the original Steam Controller's protocol, which doesn't apply here at
all (different report ID, different everything).

Confirmed:

- Report ID (`0x42`), 1-byte sequence counter.
- All 21 digital buttons: face buttons (A/B/X/Y), the "dots" quick-access
  button, both bumpers, start/back/guide, *both* stick clicks, the right
  touchpad click, D-pad, all four back paddles. Separately, a
  right-touchpad touch-detection flag also exists (fires on contact,
  alongside but distinct from the click) — not counted in the 21 since
  it's more a proximity sensor than a press, same treatment as the two
  stick cap-touch sensors below.
  - **Correction:** a bit originally captured under a "right bumper" prompt
    turned out to be the right stick's capacitive touch sensor (fires
    while gripping/resting a thumb on the stick — presumably used to
    suppress the co-located touchpad, per the hardware owner) rather than
    a button at all. The real right bumper was found separately; see
    `ButtonFlags` doc comments in `sc-protocol` for both. Left bumper was
    unaffected — confirmed as a genuine button.
  - **Real right-*stick* click found (2026-07-26):** a bit previously
    logged as "never observed to fire" (report byte 2, `0x20`) is the real
    right-thumbstick click — distinct from the right *touchpad*'s
    press-force click. Isolated via a dedicated capture (fires alongside
    the right stick's cap-touch bit, since a thumb has to be on the stick
    to click it) plus a touchpad-click negative control that never set it.
    XInput's RTHUMB now binds to this real click instead of the
    touchpad-click stand-in used previously.
  - **Right bumper re-verified (2026-07-26):** two fresh ~6.5s
    continuous-hold captures on the same unit — one held at 100% duty
    cycle throughout, the other dropped for a single contiguous ~1.0s gap
    partway through an otherwise-solid hold. The drop is one clean
    contiguous run in the raw bit (not per-sample flicker), consistent
    with the tester's finger briefly lifting off the button during that
    hold rather than a hardware or decoding fault, per the hardware
    owner. Not treated as an open issue.
- Both trigger axes (u16, 0..32767).
- Left stick, right stick, left touchpad, right touchpad — four distinct
  analog surfaces, sign and full-scale range verified in every direction
  for each.
- The 6-axis IMU block (accelerometer + gyroscope) — distinguished via
  static-tilt-and-hold tests (accelerometer settles to a stable non-zero
  value under a fixed tilt; gyroscope spikes during motion and returns
  near zero once still). Exact per-channel axis semantics (which channel
  is X/Y/Z or pitch/roll/yaw) are *not* confirmed beyond one gyro channel
  being the best candidate for yaw.
- A capacitive grip-squeeze pressure sensor — but it appears to be a
  single combined reading, not independent per-hand sensors (squeezing
  either grip alone moved the same byte by about the same amount).
- The left stick's capacitive cap-touch sensor (counterpart to the right
  stick's) — but encoded completely differently: not a bit in the normal
  button bitmask at all, just a "status/mode" byte (report offset 5, also
  used for various other transient states) reading exactly `0x01`.
  Reproduced 100% of a held grip. Because that byte seems to hold one
  exclusive value at a time rather than independent bits, this flag isn't
  yet confirmed to coexist reliably with other simultaneous input the way
  the right stick's bitmask-based flag does.
- 4 haptic motors, via **output** reports (not documented anywhere
  public — found by reading the actual HID report descriptor to get valid
  report IDs/lengths, then sweeping byte positions and asking a human
  what they felt). See `sc_hid::Motor` doc comments for per-motor
  confidence levels; two of the four are solid and reproducible, one is
  moderate, one is low-confidence. XInput's own rumble API only carries 2
  intensities regardless, so only 2 of the 4 have an actual path through
  this adapter's output today. The controller has **no separate
  speaker** — sound (e.g. UI feedback tones) is produced through these
  same haptic motors rather than a dedicated audio component, which is
  also why the earlier hunt for speaker-sized output reports never found
  anything.
- **Wireless RF signal strength**, in dBm — but from a completely separate
  HID report (`0x7b`, 13 bytes), not the main `0x42` input stream. Found by
  reading the device's HID report descriptor directly (e.g.
  `/sys/class/hidraw/hidrawN/device/report_descriptor` — see
  `crates/sc-hid/examples/feature_probe.rs`) rather than by pressing
  controls — the descriptor declares several other report IDs `sc-adapter`
  never touched before. Report `0x7b` streams continuously and
  independently at its own ~2Hz rate, interleaved with `0x42`; byte 9 is
  the only actively-varying byte in it, and only makes physical sense as a
  **signed** value (two's complement) — the observed range (~190–217
  unsigned) becomes -66..-39 dBm signed, squarely inside plausible
  real-world RSSI. See `sc_protocol::Telemetry::signal_dbm`; shown live in
  the web UI. Unconfirmed: exact calibration/accuracy, and behavior at the
  edge of range.
- **The paired controller's serial number**, via HID **feature report**
  `0x02` (`GET_FEATURE_REPORT`, request/response — distinct from every
  other report on this page, which are all streamed). Necessary over the
  wireless puck specifically because the OS-level HID string descriptor
  for whichever puck interface got opened reports the **puck's own**
  serial, not the paired controller's (confirmed distinct on the unit this
  was tested against). See `SteamControllerDevice::read_controller_serial`;
  shown live in the web UI.

Not yet implemented / unconfirmed (left out rather than guessed):

- Left touchpad click (only the right pad's press-force click was
  isolated; the left may be software/Steam-Input-synthesized rather than
  exposed over raw HID at all).
- **Battery level.** Extensively investigated (2026-07-26) and not found —
  written up here in detail since the negative results themselves took a
  real investigation to establish and are easy to accidentally re-derive
  from scratch otherwise:
  - Report `0x42` bytes 46–53 (past everything else decoded) sit at a
    rock-solid constant `ff 7f 00 00 00 00 00 00` — checked across idle,
    every digital button, both sticks/pads, a full grip squeeze, both
    triggers fully pulled, ~50 minutes of continuous max-amplitude rumble
    on all motors (to accelerate drain), and a full wired charge cycle.
    Never moves. Not a live battery/charge field on any timescale tested.
  - Report `0x42` bytes 30-31 (a 16-bit LE pair) are a "charging active"
    clock: frozen solid everywhere *except* while genuinely plugged in and
    charging, where they tick at a steady ~4ms/step. Useful as a binary
    charging-detector (and confirmed to reflect the physical battery's
    real charging state over *either* connection — wired or wireless —
    not just "is a wired PID open"), but it's a clock, not a percentage.
  - Report `0x7b` byte 9 is `Telemetry::signal_dbm` (RSSI), not battery —
    see "Confirmed" above.
  - Report `0x43` carries a "charging mode" flag (byte 1: `0x01` idle /
    `0x02` charging) plus, only while charging, several more bytes (offsets
    7-12) that light up. One of them (byte 8) is a second static flag
    (exactly `0` idle / `18` charging, confirmed via 334 samples with no
    other value ever seen); the rest cycle through small, bounded,
    quantized-looking value sets with no trend — matches the same
    PWM/ADC-sampling-aliasing character as the bytes-30-31 clock rather
    than a measurement.
  - HID **feature report** `0x01` (`GET_FEATURE_REPORT`, 63 bytes,
    alongside `0x02`'s serial-number payload — see "Confirmed" above)
    stalls (`EPIPE`/"broken pipe") on every query except exactly once,
    immediately after plugging in for charging, where it returned
    `01 8e 00 00...00` — a single non-zero byte among 63. Unreproducible
    across 21 follow-up attempts (including polling immediately after
    fresh connections) despite still being plugged in and charging.
    Real data point, not an actionable one — the timing trigger (if any)
    wasn't identified.
  - Conclusion: no clean/monotonic battery-percentage signal found
    anywhere searched. What *has* been found is multiple independent,
    mutually-redundant "charging is active" flags (bytes 30-31, `0x43`
    byte 1, `0x43` byte 8) and one very short-lived, unreproduced feature-
    report response that might be worth revisiting if it starts firing
    reliably under some as-yet-unidentified condition.
- Whether/how gripping a stick suppresses its co-located touchpad (the
  capacitive sensor above is presumably involved, per the hardware
  owner — not independently verified beyond detecting the sensor itself).

Other units/firmware revisions may differ. If `sc-adapter` doesn't behave
correctly on your hardware, re-derive the layout with the same method:

1. Find your controller's PID: `cargo run -p sc-adapter -- --list`
2. Dump raw reports and isolate one control at a time:
   `cargo run -p sc-adapter -- --pid <PID> --dump-raw`
   (timestamps each line, useful for correlating with when you pressed
   something — for anything that might be a *rate* sensor rather than a
   *position* sensor, like the gyro, do a static hold-still test as well
   as a press/release test, since rate sensors settle back to zero at
   rest regardless of orientation)
3. Update the offsets/masks in `crates/sc-protocol/src/report.rs` (and its
   `ButtonFlags` bit assignments in `lib.rs`) to match. That file is the
   only place wire-format knowledge lives — nothing else should need to
   change.
4. Optionally save a capture and replay it offline without the device
   attached: `cargo run -p sc-adapter -- --replay capture.txt`
5. For output reports (rumble/etc.), read the device's actual HID report
   descriptor first (e.g. `/sys/class/hidraw/hidrawN/device/report_descriptor`
   on Linux) to find valid report IDs/lengths instead of guessing blind —
   see `crates/sc-hid/src/lib.rs`'s `send_rumble_pulse` for the byte
   offsets found this way.
6. Double-check any button captured under a single vague prompt (e.g.
   "right bumper") against what you actually pressed — the right-bumper
   correction above happened because a capacitive sensor produced a
   plausible-looking signal under that exact prompt.

## Layout

- `crates/sc-protocol` — pure decoding logic, no I/O (`report.rs` holds the wire format).
- `crates/sc-hid` — USB HID device discovery/read loop plus output-report writing (rumble), via `hidapi`. `examples/` holds scratch protocol-investigation tools (not part of the crate's public surface) — `feature_probe.rs` probes HID feature reports and sweeps candidate device paths; `battery_drain_monitor.rs` drives continuous rumble while watching for report/byte changes over long unattended runs. See both files' module docs, and the "Protocol status" battery write-up above, for what they found.
- `crates/sc-xinput` — the `VirtualPad` trait, `XInputState`/`XInputButtons` (the fully-mapped, ready-to-send shape), plus the `windows_vigem` and `linux_uinput` backends. Backends are purely mechanical — they don't know about physical buttons at all.
- `crates/sc-config` — decides which physical control drives which XInput output: deadzone/response-curve shaping (`AxisSettings`), the button remap table (`LogicalButton` → `XInputButton`), and `Profile`/`ProfileStore` for saving/loading named configs as TOML.
- `crates/sc-adapter` — the binary: CLI, main loop, logging, and (opt-in) the `web` module serving the local monitoring/config UI.
- `scripts/` — install/setup scripts for Linux and Windows (see "Installation / setup" below).
- `tools/pygame_test.py` — a standalone script for testing the virtual pad via SDL2, independent of this project's own code (see "Running" below).

## Installation / setup

```sh
./scripts/install-linux.sh      # Linux: checks/builds deps, builds, sets up the uinput udev rule + group membership
```
```powershell
.\scripts\install-windows.ps1   # Windows: checks/builds deps, checks/installs ViGEmBus, builds
```

Both are safe to re-run and ask before making any system-wide change
(installing packages, writing the udev rule, group membership, installing
ViGEmBus). See "Building"/"Running" below for what they automate, if you'd
rather do it by hand.

## Building

```sh
cargo build --workspace   # builds the uinput backend on Linux, ViGEm backend on Windows
cargo test --workspace
```

## Running

```sh
./scripts/run.sh          # Linux — auto-detects the controller's PID
.\scripts\run.ps1         # Windows — same, PowerShell
```

These wrap `sc-adapter --list` to find the controller's PID automatically
(scoped to Valve's vendor id, `0x28de`) and launch it, so you don't need to
look up `--pid` by hand — extra flags are forwarded as-is, e.g.
`./scripts/run.sh --web`. If more than one distinct Valve device is attached
they'll refuse to guess and print the candidates so you can pass `--pid`
explicitly instead.

Equivalent manual invocation, useful if you'd rather not go through the
scripts, need `--dump-raw`/`--test-rumble`/`--replay`, or the binary isn't
built via one of the install scripts yet:

```sh
cargo run -p sc-adapter -- --list        # find the PID
cargo run -p sc-adapter -- --pid 0x1302  # then run with it
```

### Wireless (dongle/puck)

The controller's wireless receiver ("Steam Controller Puck") enumerates as
a **different USB product** from the wired connection —
`sc_hid::WIRELESS_PUCK_PID` (`0x1304`) rather than `0x1302` — so pass
`--pid 0x1304` (or just use `scripts/run.sh`/`run.ps1`, which auto-detect
either). If both a wired unit and the puck are plugged in at once, PID
auto-detection becomes ambiguous between two distinct Valve devices and
you'll need to pick one explicitly with `--pid`.

The puck also exposes **multiple HID interfaces under that one PID** — one
per pairing channel, plus a control interface — and only the channel with
an actual paired, awake controller ever produces reports; the others stay
completely silent. Earlier, `sc-adapter` naively opened the first matching
interface regardless, which was frequently the wrong (idle) one — the
symptom was the web UI (and the virtual pad) receiving no live state at all
over the dongle, despite the wired connection working fine. Device opening
(`sc_hid::SteamControllerDevice::open`) now briefly probes each candidate
interface in turn and picks the first one that actually produces a report,
so this is handled automatically; verified end-to-end against real
hardware, including confirming button/pad state streams correctly into the
web UI over the wireless connection. The wire format itself is otherwise
identical to the wired connection — same report ID, same byte layout.

On Linux, `/dev/uinput` access requires either running as root or the udev
rule `scripts/install-linux.sh` installs for you
(`scripts/60-steam-controller-x-uinput.rules`):

```
KERNEL=="uinput", GROUP="input", MODE="0660"
```
(add your user to the `input` group, then reload udev rules and replug/reboot.)

On Windows, install [ViGEmBus](https://github.com/ViGEm/ViGEmBus/releases)
first (`scripts/install-windows.ps1` checks for/offers to install it via
`winget`).

Verified end-to-end on this machine: `sc-adapter` opens the real controller,
creates a `uinput` device named "Microsoft X-Box 360 pad", and the kernel
picks it up with both `eventN` and `jsN` handlers under `/proc/bus/input/devices`.
`tools/pygame_test.py` exercises this the same way a real game would (via
SDL2's joystick backend, which is what most native Linux games and Proton
use) — run `sc-adapter`, then `python3 tools/pygame_test.py` and press
buttons/move sticks to confirm they come through correctly.

That script also documents (and works around) something worth knowing:
SDL has a built-in HIDAPI driver for the original Steam Controller that
tries to read this new controller's raw HID interface directly too,
independent of `sc-adapter` — since it doesn't know this hardware's real
protocol, it shows up as a second, mis-parsed "phantom" gamepad
(~22 buttons) alongside `sc-adapter`'s correct one. Set
`SDL_JOYSTICK_HIDAPI_STEAM=0` in the environment before launching a game
to suppress it. (Any other real Xbox-360-compatible controller connected
at the same time will also look identical to `sc-adapter`'s virtual pad —
same spoofed name and VID/PID — so don't assume it's the only one.)

#### Steam client itself shows a phantom "Steam Controller" and pops its keyboard

Not just games: the Steam client binary bundles the same SDL HIDAPI driver
described above, and opens this controller's raw `hidraw` interface
directly, independent of and in parallel with `sc-adapter`. Confirmed on
real hardware (2026-07-25) via Steam's own **Settings → Controller** page,
which lists two entries for one physical controller: a correct
`"Xbox 360 Controller"` (that's `sc-adapter`'s virtual pad) and a second
`"Steam Controller"` / `"Steam Controller Puck"` entry — Steam misidentifying
this device as the 2015 Steam Controller and misparsing its reports the
same way SDL-based games do.

Symptom: pressing a physical face button (X, in the case that prompted
this) while the Steam client is focused pops open Steam's on-screen
keyboard, because Steam applies a default "Desktop Configuration" to any
device it thinks is a Steam Controller, and that default config binds a
button to "Show Keyboard" — which fires off whatever garbled bit the
misparse happens to land on, not anything `sc-adapter` controls.

Tried and confirmed **not sufficient**:
- Unchecking "Generic Gamepad Configuration Support" in Steam's
  General Controller Settings — doesn't apply, since Steam manages this as
  a first-party Steam Controller, not a generic/Xbox/PlayStation one.
- Launching the Steam client itself with `SDL_JOYSTICK_HIDAPI_STEAM=0` in
  its environment — didn't stop Steam's own controller list from showing
  the phantom "Steam Controller" entry.
- The per-device "Details" page for that phantom entry (Signal Strength,
  Serial Number, Pairing Info, etc.) has no binding editor — and its
  "Forget" button is the actual *wireless RF pairing* between the
  controller and puck (per the on-screen Pairing Info text), not a
  Steam-software-only unlink, so clicking it would drop the wireless link
  itself rather than just stopping Steam from managing the device.

Not yet found: the exact click-path in this Steam client version to edit
that phantom device's Desktop Configuration layout and clear the
"Show Keyboard" binding (likely via holding the Guide button while the
Steam window is focused, or a "Desktop" entry in Big Picture's Library —
neither confirmed yet). Since `hidraw` has no exclusive-open mechanism
(unlike the `evdev` phantom devices below, which `--grab-phantom-inputs`
can lock out), there's no fix possible from inside `sc-adapter` itself —
this is Steam-client-side only.

#### Phantom mouse/keyboard input

Separately from the SDL phantom-gamepad issue above: this controller has no
dedicated kernel HID driver (unlike the original Steam Controller's
`hid-steam`), so Linux falls back to `hid-generic`, which creates a
standard-HID **mouse and keyboard** `evdev` device for every USB interface
the controller exposes (confirmed on real hardware: 4 pairs, one per
wireless puck pairing channel — `"Valve Software Steam Controller Puck
Mouse"` / `"...Keyboard"` in `/proc/bus/input/devices`). Whatever those
usage pages carry goes straight into X11/Wayland/`logind` as real
keyboard/mouse input, completely independent of `sc-adapter` — `hidraw`
(what `sc-adapter` reads) is a parallel, non-exclusive path and doesn't
stop the kernel's normal `evdev` processing of the same device.

Pass `--grab-phantom-inputs` (Linux only) to have `sc-adapter` exclusively
grab (`EVIOCGRAB`) each of those phantom devices for as long as it's
running, so their events stop reaching the rest of the desktop. It
re-grabs on every reconnect, since a replug can hand out different
`/dev/input/eventN` numbers. Requires read/write access to
`/dev/input/eventN`, typically already granted to the active graphical
session the same way `/dev/hidrawN` access is.

To test the haptic motors directly without running the full adapter loop:

```sh
cargo run -p sc-adapter -- --pid 0x1302 --test-rumble a   # or b/c/d
```

`sc-adapter` tolerates the controller being unplugged and replugged while
it's running (a device's HID path/index isn't stable across a reconnect —
Linux may hand out a different `/dev/hidrawN` next time, for instance).
On a lost connection it logs a warning and retries once a second, re-scanning
by vendor/product id each time, until the controller reappears; verified
against real hardware with the adapter running, unplugging the controller
mid-session, and confirming it reconnects cleanly a few seconds after
replugging. The virtual XInput pad stays present throughout — only the
physical controller drops out.

### Web UI

```sh
cargo run -p sc-adapter -- --pid 0x1302 --web
```

Opens a local web server (default `http://127.0.0.1:61302`, override with
`--web-port`/`--web-bind`) alongside the normal adapter loop:

- **Live monitoring**: every button, both sticks, both touchpads, both
  triggers, grip pressure, and raw IMU values, streamed over a WebSocket.
  The header also shows the paired controller's serial number and (over
  the wireless puck) live RF signal strength in dBm — see "Protocol
  status" above for where both come from.
- **Deadzone/curve tuning** for the 4 XInput-mappable axes (left/right
  stick, left/right trigger) — radial deadzone with rescale, plus a
  response-curve exponent. Live-previewed against the running adapter
  immediately; hit "Save" to persist.
- **Button remapping** — bind any of the 24 tracked physical
  controls/sensors to any XInput button, or leave it unmapped. 9 have no
  binding in the default profile: the four back paddles, the "dots"
  button, the right touchpad's touch *and* click flags (RTHUMB now binds
  to the real right-stick click instead — see "Protocol status" above),
  and both sticks' capacitive cap-touch sensors (binding the cap-touch
  sensors to something would cause spurious presses during normal stick
  use, so they're left unmapped rather than given a default).
- **Profiles** — save, switch, and delete named configs (TOML files under
  the platform config dir, e.g. `~/.config/steam-controller-x/profiles/`
  on Linux).
- **Rumble test buttons** for all 4 motors.

It's loopback-bound (`127.0.0.1`) by default deliberately — this controls
real hardware input, so think twice before pointing `--web-bind` anywhere
reachable from the network.

## Known limitations

- Gyro, battery, and grip-squeeze precision (per-side) aren't
  decoded/implemented yet (see "Protocol status" above).
- Left touchpad click isn't confirmed.
- Grip pressure, IMU data, and touchpad *positions* (the X/Y coordinates)
  have no standard Xbox 360 equivalent and no remapping path at all — they
  aren't part of the button table and are never forwarded, regardless of
  profile settings. (Touchpad touch/click *flags* are different: they're
  ordinary remappable buttons, just unmapped in the default profile — see
  "Button remapping" above.)
- The real right *stick* (not the right touchpad) always drives XInput's
  right thumbstick — this isn't a default that can be reconfigured, it's
  hardcoded in `sc-config`'s `Profile::map`. Deadzone/curve settings can
  only be tuned per axis *role* (left stick, right stick, left/right
  trigger), not redirected to a different physical source.
- Rumble output (`sc_hid::SteamControllerDevice::send_rumble_pulse`) is
  wired up and testable via `--test-rumble`/the web UI's rumble buttons,
  but nothing in the `VirtualPad` trait/backends automatically forwards a
  *game's* XInput rumble request into it yet — ViGEm supports this via
  `vigem-client`'s `unstable_xtarget_notification` cargo feature (its
  notification/polling API) and would need genuine force-feedback
  (`EV_FF`) support added on the uinput side, which the `uinput` crate
  currently in use doesn't expose.
