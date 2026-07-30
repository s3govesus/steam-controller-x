//! Suppresses the kernel's phantom "Mouse"/"Keyboard" `evdev` devices for
//! this controller.
//!
//! No dedicated kernel HID driver exists for this hardware (unlike the
//! original Steam Controller's `hid-steam`), so the kernel falls back to
//! `hid-generic`, which dutifully creates a standard-HID mouse and keyboard
//! `evdev` device for every USB interface the controller exposes — visible
//! as `"Valve Software Steam Controller Puck Mouse"` / `"...Keyboard"` in
//! `/proc/bus/input/devices`, confirmed on real hardware on 2026-07-25
//! (4 pairs over the wireless puck's 4 pairing-channel interfaces). Whatever
//! those usage pages carry goes straight into X11/Wayland/logind as real
//! keyboard/mouse input, completely independent of `sc-adapter`'s own
//! `hidraw` reads — `hidraw` is a parallel, non-exclusive read path and
//! doesn't stop the kernel's normal `evdev` processing of the same reports.
//!
//! This grabs each such device exclusively (`EVIOCGRAB`) for as long as the
//! returned [`GrabbedInputDevices`] is alive, which swallows those events at
//! the kernel level so nothing downstream (compositor, `logind`, etc.) ever
//! sees them. Opt-in via `--grab-phantom-inputs` since it requires
//! read/write access to `/dev/input/eventN` (typically already granted to
//! the active graphical session via `udev`'s `uaccess` tag, the same
//! mechanism that grants `/dev/hidrawN` access).

#[cfg(target_os = "linux")]
mod imp {
    use std::fs::OpenOptions;
    use std::io;
    use std::os::unix::io::AsRawFd;

    /// `EVIOCGRAB` from `linux/input.h`: `_IOW('E', 0x90, int)`. Stable
    /// kernel ABI constant, not exposed by the `libc` crate.
    const EVIOCGRAB: libc::c_ulong = 0x4004_4590;

    /// Holds open file descriptors for every phantom input device grabbed.
    /// The grab is released automatically when this (and thus each
    /// underlying `File`) is dropped — `EVIOCGRAB` has no separate "ungrab"
    /// call, closing the fd is sufficient.
    pub struct GrabbedInputDevices {
        _handles: Vec<std::fs::File>,
    }

    /// Find and exclusively grab every `/dev/input/eventN` device the
    /// kernel created for `vid`/`pid`. Best-effort: a device that fails to
    /// open or grab (e.g. permissions) is logged and skipped rather than
    /// treated as fatal, since the main adapter loop should keep running
    /// either way.
    pub fn grab(vid: u16, pid: u16) -> io::Result<GrabbedInputDevices> {
        let mut handles = Vec::new();
        for event_path in find_event_devices(vid, pid)? {
            let file = match OpenOptions::new().read(true).write(true).open(&event_path) {
                Ok(f) => f,
                Err(err) => {
                    tracing::warn!(path = %event_path, %err, "failed to open phantom input device");
                    continue;
                }
            };
            // SAFETY: `file` stays open for the ioctl call, and EVIOCGRAB
            // with argument `1` is a well-defined kernel request that takes
            // no pointer — just marks the fd's open instance as an
            // exclusive grab.
            let ret = unsafe { libc::ioctl(file.as_raw_fd(), EVIOCGRAB, 1i32) };
            if ret != 0 {
                tracing::warn!(
                    path = %event_path,
                    err = %io::Error::last_os_error(),
                    "failed to grab phantom input device"
                );
                continue;
            }
            tracing::info!(path = %event_path, "grabbed phantom mouse/keyboard input device");
            handles.push(file);
        }
        Ok(GrabbedInputDevices { _handles: handles })
    }

    /// Parse `/proc/bus/input/devices`, returning `/dev/input/eventN` for
    /// every block whose `I:` line matches `vid`/`pid`.
    fn find_event_devices(vid: u16, pid: u16) -> io::Result<Vec<String>> {
        let contents = std::fs::read_to_string("/proc/bus/input/devices")?;
        let want_vid = format!("Vendor={vid:04x}");
        let want_pid = format!("Product={pid:04x}");

        let mut out = Vec::new();
        for block in contents.split("\n\n") {
            let is_match = block
                .lines()
                .find(|l| l.starts_with("I:"))
                .is_some_and(|l| l.contains(&want_vid) && l.contains(&want_pid));
            if !is_match {
                continue;
            }
            for line in block.lines() {
                let Some(handlers) = line.strip_prefix("H: Handlers=") else {
                    continue;
                };
                for token in handlers.split_whitespace() {
                    if let Some(n) = token.strip_prefix("event") {
                        out.push(format!("/dev/input/event{n}"));
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(target_os = "linux")]
#[allow(unused_imports)] // GrabbedInputDevices is public API, held via `_` at the call site
pub use imp::{grab, GrabbedInputDevices};

#[cfg(not(target_os = "linux"))]
pub struct GrabbedInputDevices;

#[cfg(not(target_os = "linux"))]
pub fn grab(_vid: u16, _pid: u16) -> std::io::Result<GrabbedInputDevices> {
    tracing::warn!(
        "--grab-phantom-inputs has no effect on this platform (EVIOCGRAB is Linux-only)"
    );
    Ok(GrabbedInputDevices)
}
