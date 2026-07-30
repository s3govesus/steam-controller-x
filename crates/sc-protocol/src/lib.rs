//! Pure decoding logic for Steam Controller HID input reports.
//!
//! This crate does no I/O — it only turns raw report bytes into a
//! normalized [`PadState`]. See [`report`] for the (unverified, best-effort)
//! wire layout this is built against.

pub mod report;

pub use report::{decode, decode_telemetry};

use bitflags::bitflags;

bitflags! {
    /// Digital button state.
    ///
    /// Bit assignments were determined empirically on 2026-07-25 by
    /// capturing raw HID reports from a real "new Steam Controller" unit
    /// (USB VID `0x28de` PID `0x1302`) while pressing one control at a
    /// time — see [`report`] module docs for the byte layout.
    ///
    /// Byte 2 bit 0x20 was originally logged as "never observed to fire";
    /// a follow-up session (2026-07-26) isolated it as the real right
    /// *stick* click (pressing straight down on the stick, distinct from
    /// the right *touchpad*'s press-force click) — see
    /// [`Self::RIGHT_STICK_CLICK`]. Bit 0x80 of byte 4 is still unmapped
    /// (reserved/unknown).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ButtonFlags: u32 {
        // Report byte 2
        const A                   = 1 << 0;
        const B                   = 1 << 1;
        const X                   = 1 << 2;
        const Y                   = 1 << 3;
        /// The "..." quick-access button.
        const DOTS                = 1 << 4;
        /// The real right *stick* click (straight-down press on the
        /// stick), distinct from [`Self::RIGHT_PAD_CLICK`] (the right
        /// *touchpad*'s press-force click). Confirmed 2026-07-26: fires
        /// alongside [`Self::RIGHT_STICK_CAP_TOUCH`] (your thumb has to be
        /// on the stick to click it), and never alongside
        /// [`Self::RIGHT_PAD_CLICK`]/[`Self::RIGHT_PAD_TOUCH`] — isolated
        /// via a dedicated stick-click capture plus a touchpad-click
        /// negative control.
        const RIGHT_STICK_CLICK   = 1 << 5;
        const START               = 1 << 6;
        const RIGHT_PADDLE_UPPER  = 1 << 7;
        // Report byte 3
        const RIGHT_PADDLE_LOWER  = 1 << 8;
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
        /// Left touchpad press-force click. Unlike the rest of this type,
        /// not a byte 2-4 bitmask bit — set from `report::offset::STATUS`
        /// byte bit `0x04`, alongside [`Self::LEFT_PAD_TOUCH`]'s `0x02` and
        /// [`Self::LEFT_STICK_CAP_TOUCH`]'s `0x01` in that same byte — see
        /// `report::decode`.
        const LEFT_PAD_CLICK      = 1 << 23;
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
        /// Left touchpad touch. Set from `report::offset::STATUS` byte bit
        /// `0x02` (see [`Self::LEFT_PAD_CLICK`]), and additionally
        /// synthesized whenever `left_pad`'s position is non-zero as a
        /// belt-and-suspenders fallback in case the status bit misses a
        /// light touch — see `report::decode`. Not yet confirmed to fire
        /// alongside [`Self::LEFT_PAD_CLICK`] the same reliable way
        /// [`Self::RIGHT_PAD_TOUCH`] does with [`Self::RIGHT_PAD_CLICK`].
        const LEFT_PAD_TOUCH      = 1 << 25;
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
    /// 0 (released) to 32767 (fully pulled). 15-bit ADC?
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
    /// Raw wire byte 32. Originally believed to be capacitive grip-squeeze
    /// pressure based on a single before/after squeeze test, but a
    /// dedicated probe (`sc-hid`'s `grip_probe` example, 2026-07-29) showed
    /// it free-runs on its own at a steady ~14Hz regardless of touch,
    /// sweeping 0..=255 and wrapping every ~18s — a counter, not a
    /// pressure reading. Real identity unknown; see
    /// `report::offset::UNKNOWN_COUNTER_32`. The digital "is gripping"
    /// signal (`0x20` bit of `report::offset::STATUS`, folded into
    /// [`ButtonFlags::LEFT_STICK_CAP_TOUCH`]'s doc) is unrelated and still
    /// reliable.
    pub unknown_counter_32: u8,
    pub imu: ImuSample,
}

/// Decoded contents of the separate telemetry report (`0x7b`) — see
/// [`report::decode_telemetry`] and [`report::TELEMETRY_REPORT_ID`].
///
/// This is a genuinely different HID report from the main `0x42` input
/// stream: it was found by reading the device's HID report descriptor
/// directly rather than by pressing controls (see `feature_probe.rs`'s
/// module docs, and the "Protocol status" section of the README), streams
/// continuously and independently at its own (~2Hz) rate, and is *not*
/// part of [`PadState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Telemetry {
    /// Wireless RF signal strength, in dBm (more negative = weaker).
    ///
    /// Confirmed 2026-07-26 by empirical range only, not against a
    /// reference signal meter: report `0x7b` byte 9 is the most
    /// actively-varying byte in that report (jittered ~190-217 unsigned
    /// across observation), which only makes sense as a *signed* byte
    /// (two's complement) — that range is -66..-39, squarely inside
    /// plausible real-world RSSI. No other byte in the report produced a
    /// physically plausible range under either interpretation. Unconfirmed:
    /// exact accuracy/calibration, and behavior at the edge of range (very
    /// weak signal, e.g. controller far from the receiver).
    pub signal_dbm: i8,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("report too short to decode")]
    TooShort,
    #[error("unexpected report id: {0:#04x}")]
    UnexpectedReportId(u8),
}
