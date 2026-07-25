//! USB HID device discovery and I/O for the Steam Controller.

use std::ffi::CString;

use hidapi::{HidApi, HidDevice};

/// Valve's USB vendor ID. Shared across all their controllers.
pub const VALVE_VID: u16 = 0x28de;

/// PID observed on a real "new Steam Controller" unit during development
/// (2026-07-25), connected over USB directly (wired). Other units/revisions
/// may differ — override with `--pid` / [`DeviceFilter::pid`] and
/// `sc-adapter --list` if this doesn't match.
pub const DEFAULT_PID: u16 = 0x1302;

/// PID observed on the wireless receiver ("Steam Controller Puck") that
/// ships with the controller, distinct from [`DEFAULT_PID`]. Pass
/// `--pid 0x1304` (or let `scripts/run.sh`/`run.ps1` auto-detect it) when
/// using the controller wirelessly instead of over a direct USB connection.
pub const WIRELESS_PUCK_PID: u16 = 0x1304;

#[derive(Debug, thiserror::Error)]
pub enum HidError {
    #[error("hidapi error: {0}")]
    HidApi(#[from] hidapi::HidError),
    #[error("no matching device found (vid={vid:#06x}, pid={pid:?})")]
    NotFound { vid: u16, pid: Option<u16> },
}

/// Criteria for locating the controller among connected HID devices.
#[derive(Debug, Clone, Copy)]
pub struct DeviceFilter {
    pub vid: u16,
    /// `None` matches any PID under `vid`.
    pub pid: Option<u16>,
}

impl Default for DeviceFilter {
    fn default() -> Self {
        Self {
            vid: VALVE_VID,
            pid: Some(DEFAULT_PID),
        }
    }
}

/// A basic description of a discovered HID device, for `--list` output.
#[derive(Debug, Clone)]
pub struct DeviceSummary {
    pub vid: u16,
    pub pid: u16,
    pub path: String,
    pub product: Option<String>,
    pub serial: Option<String>,
}

/// Enumerate all connected HID devices matching `vid` (PID unfiltered),
/// primarily so a user can discover the real PID of their controller.
pub fn list_devices(api: &HidApi, vid: u16) -> Vec<DeviceSummary> {
    api.device_list()
        .filter(|d| d.vendor_id() == vid)
        .map(|d| DeviceSummary {
            vid: d.vendor_id(),
            pid: d.product_id(),
            path: d.path().to_string_lossy().into_owned(),
            product: d.product_string().map(str::to_owned),
            serial: d.serial_number().map(str::to_owned),
        })
        .collect()
}

/// An open handle to the controller's HID interface.
pub struct SteamControllerDevice {
    device: HidDevice,
}

/// How long to wait for a probe report before giving up on a candidate
/// interface and moving to the next one. Generous relative to the report
/// rate observed on both the wired connection and the wireless dongle's
/// active channel (well under a millisecond between reports in practice),
/// while keeping the total worst-case startup delay (this, times the
/// number of candidate interfaces) small.
const PROBE_TIMEOUT_MS: i32 = 300;

impl SteamControllerDevice {
    /// Open the device matching `filter`.
    ///
    /// A single physical unit can expose more than one HID interface with
    /// the *same* vid/pid — most notably the wireless dongle/"puck", which
    /// presents one interface per pairing channel (plus a control
    /// interface) rather than the single interface a wired unit has. Only
    /// the channel with an actual paired, awake controller ever emits
    /// reports; the rest sit silent. So when more than one interface
    /// matches, this briefly probes each in turn and picks the first one
    /// that produces *any* report within [`PROBE_TIMEOUT_MS`] — cheap and
    /// protocol-agnostic (no need to know report IDs/layout here, that's
    /// `sc-protocol`'s job) since an idle/unpaired channel simply never
    /// sends anything at all. Falls back to the first candidate if none
    /// respond (e.g. the controller is asleep), so `open` still succeeds
    /// and the normal reconnect-retry loop in `sc-adapter` gets a chance
    /// once it wakes up.
    pub fn open(api: &HidApi, filter: DeviceFilter) -> Result<Self, HidError> {
        let candidates: Vec<(CString, u16, u16)> = api
            .device_list()
            .filter(|d| {
                d.vendor_id() == filter.vid
                    && filter.pid.is_none_or(|pid| d.product_id() == pid)
            })
            .map(|d| (d.path().to_owned(), d.vendor_id(), d.product_id()))
            .collect();

        if candidates.is_empty() {
            return Err(HidError::NotFound {
                vid: filter.vid,
                pid: filter.pid,
            });
        }

        if candidates.len() == 1 {
            let (path, vid, pid) = &candidates[0];
            return Self::open_candidate(api, path, *vid, *pid);
        }

        let mut probe_buf = [0u8; 64];
        for (path, vid, pid) in &candidates {
            let Ok(candidate) = Self::open_candidate(api, path, *vid, *pid) else {
                continue;
            };
            match candidate.read_report(&mut probe_buf, PROBE_TIMEOUT_MS) {
                Ok(n) if n > 0 => return Ok(candidate),
                _ => tracing::debug!(
                    path = %path.to_string_lossy(),
                    "interface produced no data while probing, trying next"
                ),
            }
        }

        tracing::warn!(
            "no matching HID interface produced data while probing \
             (controller may be asleep/unpaired) — using the first one found"
        );
        let (path, vid, pid) = &candidates[0];
        Self::open_candidate(api, path, *vid, *pid)
    }

    fn open_candidate(api: &HidApi, path: &CString, vid: u16, pid: u16) -> Result<Self, HidError> {
        let device = api.open_path(path)?;
        tracing::info!(
            vid = %format!("{vid:#06x}"),
            pid = %format!("{pid:#06x}"),
            path = %path.to_string_lossy(),
            "opened Steam Controller HID device"
        );
        Ok(Self { device })
    }

    /// Blocking read of one HID report into `buf`, with a timeout in
    /// milliseconds. Returns the number of bytes read (0 on timeout).
    pub fn read_report(&self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, HidError> {
        Ok(self.device.read_timeout(buf, timeout_ms)?)
    }

    /// Write a raw HID output report (first byte is the report ID).
    pub fn write_report(&self, report: &[u8]) -> Result<usize, HidError> {
        Ok(self.device.write(report)?)
    }

    /// Trigger a haptic pulse on one motor.
    ///
    /// The controller has (per the owner's own research) 4 haptic motors.
    /// Determined empirically on 2026-07-25 by sweeping output-report byte
    /// positions (found via the real HID report descriptor — see
    /// `sc-protocol` docs, not a public spec) and asking a human to report
    /// what they felt. Confidence varies a lot by motor — see [`Motor`].
    ///
    /// Note: XInput's own rumble API only ever carries 2 intensities
    /// (left/right), so [`Motor::A`]/[`Motor::B`] are the only two with an
    /// actual path through this adapter's `VirtualPad` output today; `C`
    /// and `D` are exposed here for completeness/future use but nothing
    /// currently drives them automatically.
    pub fn send_rumble_pulse(&self, motor: Motor, amplitude: u8) -> Result<usize, HidError> {
        let (report_id, len, byte_offsets): (u8, usize, &[usize]) = match motor {
            Motor::A => (0x80, 10, &[4, 5]),
            Motor::B => (0x80, 10, &[7, 8]),
            Motor::C => (0x81, 8, &[2]),
            Motor::D => (0x84, 9, &[3]),
        };
        let mut report = vec![0u8; len];
        report[0] = report_id;
        for &off in byte_offsets {
            report[off] = amplitude;
        }
        self.write_report(&report)
    }
}

/// One of the controller's 4 haptic motors.
///
/// - [`Motor::A`] / [`Motor::B`]: HID output report `0x80`, high
///   confidence, highly reproducible discrete pulses. Felt like they came
///   from the left ([`Motor::A`]) and right ([`Motor::B`]) grip/handle
///   area in testing, but that side assignment rests on a couple of
///   subjective reports and should be treated as tentative.
/// - [`Motor::C`]: HID output report `0x81`, byte 2. Moderate confidence —
///   reproduced twice, felt like it came from further forward/toward one
///   touchpad rather than the grips.
/// - [`Motor::D`]: HID output report `0x84`, byte 3. Low confidence —
///   inconsistent across repeated attempts, distinct "double pulse"
///   character when it did register, physical location unclear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motor {
    A,
    B,
    C,
    D,
}
