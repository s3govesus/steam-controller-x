//! Scratch diagnostic tool, not part of the crate's public surface.
//!
//! The device's HID report descriptor (read directly from
//! /sys/class/hidraw/hidrawN/device/report_descriptor) declares far more
//! than the report ID 0x42 `sc-protocol` decodes: other input report IDs
//! (0x40, 0x41, 0x43, 0x44, 0x45, 0x79, 0x7b) that normal polling has never
//! examined, plus two Feature reports (0x01, 0x02, 63 bytes each) that are
//! request/response (GET_FEATURE_REPORT) rather than streamed — a classic
//! place for status/config data like battery level to live. This tool (1)
//! requests both feature reports once and prints the raw bytes, then (2)
//! passively listens on the normal input-report stream and logs any report
//! whose ID isn't 0x42, to catch whichever of those other IDs actually
//! fire during real operation.
//!
//! Usage: `cargo run -p sc-hid --example feature_probe -- --pid 0x1304 [--seconds 60]`

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

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// Sweep every candidate hidraw path for `vid`/`pid` and query feature
/// report 0x02 on each directly (bypassing `SteamControllerDevice::open`'s
/// probing, which only picks a path based on which one answers an *input*
/// report first — not necessarily the same one that answers *feature*
/// reports usefully).
fn sweep_paths(pid: u16) {
    let api = hidapi::HidApi::new().expect("HidApi::new failed");
    let start = Instant::now();
    let mut paths: Vec<_> = api
        .device_list()
        .filter(|d| d.vendor_id() == VALVE_VID && d.product_id() == pid)
        .map(|d| d.path().to_owned())
        .collect();
    paths.sort();
    paths.dedup();
    log(start, &format!("found {} distinct candidate path(s)", paths.len()));
    for path in paths {
        let path_str = path.to_string_lossy().into_owned();
        let Ok(dev) = api.open_path(&path) else {
            log(start, &format!("{path_str}: open failed"));
            continue;
        };

        for report_id in [0x01u8, 0x02u8] {
            let mut fbuf = [0u8; 64];
            fbuf[0] = report_id;
            match dev.get_feature_report(&mut fbuf) {
                Ok(n) => log(
                    start,
                    &format!("{path_str}: FEATURE {report_id:#04x} ({n} bytes): {}", to_hex(&fbuf[..n])),
                ),
                Err(err) => log(start, &format!("{path_str}: FEATURE {report_id:#04x} FAILED: {err}")),
            }
        }

        // Listen briefly on this exact path to see which input report
        // id(s) it actually streams — the point being to find out whether
        // 0x42 (gamepad input) and 0x7b/0x43 (telemetry) come from the
        // *same* interface or are split across different ones.
        let mut seen: std::collections::HashSet<u8> = std::collections::HashSet::new();
        let listen_start = Instant::now();
        let mut buf = [0u8; 64];
        while listen_start.elapsed() < Duration::from_secs(2) {
            if let Ok(n) = dev.read_timeout(&mut buf, 200) {
                if n > 0 {
                    seen.insert(buf[0]);
                }
            }
        }
        log(start, &format!("{path_str}: input report ids seen in 2s: {seen:02x?}"));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pid = parse_u16_arg(&args, "--pid", 0x1304);
    let listen_secs = parse_secs_arg(&args, "--seconds", 60);
    let filter = DeviceFilter { vid: VALVE_VID, pid: Some(pid) };

    if args.iter().any(|a| a == "--sweep-paths") {
        sweep_paths(pid);
        return;
    }

    let start = Instant::now();
    let api = hidapi::HidApi::new().expect("HidApi::new failed");
    let device = SteamControllerDevice::open(&api, filter).expect("failed to open device");
    log(start, &format!("device connected, pid={pid:#06x}"));

    for report_id in [0x01u8, 0x02u8] {
        let mut buf = [0u8; 64];
        match device.get_feature_report(report_id, &mut buf) {
            Ok(n) => log(
                start,
                &format!("FEATURE REPORT {report_id:#04x}: {n} bytes: {}", to_hex(&buf[..n])),
            ),
            Err(err) => log(start, &format!("FEATURE REPORT {report_id:#04x} FAILED: {err}")),
        }
    }

    log(start, &format!("listening for {listen_secs}s for any input report id != 0x42..."));
    let listen_start = Instant::now();
    let mut buf = [0u8; 64];
    let mut seen_ids: std::collections::HashSet<u8> = std::collections::HashSet::new();
    while listen_start.elapsed() < Duration::from_secs(listen_secs) {
        match device.read_report(&mut buf, 200) {
            Ok(n) if n > 0 && buf[0] != 0x42 => {
                if seen_ids.insert(buf[0]) {
                    log(
                        start,
                        &format!("NEW REPORT ID {:#04x}: {n} bytes: {}", buf[0], to_hex(&buf[..n])),
                    );
                } else {
                    log(start, &format!("report {:#04x} again: {}", buf[0], to_hex(&buf[..n])));
                }
            }
            Ok(_) => {}
            Err(err) => log(start, &format!("read error: {err}")),
        }
    }
    log(start, &format!("done. distinct non-0x42 ids seen: {seen_ids:02x?}"));
}
