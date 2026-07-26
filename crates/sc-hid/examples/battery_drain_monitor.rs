//! Scratch diagnostic tool, not part of the crate's public surface.
//!
//! Continuously buzzes motors A/B (the two with a real XInput rumble path)
//! to accelerate battery drain on the wireless puck, while watching report
//! bytes 46..54 (currently undecoded, constant `ff 7f 00 00 00 00 00 00` at
//! rest — see README's "Protocol status") for any change as the battery
//! actually depletes. Logs a heartbeat periodically and any byte change or
//! disconnect/reconnect event immediately, with elapsed time.
//!
//! Also watches bytes 30-31 as a little-endian 16-bit counter: confirmed
//! (2026-07-26) to sit completely frozen at a constant value while idle or
//! wireless, but start ticking at a steady ~4ms/step the moment the
//! controller is plugged in via USB — likely a charge-pump/PWM/ADC sample
//! clock exposed in the report. Logs when it freezes/resumes, as a
//! secondary signal for charge-complete (or charging having started).
//!
//! As of 2026-07-26, also tracks two *other* input report IDs the device's
//! HID report descriptor declares but `sc-protocol` never decodes (found by
//! reading /sys/class/hidraw/hidrawN/device/report_descriptor directly —
//! see `feature_probe.rs`), since they arrive interleaved with the normal
//! 0x42 stream and were previously silently discarded by this tool as if
//! they were read timeouts:
//! - `0x7b` (13 bytes on the wire): streams continuously (~2Hz), mostly
//!   stable with a few actively-jittering bytes — shaped like telemetry
//!   (RSSI/battery). Byte 9 jitters the most; tracked with running
//!   min/max and logged on a real drift, not just per-sample noise.
//! - `0x43` (15 bytes on the wire): fires rarely (every several seconds),
//!   almost perfectly static. Logged whenever its content actually changes.
//!
//! Usage: `cargo run -p sc-hid --example battery_drain_monitor -- --pid 0x1304 [--no-rumble]`
//! `--no-rumble` skips driving the motors — use this when watching a wired
//! connection charge rather than trying to drain a wireless one.

use std::io::Write;
use std::time::{Duration, Instant};

use sc_hid::{DeviceFilter, Motor, SteamControllerDevice, VALVE_VID};

fn parse_pid(args: &[String]) -> u16 {
    for w in args.windows(2) {
        if w[0] == "--pid" {
            let s = w[1].trim_start_matches("0x").trim_start_matches("0X");
            return u16::from_str_radix(s, 16).expect("invalid --pid");
        }
    }
    0x1304 // wireless puck default
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn log(start: Instant, msg: &str) {
    let elapsed = start.elapsed();
    println!(
        "[{:>6}.{:03}s] {}",
        elapsed.as_secs(),
        elapsed.subsec_millis(),
        msg
    );
    std::io::stdout().flush().ok();
}

fn open_with_retry(api: &mut hidapi::HidApi, filter: DeviceFilter, start: Instant) -> SteamControllerDevice {
    loop {
        if let Err(err) = api.refresh_devices() {
            log(start, &format!("refresh_devices error: {err}"));
        }
        match SteamControllerDevice::open(api, filter) {
            Ok(dev) => return dev,
            Err(err) => {
                log(start, &format!("open failed ({err}), retrying in 2s"));
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn main() {
    tracing_subscriber::fmt().with_env_filter("sc_hid=debug").init();
    let args: Vec<String> = std::env::args().collect();
    let pid = parse_pid(&args);
    let no_rumble = has_flag(&args, "--no-rumble");
    let filter = DeviceFilter { vid: VALVE_VID, pid: Some(pid) };

    let start = Instant::now();
    log(
        start,
        &format!("battery drain monitor starting, pid={pid:#06x}, rumble={}", !no_rumble),
    );

    let mut api = hidapi::HidApi::new().expect("HidApi::new failed");
    let mut device = open_with_retry(&mut api, filter, start);
    log(start, "device connected");

    let baseline_tail: [u8; 8] = [0xff, 0x7f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let mut last_tail = baseline_tail;

    let mut motor_toggle = false;
    let mut iterations: u64 = 0;
    let mut consecutive_errors = 0u32;
    // Separate from `consecutive_errors`: the wireless puck's hidraw
    // interface stays open (writes still return `Ok`, reads just time out)
    // even while the controller itself is asleep/RF-disconnected — no
    // hidapi error is ever raised for that. Tracked independently so a
    // real idle-out/wake is visible in the log instead of silently
    // read-timing-out forever.
    let mut consecutive_timeouts = 0u32;
    let mut quiet_since: Option<Instant> = None;
    let mut last_heartbeat = Instant::now();
    let mut buf = [0u8; 64];
    // Report byte 33 behaves like a ~1s-tick connection-uptime counter
    // (climbs steadily through a session, resets low on a fresh
    // connect — confirmed by comparing captures across a wired-to-wireless
    // switch). A backward jump mid-session is a second, independent signal
    // for a reconnect/idle-out, in case a sleep cycle ever doesn't show up
    // as read timeouts.
    let mut last_uptime_byte: Option<u8> = None;
    // Bytes 30-31 as a little-endian 16-bit value: frozen solid everywhere
    // except while actively plugged in/charging (see module docs). Tracked
    // as "ticking" vs "frozen" (a handful of consecutive equal readings in
    // a row, since occasional duplicate reads happen even while it's
    // actively counting) rather than a raw last-value comparison.
    let mut last_charge_counter: Option<u16> = None;
    let mut consecutive_frozen_reads = 0u32;
    let mut charge_counter_active = false;
    // Report 0x7b, byte 9 (0-indexed within the 13-byte wire report,
    // i.e. buf[9]): the most actively-varying byte in that report,
    // shaped like telemetry (RSSI/battery) rather than a counter. Tracked
    // as a running min/max since start, plus a last-*logged* value so we
    // only log on a real drift rather than every ~1-count jitter.
    let mut report_7b_seen = 0u64;
    let mut byte9_min: Option<u8> = None;
    let mut byte9_max: Option<u8> = None;
    let mut byte9_last_logged: Option<u8> = None;
    const BYTE9_DRIFT_THRESHOLD: i16 = 15;
    // Report 0x43: rare, almost-static. Just log on any actual change.
    let mut last_43: Option<[u8; 15]> = None;

    let mut last_rumble = Instant::now() - Duration::from_secs(1);

    loop {
        // Rumble is throttled to ~150ms regardless of read rate, but reads
        // themselves run in a tight loop (no sleep) — 0x42 streams at
        // ~250Hz while 0x7b/0x43 are much rarer (~2Hz / every few
        // seconds), and this tool's original 150ms-per-iteration sleep
        // undersampled badly enough that it reliably starved out the
        // rarer report ids entirely (confirmed: a tight read loop caught
        // them every time; this sleep-gated one never did in several
        // 8-15s runs). `read_report`'s own 100ms timeout already paces
        // CPU use when the device is quiet.
        if !no_rumble && last_rumble.elapsed() >= Duration::from_millis(150) {
            let motor = if motor_toggle { Motor::B } else { Motor::A };
            motor_toggle = !motor_toggle;
            last_rumble = Instant::now();
            match device.send_rumble_pulse(motor, 0xff) {
                Ok(_) => consecutive_errors = 0,
                Err(err) => {
                    consecutive_errors += 1;
                    log(start, &format!("rumble write error: {err} (consecutive={consecutive_errors})"));
                }
            }
        }

        match device.read_report(&mut buf, 100) {
            Ok(0) => {
                // Real timeout — not a hidapi error, but on the wireless
                // puck this is exactly what a sleeping/RF-disconnected
                // controller looks like.
                consecutive_timeouts += 1;
                if quiet_since.is_none() && consecutive_timeouts >= 3 {
                    quiet_since = Some(Instant::now());
                    log(start, "REPORT STREAM WENT QUIET (possible idle-out/sleep) — rumble writes will keep going but likely aren't reaching the controller");
                }
            }
            Ok(n) => {
                // Any successful read of any report id means the
                // connection is alive — previously only an 0x42 report
                // reset these, so an interleaved 0x7b/0x43 report was
                // silently treated as if it were a timeout.
                consecutive_errors = 0;
                consecutive_timeouts = 0;
                if let Some(since) = quiet_since.take() {
                    log(
                        start,
                        &format!(
                            "REPORT STREAM RESUMED after {:.1}s quiet (idle-out/wake?)",
                            since.elapsed().as_secs_f64()
                        ),
                    );
                }

                match buf[0] {
                    0x42 if n >= 54 => {
                        let tail: [u8; 8] = buf[46..54].try_into().unwrap();
                        if tail != last_tail {
                            log(
                                start,
                                &format!(
                                    "TAIL BYTES CHANGED: {:02x?} -> {:02x?} (baseline was {:02x?})",
                                    last_tail, tail, baseline_tail
                                ),
                            );
                            last_tail = tail;
                        }

                        let uptime = buf[33];
                        if let Some(prev) = last_uptime_byte {
                            // Single byte, wraps at 256 — a drop from near
                            // the top back to near zero is an expected
                            // wraparound (ticks roughly once/sec, so this
                            // recurs every ~4.3min), not a reset. Only flag
                            // drops that aren't explained by that.
                            let is_wraparound = prev >= 250 && uptime <= 5;
                            if uptime < prev && !is_wraparound {
                                log(
                                    start,
                                    &format!(
                                        "UPTIME COUNTER RESET (byte 33): {prev} -> {uptime} — reconnect/idle-out likely happened here"
                                    ),
                                );
                            }
                        }
                        last_uptime_byte = Some(uptime);

                        let charge_counter = (buf[31] as u16) * 256 + buf[30] as u16;
                        if let Some(prev) = last_charge_counter {
                            if charge_counter == prev {
                                consecutive_frozen_reads += 1;
                                if charge_counter_active && consecutive_frozen_reads >= 5 {
                                    charge_counter_active = false;
                                    log(
                                        start,
                                        &format!(
                                            "CHARGE COUNTER FROZE at {charge_counter} (possible charge complete / unplugged)"
                                        ),
                                    );
                                }
                            } else {
                                consecutive_frozen_reads = 0;
                                if !charge_counter_active {
                                    charge_counter_active = true;
                                    log(
                                        start,
                                        &format!(
                                            "CHARGE COUNTER STARTED TICKING at {prev} -> {charge_counter} (charging active?)"
                                        ),
                                    );
                                }
                            }
                        }
                        last_charge_counter = Some(charge_counter);
                    }
                    0x7b if n >= 13 => {
                        report_7b_seen += 1;
                        let b9 = buf[9];
                        byte9_min = Some(byte9_min.map_or(b9, |m| m.min(b9)));
                        byte9_max = Some(byte9_max.map_or(b9, |m| m.max(b9)));
                        let drifted = match byte9_last_logged {
                            None => true,
                            Some(prev) => (b9 as i16 - prev as i16).abs() >= BYTE9_DRIFT_THRESHOLD,
                        };
                        if drifted {
                            log(
                                start,
                                &format!(
                                    "REPORT 0x7b byte9 DRIFT: {:?} -> {b9} (full: {})",
                                    byte9_last_logged,
                                    to_hex(&buf[..n]),
                                ),
                            );
                            byte9_last_logged = Some(b9);
                        }
                    }
                    0x43 if n >= 15 => {
                        let content: [u8; 15] = buf[..15].try_into().unwrap();
                        if last_43 != Some(content) {
                            log(start, &format!("REPORT 0x43 CHANGED: {}", to_hex(&content)));
                            last_43 = Some(content);
                        }
                    }
                    _ => {} // some other/short report — connection is alive, nothing more to track
                }
            }
            Err(err) => {
                consecutive_errors += 1;
                log(start, &format!("read error: {err} (consecutive={consecutive_errors})"));
            }
        }

        if consecutive_errors >= 3 {
            log(start, "DEVICE LOST — attempting reconnect");
            device = open_with_retry(&mut api, filter, start);
            log(start, "DEVICE RECONNECTED");
            consecutive_errors = 0;
            consecutive_timeouts = 0;
            quiet_since = None;
            last_uptime_byte = None; // a fresh device handle resets the counter too
            last_charge_counter = None;
            consecutive_frozen_reads = 0;
            byte9_last_logged = None; // force a fresh baseline log after reconnect
        }

        iterations += 1;
        if last_heartbeat.elapsed() >= Duration::from_secs(60) {
            log(
                start,
                &format!(
                    "heartbeat: {iterations} iterations, tail={:02x?}, uptime_byte={:?}, charge_counter={:?} (ticking={charge_counter_active}), tail_unchanged={}, 0x7b_seen={report_7b_seen} byte9=[{:?}..{:?}] last={:?}, 0x43_last={:02x?}",
                    last_tail,
                    last_uptime_byte,
                    last_charge_counter,
                    last_tail == baseline_tail,
                    byte9_min,
                    byte9_max,
                    byte9_last_logged,
                    last_43,
                ),
            );
            last_heartbeat = Instant::now();
        }
    }
}
