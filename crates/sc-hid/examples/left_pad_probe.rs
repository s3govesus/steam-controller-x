use std::time::{Duration, Instant};
use sc_hid::{DeviceFilter, SteamControllerDevice, VALVE_VID};

fn main() {
    let api = hidapi::HidApi::new().expect("HidApi::new failed");
    let filter = DeviceFilter { vid: VALVE_VID, pid: Some(0x1304) };
    let device = SteamControllerDevice::open(&api, filter).expect("Failed to open controller");

    println!("============================================================");
    println!("PROBE STARTED: Please PRESS & CLICK the LEFT TOUCHPAD now!");
    println!("Logging byte diffs against baseline for 30 seconds...");
    println!("============================================================");

    let start = Instant::now();
    let mut prev = [0u8; 64];
    let mut buf = [0u8; 64];
    let mut report_count = 0;

    while start.elapsed() < Duration::from_secs(30) {
        match device.read_report(&mut buf, 200) {
            Ok(n) if n > 0 => {
                report_count += 1;
                if report_count == 1 {
                    prev[..n].copy_from_slice(&buf[..n]);
                    println!("Baseline report (len {}): {:02x?}", n, &buf[..n]);
                    continue;
                }

                let mut diffs = Vec::new();
                for i in 0..n {
                    if buf[i] != prev[i] {
                        diffs.push(format!("byte[{i}]: {:#04x} -> {:#04x} ({:#010b})", prev[i], buf[i], buf[i]));
                    }
                }

                if !diffs.is_empty() {
                    let elapsed = start.elapsed().as_secs_f32();
                    println!("[{:>6.3}s] Report {:#04x} diffs: {}", elapsed, buf[0], diffs.join(" | "));
                    prev[..n].copy_from_slice(&buf[..n]);
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("Read error: {e}"),
        }
    }

    println!("Probe finished.");
}
