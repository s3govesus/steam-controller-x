//! Pure decoding logic for Steam Controller HID input reports.
//!
//! This crate does no I/O — it only turns raw report bytes into a
//! normalized [`PadState`]. See [`report`] for the (unverified, best-effort)
//! wire layout this is built against.

pub mod report;

pub use report::decode;

use bitflags::bitflags;

bitflags! {
    /// Digital button state.
    ///
    /// Bit assignments were determined empirically on 2026-07-25 by
    /// capturing raw HID reports from a real "new Steam Controller" unit
    /// (USB VID `0x28de` PID `0x1302`) while pressing one control at a
    /// time — see [`report`] module docs for the byte layout.
    ///
    /// Correction (same day, later session): byte 4 bit 0x10 was initially
    /// captured under a "right bumper" prompt and labeled accordingly, but
    /// the hardware's owner identified it afterward as actually the
    /// capacitive touch sensor on top of the right stick (used by the
    /// firmware to suppress accidental right-touchpad input while gripping
    /// the stick) — see [`Self::RIGHT_STICK_CAP_TOUCH`]. The real right
    /// bumper was then found at byte 3 bit 0x02, confirmed across two
    /// captures landing on the identical value, though contact seemed
    /// intermittent (present for well under 100% of a held press in both) —
    /// possibly a hardware quirk of this unit's shoulder button, not
    /// necessarily a decoding error.
    ///
    /// Bit 0x20 of report byte 2 and bit 0x80 of byte 4 were never observed
    /// to fire and are left unmapped (reserved/unknown).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ButtonFlags: u32 {
        // Report byte 2
        const A                   = 1 << 0;
        const B                   = 1 << 1;
        const X                   = 1 << 2;
        const Y                   = 1 << 3;
        /// The "..." quick-access button.
        const DOTS                = 1 << 4;
        const START               = 1 << 6;
        const RIGHT_PADDLE_UPPER  = 1 << 7;
        // Report byte 3
        const RIGHT_PADDLE_LOWER  = 1 << 8;
        /// The real right bumper/shoulder button — see the correction note
        /// above. Contact seemed intermittent during a held press in both
        /// confirming captures; may need re-verification on this or other
        /// units.
        const RIGHT_BUMPER        = 1 << 9;
        const DPAD_DOWN           = 1 << 10;
        const DPAD_RIGHT          = 1 << 11;
        const DPAD_LEFT           = 1 << 12;
        const DPAD_UP             = 1 << 13;
        const BACK_SELECT_VIEW    = 1 << 14;
        const LEFT_STICK_CLICK    = 1 << 15;
        // Report byte 4
        const GUIDE               = 1 << 16;
        const LEFT_PADDLE_UPPER   = 1 << 17;
        const LEFT_PADDLE_LOWER   = 1 << 18;
        const LEFT_BUMPER         = 1 << 19;
        /// **Not a button.** Capacitive touch sensor on top of the right
        /// stick — see the correction note above. Fires while gripping/
        /// resting a thumb on the right stick, independent of actually
        /// moving it. Left unmapped by default since binding it to an
        /// XInput button would cause spurious presses during normal
        /// right-stick use.
        const RIGHT_STICK_CAP_TOUCH = 1 << 20;
        /// Fires alongside [`Self::RIGHT_PAD_CLICK`] whenever the pad is
        /// touched; never independently isolated from click, so treat as
        /// "touched or clicked" rather than a confirmed pure-touch signal.
        const RIGHT_PAD_TOUCH     = 1 << 21;
        const RIGHT_PAD_CLICK     = 1 << 22;
        /// **Not a button**, and **not a raw positional bit** like the rest
        /// of this type — the left stick's counterpart to
        /// [`Self::RIGHT_STICK_CAP_TOUCH`], but exposed completely
        /// differently by the firmware. Gripping the left stick doesn't set
        /// a bit in the byte-2/3/4 bitmask at all; instead report offset 5
        /// (elsewhere treated as a "status/mode" byte) has its `0x01` bit
        /// set — `report::decode` synthesizes this flag via bitwise AND
        /// against that byte, not equality. Confirmed (2026-07-25) to
        /// correctly coexist with a simultaneous left-grip squeeze: that
        /// byte reads `0x21` (`0x01 | 0x20`, grip's own bit) when both are
        /// active together, not a mutually-exclusive third code as an
        /// earlier session's decode logic assumed — see
        /// `report::offset::STATUS`.
        const LEFT_STICK_CAP_TOUCH = 1 << 24;
    }
}

/// A single analog axis pair (stick or trackpad), raw signed range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StickAxis {
    pub x: i16,
    pub y: i16,
}

/// Raw 6-axis IMU sample (accelerometer + gyroscope).
///
/// Confirmed empirically on 2026-07-25: `accel` responds to static tilt
/// (stable, non-zero when held at an angle — see `report` module docs),
/// `gyro` responds only to rotation and settles near zero when held still
/// regardless of orientation. Per-channel axis semantics (which of the 3
/// accel/gyro channels is X/Y/Z or pitch/roll/yaw) are **not** confirmed
/// beyond `gyro[2]` being the best candidate for yaw (it showed the
/// strongest response to a flat left-right twisting motion). There is no
/// XInput field to forward this to, so it's decoded but currently unused
/// by the `sc-xinput` backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImuSample {
    pub accel: [i16; 3],
    pub gyro: [i16; 3],
}

/// Normalized snapshot of the controller's full input state for one report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadState {
    /// Per-device sequence counter (wraps every 256 reports), useful for
    /// detecting drops.
    pub sequence: u8,
    pub buttons: ButtonFlags,
    /// 0 (released) to 32767 (fully pulled).
    pub left_trigger: u16,
    /// 0 (released) to 32767 (fully pulled).
    pub right_trigger: u16,
    pub left_stick: StickAxis,
    pub right_stick: StickAxis,
    /// Left touchpad position; `(0, 0)` while untouched. No confirmed click
    /// signal — see `report` module docs.
    pub left_pad: StickAxis,
    /// Right touchpad position; `(0, 0)` while untouched. Distinct from
    /// [`ButtonFlags::RIGHT_PAD_TOUCH`]/[`ButtonFlags::RIGHT_PAD_CLICK`].
    pub right_pad: StickAxis,
    /// Capacitive grip-squeeze pressure, roughly 49 (unsqueezed) to 130+
    /// (firm squeeze). Appears to be a single combined reading rather than
    /// independent left/right sensors — squeezing either grip alone moved
    /// it by about the same amount in testing.
    pub grip: u8,
    pub imu: ImuSample,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("report too short to decode")]
    TooShort,
    #[error("unexpected report id: {0:#04x}")]
    UnexpectedReportId(u8),
}
