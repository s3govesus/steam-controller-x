mod input_grab;
mod web;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, RwLock};

use clap::Parser;
use sc_config::ProfileStore;
use sc_hid::{DeviceFilter, SteamControllerDevice};
use sc_xinput::VirtualPad;

/// XInput adapter for the new Steam Controller.
///
/// Reads raw USB HID reports from the controller and drives a virtual
/// XInput-compatible gamepad (ViGEmBus on Windows, uinput on Linux).
#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    /// USB vendor id to match (accepts 0x-prefixed hex or decimal).
    #[arg(long, value_parser = parse_u16, default_value = "0x28de")]
    vid: u16,

    /// USB product id to match. Omit to match any device under `vid` —
    /// useful with `--list` to discover the real value for your controller.
    #[arg(long, value_parser = parse_u16)]
    pid: Option<u16>,

    /// List connected HID devices matching `--vid` and exit.
    #[arg(long)]
    list: bool,

    /// Print raw input report bytes instead of driving a virtual pad.
    /// Use this to reverse-engineer the real report layout: press one
    /// button/move one axis at a time and watch which bytes change.
    #[arg(long)]
    dump_raw: bool,

    /// Replay a captured report dump (one whitespace-separated hex report
    /// per line, '#' comments allowed) through the decoder instead of
    /// opening a real device.
    #[arg(long)]
    replay: Option<PathBuf>,

    /// Trigger a single haptic pulse on the given motor slot (a/b/c/d) and
    /// exit, instead of running the adapter loop. Exercises the
    /// best-effort rumble encoding — see `sc_hid::Motor` for confidence
    /// levels on each slot.
    #[arg(long, value_name = "a|b|c|d")]
    test_rumble: Option<String>,

    /// Serve a local web UI for live monitoring and profile/remap
    /// configuration alongside the normal adapter loop.
    #[arg(long)]
    web: bool,

    /// Address the web UI binds to. Loopback-only by default — this
    /// controls real hardware input, so think twice before binding it
    /// anywhere reachable from the network.
    #[arg(long, default_value = "127.0.0.1")]
    web_bind: String,

    /// Port the web UI listens on. Defaults to a port in the IANA
    /// dynamic/private range (never assigned to a registered service,
    /// unlike 8080/"http-alt") to avoid clashing with other local dev
    /// servers/proxies.
    #[arg(long, default_value_t = 61302)]
    web_port: u16,

    /// Increase log verbosity (-v, -vv).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Exclusively grab the kernel's phantom "Mouse"/"Keyboard" evdev
    /// devices this controller's HID interfaces cause hid-generic to
    /// create (no dedicated kernel driver exists for this hardware), so
    /// their button/motion reports stop leaking into the desktop
    /// environment as real keyboard/mouse input while this adapter runs.
    /// See `input_grab` module docs. Linux only; a no-op elsewhere.
    #[arg(long)]
    grab_phantom_inputs: bool,
}

fn parse_u16(s: &str) -> Result<u16, String> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16).map_err(|e| e.to_string()),
        None => s.parse::<u16>().map_err(|e| e.to_string()),
    }
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = format!(
        "sc_adapter={level},sc_hid={level},sc_xinput={level},sc_config={level}"
    );
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    if let Some(path) = &cli.replay {
        return replay(path);
    }

    let mut api = hidapi::HidApi::new()?;

    if cli.list {
        let devices = sc_hid::list_devices(&api, cli.vid);
        if devices.is_empty() {
            println!("No HID devices found with vendor id {:#06x}", cli.vid);
        }
        for d in devices {
            println!(
                "vid={:#06x} pid={:#06x} product={:?} serial={:?} path={}",
                d.vid, d.pid, d.product, d.serial, d.path
            );
        }
        return Ok(());
    }

    let filter = DeviceFilter {
        vid: cli.vid,
        pid: cli.pid.or(Some(sc_hid::DEFAULT_PID)),
    };

    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        ctrlc::set_handler(move || running.store(false, Ordering::SeqCst))?;
    }

    if let Some(motor) = &cli.test_rumble {
        let motor = match motor.to_ascii_lowercase().as_str() {
            "a" => sc_hid::Motor::A,
            "b" => sc_hid::Motor::B,
            "c" => sc_hid::Motor::C,
            "d" => sc_hid::Motor::D,
            other => anyhow::bail!("--test-rumble expects a/b/c/d, got {other:?}"),
        };
        // One-shot diagnostic command: fail fast rather than retry, since an
        // instant "not found" is more useful here than waiting around.
        let device = SteamControllerDevice::open(&api, filter)?;
        device.send_rumble_pulse(motor, 0xff)?;
        println!("sent rumble pulse to motor {motor:?}");
        return Ok(());
    }

    let mut buf = [0u8; 64];

    if cli.dump_raw {
        let start = std::time::Instant::now();
        while running.load(Ordering::SeqCst) {
            let Some(device) = open_with_retry(&mut api, filter, &running) else {
                break;
            };
            tracing::info!("controller connected");
            while running.load(Ordering::SeqCst) {
                match device.read_report(&mut buf, 200) {
                    Ok(n) if n > 0 => {
                        println!("{:>8.3} {}", start.elapsed().as_secs_f64(), to_hex(&buf[..n]));
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(%err, "lost connection to controller, reconnecting");
                        break;
                    }
                }
            }
        }
        return Ok(());
    }

    let store = Arc::new(ProfileStore::new()?);
    let profile = Arc::new(RwLock::new(store.load_active_or_default()));
    let (rumble_tx, rumble_rx) = mpsc::channel::<(sc_hid::Motor, u8)>();
    let (live_tx, live_rx) = tokio::sync::watch::channel(web::LiveState::default());

    if cli.web {
        let bind: SocketAddr = format!("{}:{}", cli.web_bind, cli.web_port)
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid --web-bind/--web-port: {e}"))?;
        let state = web::AppState {
            live: live_rx,
            profile: profile.clone(),
            store: store.clone(),
            rumble_tx: rumble_tx.clone(),
        };
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(err) => {
                    tracing::error!(%err, "failed to start web UI runtime");
                    return;
                }
            };
            if let Err(err) = rt.block_on(web::run(bind, state)) {
                tracing::error!(%err, "web UI server exited with an error");
            }
        });
        println!("Web UI: http://{bind}");
    }

    let mut pad = make_pad()?;
    while running.load(Ordering::SeqCst) {
        let Some(device) = open_with_retry(&mut api, filter, &running) else {
            break;
        };
        tracing::info!("controller connected");

        // Re-grab on every (re)connect, not just once at startup: a
        // replug can hand out different `/dev/input/eventN` numbers, same
        // as the `/dev/hidrawN` instability `open_with_retry` already
        // works around.
        let _input_grab = if cli.grab_phantom_inputs {
            match input_grab::grab(filter.vid, filter.pid.unwrap_or(sc_hid::DEFAULT_PID)) {
                Ok(grabbed) => Some(grabbed),
                Err(err) => {
                    tracing::warn!(%err, "failed to grab phantom input devices");
                    None
                }
            }
        } else {
            None
        };

        while running.load(Ordering::SeqCst) {
            if let Ok((motor, amplitude)) = rumble_rx.try_recv() {
                if let Err(err) = device.send_rumble_pulse(motor, amplitude) {
                    tracing::warn!(%err, "failed to send rumble pulse");
                }
            }

            match device.read_report(&mut buf, 200) {
                Ok(0) => continue,
                Ok(n) => match sc_protocol::decode(&buf[..n]) {
                    Ok(state) => {
                        let mapped = { profile.read().unwrap().map(&state) };
                        if let Err(err) = pad.update(&mapped) {
                            tracing::warn!(%err, "failed to update virtual pad");
                        }
                        let _ = live_tx.send(web::LiveState {
                            raw: Some(state),
                            mapped: Some(mapped),
                        });
                    }
                    Err(err) => tracing::debug!(%err, "failed to decode report"),
                },
                Err(err) => {
                    tracing::warn!(%err, "lost connection to controller, reconnecting");
                    break;
                }
            }
        }
    }

    tracing::info!("shutting down");
    Ok(())
}

/// Repeatedly tries to open the controller (e.g. across a plug-in-later
/// startup, or an unplug/replug mid-session), waiting 1s between attempts.
/// Device paths/indices can change across a reconnect, so this always
/// re-searches by VID/PID via [`SteamControllerDevice::open`] rather than
/// reusing anything from the previous connection — and, since
/// `HidApi::device_list` reflects a snapshot rather than live state, this
/// explicitly re-enumerates via `refresh_devices` before each attempt, or a
/// replugged controller would never be found again. Returns `None` only if
/// `running` was cleared (e.g. Ctrl-C) while waiting.
fn open_with_retry(
    api: &mut hidapi::HidApi,
    filter: DeviceFilter,
    running: &AtomicBool,
) -> Option<SteamControllerDevice> {
    loop {
        if !running.load(Ordering::SeqCst) {
            return None;
        }
        if let Err(err) = api.refresh_devices() {
            tracing::debug!(%err, "failed to refresh HID device list");
        }
        match SteamControllerDevice::open(api, filter) {
            Ok(device) => return Some(device),
            Err(err) => {
                tracing::warn!(%err, "controller not found, retrying in 1s");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
}

fn replay(path: &std::path::Path) -> anyhow::Result<()> {
    let contents = std::fs::read_to_string(path)?;
    for (i, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Tolerate a leading `--dump-raw` timestamp column (contains a '.').
        let mut tokens = line.split_whitespace().peekable();
        if tokens.peek().is_some_and(|t| t.contains('.')) {
            tokens.next();
        }
        let bytes: Result<Vec<u8>, _> = tokens
            .map(|tok| u8::from_str_radix(tok.trim_start_matches("0x"), 16))
            .collect();
        let bytes = match bytes {
            Ok(b) => b,
            Err(err) => {
                eprintln!("line {}: invalid hex ({err})", i + 1);
                continue;
            }
        };
        match sc_protocol::decode(&bytes) {
            Ok(state) => println!("line {}: {:?}", i + 1, state),
            Err(err) => println!("line {}: decode error: {err}", i + 1),
        }
    }
    Ok(())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

// The platform-appropriate feature on `sc-xinput` (`vigem` / `uinput`) is
// unconditionally enabled per-target via the `[target.'cfg(...)'.dependencies]`
// tables in this crate's Cargo.toml, so a plain `cfg(windows)`/`cfg(unix)`
// here is sufficient.
#[cfg(windows)]
fn make_pad() -> anyhow::Result<impl VirtualPad> {
    sc_xinput::windows_vigem::VigemPad::new()
}

#[cfg(unix)]
fn make_pad() -> anyhow::Result<impl VirtualPad> {
    sc_xinput::linux_uinput::UinputPad::new()
}
