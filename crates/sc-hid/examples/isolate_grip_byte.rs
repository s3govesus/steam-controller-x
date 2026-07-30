//! Scratch diagnostic tool, not part of the crate's public surface.
//!
//! Follow-up to `grip_probe.rs`: that tool confirmed report byte 32 (the
//! one `sc-protocol` used to call "grip") is actually a free-running ~14Hz
//! counter, unrelated to squeezing anything — see `sc-protocol/report.rs`'s
//! module doc for the writeup. So the real grip-squeeze pressure byte (if
//! this report exposes one at all) is still unknown.
//!
//! This tool isolates it properly, the same way the rest of this file's
//! neighbors were found (see the module doc in `sc-protocol/report.rs`):
//! record an idle baseline, record a sustained squeeze, record release, then
//! diff. A byte that shifts to a new stable range during the squeeze phase
//! and returns to its baseline range after release is a real candidate —
//! unlike a one-shot before/after snapshot, this can't be fooled by a byte
//! that's simply always drifting on its own (like byte 32 or the confirmed
//! byte-33 uptime counter — both are called out explicitly in the report so
//! they don't get mistaken for a hit again).
//!
//! This needs an actual human hand on the grips at the right time, so run it
//! interactively and follow the on-screen prompts — squeeze BOTH grips
//! firmly for the whole "SQUEEZE NOW" phase, then let go for "RELEASE" and
//! stay off the controller entirely for "BASELINE".
//!
//! Usage: `cargo run -p sc-hid --example isolate_grip_byte -- --pid 0x1304`
//! (`--baseline-secs`/`--squeeze-secs`/`--release-secs` to adjust phase
//! lengths, default 6/10/6).

use std::io::Write;
use std::time::{Duration, Instant};

use sc_hid::{DeviceFilter, SteamControllerDevice, VALVE_VID};

const REPORT_LEN: usize = 46; // sc_protocol::report::MIN_REPORT_LEN

// Bytes already independently identified elsewhere, so a hit here isn't
// news — called out in the summary instead of presented as a discovery.
const KNOWN_BYTES: &[(usize, &str)] = &[
    (1, "sequence counter, wraps every 256 reports"),
    (32, "free-running ~14Hz counter (see grip_probe.rs) — NOT grip pressure"),
    (33, "confirmed ~1Hz connection-uptime counter (see battery_drain_monitor.rs)"),
];

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

/// Collect `[u8; REPORT_LEN]` snapshots of every 0x42 report seen for
/// `duration`, printing a countdown so whoever's holding the controller
/// knows what phase they're in.
fn collect_phase(device: &SteamControllerDevice, start: Instant, phase_name: &str, duration: Duration) -> Vec<[u8; REPORT_LEN]> {
    log(start, &format!("=== {phase_name} starting ({:.0}s) ===", duration.as_secs_f32()));
    let mut samples = Vec::new();
    let mut buf = [0u8; 64];
    let phase_start = Instant::now();
    let mut next_countdown = duration.as_secs();
    while phase_start.elapsed() < duration {
        let remaining = duration.saturating_sub(phase_start.elapsed()).as_secs();
        if remaining < next_countdown {
            log(start, &format!("{phase_name}: {remaining}s remaining..."));
            next_countdown = remaining;
        }
        match device.read_report(&mut buf, 100) {
            Ok(n) if n >= REPORT_LEN && buf[0] == 0x42 => {
                let mut snap = [0u8; REPORT_LEN];
                snap.copy_from_slice(&buf[..REPORT_LEN]);
                samples.push(snap);
            }
            _ => {}
        }
    }
    log(start, &format!("=== {phase_name} done: {} samples ===", samples.len()));
    samples
}

/// Drop the first/last `trim` samples of a phase — transition edges (the
/// moment a squeeze starts landing, or a release finishes) are exactly
/// where a fast-settling sensor would look like noise rather than a clean
/// step change.
fn trimmed(samples: &[[u8; REPORT_LEN]], trim: usize) -> &[[u8; REPORT_LEN]] {
    if samples.len() <= trim * 2 {
        return samples;
    }
    &samples[trim..samples.len() - trim]
}

fn byte_range(samples: &[[u8; REPORT_LEN]], idx: usize) -> (u8, u8) {
    let mut min = u8::MAX;
    let mut max = 0u8;
    for s in samples {
        min = min.min(s[idx]);
        max = max.max(s[idx]);
    }
    (min, max)
}

fn ranges_overlap(a: (u8, u8), b: (u8, u8)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pid = parse_u16_arg(&args, "--pid", 0x1304);
    let baseline_secs = parse_secs_arg(&args, "--baseline-secs", 6);
    let squeeze_secs = parse_secs_arg(&args, "--squeeze-secs", 10);
    let release_secs = parse_secs_arg(&args, "--release-secs", 6);
    let filter = DeviceFilter { vid: VALVE_VID, pid: Some(pid) };

    let start = Instant::now();
    println!("============================================================");
    println!("GRIP BYTE ISOLATION PROBE");
    println!("Phases: BASELINE (hands off) -> SQUEEZE (grip both handles");
    println!("firmly and hold) -> RELEASE (hands off again).");
    println!("Follow the countdown prompts as they print.");
    println!("============================================================");

    let api = hidapi::HidApi::new().expect("HidApi::new failed");
    let device = SteamControllerDevice::open(&api, filter).expect("failed to open device");
    log(start, "device connected");

    let baseline1 = collect_phase(&device, start, "BASELINE (hands off)", Duration::from_secs(baseline_secs));
    let squeeze = collect_phase(&device, start, "SQUEEZE NOW (hold both grips firmly)", Duration::from_secs(squeeze_secs));
    let baseline2 = collect_phase(&device, start, "RELEASE (hands off again)", Duration::from_secs(release_secs));

    if baseline1.is_empty() || squeeze.is_empty() || baseline2.is_empty() {
        log(start, "one or more phases collected zero samples — device likely went quiet/idle mid-run, results below are unreliable");
    }

    // Trim ~300ms of edge samples per phase (reports stream at ~250Hz on
    // this device, so ~75 samples), same idea as the transition-noise
    // handling described in battery_drain_monitor.rs's charge-counter logic.
    let b1 = trimmed(&baseline1, 75);
    let sq = trimmed(&squeeze, 75);
    let b2 = trimmed(&baseline2, 75);

    log(start, "=== ANALYSIS ===");
    log(start, "byte | baseline1 range | squeeze range | baseline2 range | verdict");
    let mut candidates = Vec::new();
    for idx in 0..REPORT_LEN {
        let r1 = byte_range(b1, idx);
        let rs = byte_range(sq, idx);
        let r2 = byte_range(b2, idx);

        let known = KNOWN_BYTES.iter().find(|(i, _)| *i == idx).map(|(_, desc)| *desc);

        let unchanged = r1 == rs && rs == r2;
        if unchanged {
            continue; // silent — nothing happened here at all
        }

        let squeeze_shifted = !ranges_overlap(r1, rs);
        let returned = ranges_overlap(r1, r2);
        let is_real_candidate = squeeze_shifted && returned && known.is_none();

        let verdict = if let Some(desc) = known {
            format!("already identified: {desc}")
        } else if is_real_candidate {
            "*** GRIP CANDIDATE: shifted during squeeze, returned after release ***".to_string()
        } else if squeeze_shifted && !returned {
            "shifted during squeeze but didn't return — could be a slow drift, not a clean squeeze signal".to_string()
        } else {
            "changed but not squeeze-shaped (didn't hold a distinct range during squeeze) — likely just noise/independent activity".to_string()
        };

        if is_real_candidate {
            candidates.push(idx);
        }

        log(
            start,
            &format!("byte[{idx:>2}]: base1={r1:?} squeeze={rs:?} base2={r2:?} -> {verdict}"),
        );
    }

    log(start, "=== SUMMARY ===");
    if candidates.is_empty() {
        log(start, "no byte showed a clean squeeze-and-release pattern — the sensor may not be in this report, may need a firmer/longer squeeze, or may need more samples (try longer --squeeze-secs)");
    } else {
        log(start, &format!("candidate byte(s) for real grip-squeeze pressure: {candidates:?}"));
    }
}
