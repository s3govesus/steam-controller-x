//! Scratch diagnostic tool, not part of the crate's public surface.
//!
//! Investigates a report from the field: the web UI's "Grip" meter
//! (driven by `offset::GRIP` = report byte 32, see `sc-protocol`'s
//! `report.rs`) climbs from 0% to 100%, holds there for several seconds,
//! then drops back to 0% and climbs again — repeating in a loop even when
//! nobody is touching the grips.
//!
//! That pattern (steady climb, plateau, hard reset) is exactly what a
//! free-running `u8` counter looks like once it's pushed through the UI's
//! `min(100, raw / 1.6)` clamp: values above 160 all display as 100%
//! (the "plateau" is the clamp hiding the climb from 160 to 255), then the
//! byte wraps from 255 to 0 (the "reset"), and the cycle repeats. This is
//! suspicious because `battery_drain_monitor.rs` already confirmed the
//! *adjacent* byte 33 is exactly this kind of thing: a ~1s-tick
//! connection-uptime counter that wraps every ~256 seconds. Byte 32 sitting
//! right next to it and behaving the same way would mean it isn't grip
//! pressure at all (or at least that something else is aliased onto it).
//!
//! This tool logs byte 32 (GRIP) and byte 33 (confirmed uptime counter, for
//! comparison) side by side: tick rate, min/max, and wraparound events. Run
//! it while deliberately NOT touching the grips at all — if byte 32 ticks
//! on its own at a steady rate and wraps like byte 33 does, that confirms
//! it's not (purely) a squeeze-pressure reading.
//!
//! Usage: `cargo run -p sc-hid --example grip_probe -- --pid 0x1304 [--seconds 300]`

use std::io::Write;
use std::time::{Duration, Instant};

use sc_hid::{DeviceFilter, SteamControllerDevice, VALVE_VID};

fn parse_u16_arg(args: &[String], flag: &str, default: u16) -> u16 {
    for w in args.windows(2) {
        if w[0] == flag {
            let s = w[1].trim_start_matches("0x").trim_start_matches("0X");
            return u16::from_str_radix(s, 16).unwrap_or(default);
        }
    }
    default
}

fn parse_secs_arg(args: &[String], flag: &str, default: u64) -> u64 {
    for w in args.windows(2) {
        if w[0] == flag {
            return w[1].parse().unwrap_or(default);
        }
    }
    default
}

fn log(start: Instant, msg: &str) {
    let elapsed = start.elapsed();
    println!("[{:>6}.{:03}s] {}", elapsed.as_secs(), elapsed.subsec_millis(), msg);
    std::io::stdout().flush().ok();
}

struct ByteTracker {
    name: &'static str,
    last: Option<u8>,
    min: u8,
    max: u8,
    ticks: u64,
    last_tick_at: Option<Instant>,
    tick_gaps: Vec<Duration>,
}

impl ByteTracker {
    fn new(name: &'static str) -> Self {
        Self { name, last: None, min: u8::MAX, max: 0, ticks: 0, last_tick_at: None, tick_gaps: Vec::new() }
    }

    fn observe(&mut self, start: Instant, value: u8) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);

        match self.last {
            None => {
                log(start, &format!("{} baseline: {value}", self.name));
            }
            Some(prev) if prev != value => {
                self.ticks += 1;
                let now = Instant::now();
                if let Some(last_tick) = self.last_tick_at {
                    self.tick_gaps.push(now.duration_since(last_tick));
                }
                self.last_tick_at = Some(now);

                // A single u8 wraps at 256 — a big downward jump from near
                // the top back to near zero is an expected wraparound, not
                // a real reset, and is the whole thing under investigation
                // here rather than an error case to hide.
                let is_wraparound = prev >= 250 && value <= 5;
                if is_wraparound {
                    log(start, &format!("{} WRAPAROUND: {prev} -> {value} (tick #{})", self.name, self.ticks));
                } else if value < prev {
                    log(start, &format!("{} DROPPED (non-wraparound): {prev} -> {value}", self.name));
                } else {
                    log(start, &format!("{}: {prev} -> {value}", self.name));
                }
            }
            Some(_) => {}
        }
        self.last = Some(value);
    }

    fn heartbeat_summary(&self) -> String {
        let avg_gap_ms = if self.tick_gaps.is_empty() {
            0.0
        } else {
            self.tick_gaps.iter().map(Duration::as_secs_f64).sum::<f64>() / self.tick_gaps.len() as f64 * 1000.0
        };
        format!(
            "{}: last={:?} min={} max={} ticks={} avg_tick_gap={avg_gap_ms:.0}ms",
            self.name, self.last, self.min, self.max, self.ticks
        )
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pid = parse_u16_arg(&args, "--pid", 0x1304);
    let run_secs = parse_secs_arg(&args, "--seconds", 300);
    let filter = DeviceFilter { vid: VALVE_VID, pid: Some(pid) };

    let start = Instant::now();
    println!("============================================================");
    println!("GRIP PROBE — leave the controller UNTOUCHED (don't squeeze the");
    println!("grips) for the whole run, so any change in byte 32 can only be");
    println!("coming from the device itself, not your hands.");
    println!("Running for {run_secs}s against pid {pid:#06x}...");
    println!("============================================================");

    let api = hidapi::HidApi::new().expect("HidApi::new failed");
    let device = SteamControllerDevice::open(&api, filter).expect("failed to open device");
    log(start, "device connected");

    let mut grip = ByteTracker::new("byte32(grip)");
    let mut uptime = ByteTracker::new("byte33(uptime, confirmed)");

    let mut buf = [0u8; 64];
    let mut last_heartbeat = Instant::now();

    while start.elapsed() < Duration::from_secs(run_secs) {
        match device.read_report(&mut buf, 200) {
            Ok(n) if n >= 34 && buf[0] == 0x42 => {
                grip.observe(start, buf[32]);
                uptime.observe(start, buf[33]);
            }
            Ok(_) => {}
            Err(err) => log(start, &format!("read error: {err}")),
        }

        if last_heartbeat.elapsed() >= Duration::from_secs(10) {
            log(start, &format!("heartbeat: {} | {}", grip.heartbeat_summary(), uptime.heartbeat_summary()));
            last_heartbeat = Instant::now();
        }
    }

    log(start, "probe finished.");
    log(start, &format!("FINAL: {}", grip.heartbeat_summary()));
    log(start, &format!("FINAL: {}", uptime.heartbeat_summary()));
}
