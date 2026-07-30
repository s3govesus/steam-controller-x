//! Raw HID input report layout for the new Steam Controller.
//!
//! **Provenance:** this layout was determined empirically on 2026-07-25 by
//! capturing raw USB HID reports from a real unit (VID `0x28de`, PID
//! `0x1302`) via `sc-adapter --dump-raw`, isolating one control at a time
//! and diffing against an idle baseline. It is not sourced from a public
//! spec or reused from the original Steam Controller — that device's
//! report format (report ID `0x01`, different button/axis offsets) turned
//! out not to match this hardware at all.
//!
//! Confirmed:
//! - Report ID, sequence counter, all 20 digital buttons (including the
//!   "dots" quick-access button). Corrected in a follow-up session: the
//!   bit originally captured under a "right bumper" prompt turned out to
//!   be the right stick's capacitive touch sensor instead (see
//!   [`crate::ButtonFlags::RIGHT_STICK_CAP_TOUCH`]) — the real right
//!   bumper was found separately at byte 3 bit 0x02.
//! - Both trigger axes; left stick, right stick, left touchpad, and right
//!   touchpad axes (sign and full-scale range verified in both directions
//!   for every axis). The two touchpads and two sticks are physically
//!   distinct surfaces — this controller has both, like Steam Deck.
//! - The 6-axis IMU block (accelerometer + gyroscope), distinguished via
//!   static-tilt-and-hold tests (accelerometer channels shift to a stable
//!   non-zero value and stay there; gyroscope channels spike during motion
//!   and settle back near zero once still). Per-channel axis semantics
//!   (which of the 3 accel/gyro channels is X/Y/Z or pitch/roll/yaw) are
//!   *not* confirmed beyond gyro channel 3 being the best candidate for
//!   yaw.
//! - The left stick's capacitive cap-touch sensor (counterpart to
//!   [`crate::ButtonFlags::RIGHT_STICK_CAP_TOUCH`]) — but exposed
//!   completely differently: not a bit in the button bitmask at all, just
//!   bit `0x01` of the status byte (offset 5). See
//!   [`crate::ButtonFlags::LEFT_STICK_CAP_TOUCH`] — confirmed to coexist
//!   correctly with the left grip's bit (`0x20`) as of a 2026-07-25
//!   capture, after an earlier session's "exclusive value" theory turned
//!   out to be wrong (see the `offset::STATUS` doc comment below).
//!
//! Not yet confirmed (left unimplemented rather than guessed):
//! - Left touchpad click (only the right pad's click signal was isolated;
//!   the left may be software-synthesized rather than exposed over HID).
//! - Battery level, connection-type/status bits.
//! - An analog capacitive grip-squeeze *pressure* reading. Byte 32 was
//!   originally logged as this (see `git blame`), based on a single
//!   before/after squeeze test that isn't safe to trust: a dedicated probe
//!   (`sc-hid`'s `grip_probe` example) on 2026-07-29 showed byte 32 free-runs
//!   on its own at a steady ~14Hz regardless of touch, sweeping its full
//!   0..=255 range and wrapping roughly every 18s — a counter, not a
//!   pressure sensor. It's tracked as `offset::UNKNOWN_COUNTER_32` /
//!   [`PadState::unknown_counter_32`] now instead of being surfaced as grip.
//!   A proper isolation test to find the real byte (`sc-hid`'s
//!   `isolate_grip_byte` example: idle baseline / sustained squeeze /
//!   release, diffing per-byte value ranges across all three phases so a
//!   real signal can't be confused with a byte that's simply always
//!   drifting) also ran on 2026-07-29 and found **no byte in this report
//!   that shifts to a new stable range during a squeeze and returns after
//!   release** — so either the pressure signal isn't in report `0x42` at
//!   all (maybe one of the still-unexamined report IDs from
//!   `feature_probe.rs`?), or it needs a firmer/longer squeeze than that
//!   run used to show up cleanly. The `0x20` bit in `offset::STATUS` (see
//!   [`crate::ButtonFlags::LEFT_STICK_CAP_TOUCH`]'s doc comment) remains a
//!   separate, independently-confirmed *digital* (on/off, not pressure)
//!   grip-squeeze signal and is unaffected by any of this.
//! - Whether/how the capacitive stick sensors suppress the co-located
//!   touchpad while a stick is gripped (mentioned by the hardware owner;
//!   the decode-side flags now correctly fire in combination with grip,
//!   but no consumer in `sc-config`/`sc-adapter` yet acts on them to
//!   actually suppress touchpad input).
//!
//! This file is intentionally the only place that knows about wire
//! offsets — nothing outside it should need to change to correct or extend
//! the layout. To extend it, follow the same capture methodology: run
//! `sc-adapter --dump-raw`, isolate one signal at a time, diff against
//! idle (add `--tilt`-style static-hold tests, not just press/release, for
//! anything that might be a rate sensor rather than a position sensor).

use crate::{ButtonFlags, DecodeError, ImuSample, PadState, StickAxis, Telemetry};

/// The main gamepad input report type — everything in [`PadState`].
pub const INPUT_REPORT_ID: u8 = 0x42;

/// Minimum report length we're willing to parse. Anything shorter is
/// rejected rather than guessed at.
pub const MIN_REPORT_LEN: usize = 46;

/// The separate telemetry report type — see [`Telemetry`]/[`decode_telemetry`].
pub const TELEMETRY_REPORT_ID: u8 = 0x7b;

/// Minimum length for [`decode_telemetry`]. The full report is 13 bytes
/// (including the id byte); this only needs through byte 9.
pub const TELEMETRY_MIN_LEN: usize = 10;

// Byte offsets within the report (including the leading report-ID byte).
mod offset {
    pub const SEQUENCE: usize = 1; // u8, wraps every 256 reports
    pub const BUTTONS_0: usize = 2; // A/B/X/Y/DOTS/START/RIGHT_PADDLE_UPPER
    pub const BUTTONS_1: usize = 3; // RIGHT_PADDLE_LOWER/RIGHT_BUMPER/DPAD/BACK/LEFT_STICK_CLICK
    pub const BUTTONS_2: usize = 4; // GUIDE/paddles/LEFT_BUMPER/RIGHT_STICK_CAP_TOUCH/RIGHT_PAD touch+click
    /// "Status/mode" byte — takes on different values depending on which
    /// subsystem is active (idle is 0x30; many buttons read 0x00; various
    /// others 0x06/0x08/0x10/0x31 have been observed). Originally believed
    /// to hold one exclusive value at a time, but a live capture on
    /// 2026-07-25 (squeezing the left grip while touching the left stick)
    /// showed it reading `0x21` — exactly `0x01 | 0x20`, i.e. the left
    /// stick's cap-touch bit (`0x01`, confirmed alone) OR'd with the left
    /// grip's bit (`0x20`, confirmed alone by squeezing the grip with the
    /// stick untouched). So it's a bitfield, at least for these two bits;
    /// the only bit currently decoded from it is `0x01`, tested via
    /// bitwise AND (not equality) so it still fires when combined with
    /// other simultaneously-active bits, synthesized into
    /// [`ButtonFlags::LEFT_STICK_CAP_TOUCH`].
    pub const STATUS: usize = 5;
    pub const LEFT_TRIGGER: usize = 6; // u16 LE, 0..=32767
    pub const RIGHT_TRIGGER: usize = 8; // u16 LE, 0..=32767
    pub const LEFT_STICK_X: usize = 10; // i16 LE
    pub const LEFT_STICK_Y: usize = 12; // i16 LE
    pub const RIGHT_STICK_X: usize = 14; // i16 LE
    pub const RIGHT_STICK_Y: usize = 16; // i16 LE
    pub const LEFT_PAD_X: usize = 18; // i16 LE, (0,0) while untouched
    pub const LEFT_PAD_Y: usize = 20; // i16 LE, (0,0) while untouched
    pub const RIGHT_PAD_X: usize = 24; // i16 LE, (0,0) while untouched
    pub const RIGHT_PAD_Y: usize = 26; // i16 LE, (0,0) while untouched
    /// u8. NOT grip pressure — despite the name this byte was originally
    /// captured under, `sc-hid`'s `grip_probe` example confirmed
    /// (2026-07-29) it free-runs at a steady ~14Hz regardless of touch,
    /// full-range 0..=255, wrapping every ~18s. Real identity unknown; see
    /// the module doc comment's "Not yet confirmed" section.
    pub const UNKNOWN_COUNTER_32: usize = 32;
    pub const ACCEL: usize = 34; // 3 x i16 LE
    pub const GYRO: usize = 40; // 3 x i16 LE
}

fn read_i16(buf: &[u8], offset: usize) -> Result<i16, DecodeError> {
    let bytes: [u8; 2] = buf
        .get(offset..offset + 2)
        .ok_or(DecodeError::TooShort)?
        .try_into()
        .map_err(|_| DecodeError::TooShort)?;
    Ok(i16::from_le_bytes(bytes))
}

fn read_u16(buf: &[u8], offset: usize) -> Result<u16, DecodeError> {
    let bytes: [u8; 2] = buf
        .get(offset..offset + 2)
        .ok_or(DecodeError::TooShort)?
        .try_into()
        .map_err(|_| DecodeError::TooShort)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_i16_triplet(buf: &[u8], offset: usize) -> Result<[i16; 3], DecodeError> {
    Ok([
        read_i16(buf, offset)?,
        read_i16(buf, offset + 2)?,
        read_i16(buf, offset + 4)?,
    ])
}

/// Decode a raw HID input report into a [`PadState`].
///
/// `report` should be the full report as read from the device, including
/// the leading report-ID byte.
pub fn decode(report: &[u8]) -> Result<PadState, DecodeError> {
    if report.is_empty() {
        return Err(DecodeError::TooShort);
    }
    if report[0] != INPUT_REPORT_ID {
        return Err(DecodeError::UnexpectedReportId(report[0]));
    }
    if report.len() < MIN_REPORT_LEN {
        return Err(DecodeError::TooShort);
    }

    let sequence = report[offset::SEQUENCE];

    let raw = [
        report[offset::BUTTONS_0],
        report[offset::BUTTONS_1],
        report[offset::BUTTONS_2],
    ];
    let mut buttons = ButtonFlags::from_bits_truncate(
        (raw[0] as u32) | ((raw[1] as u32) << 8) | ((raw[2] as u32) << 16),
    );
    let status = report[offset::STATUS];
    if status & 0x01 != 0 {
        buttons |= ButtonFlags::LEFT_STICK_CAP_TOUCH;
    }
    if status & 0x02 != 0 {
        buttons |= ButtonFlags::LEFT_PAD_TOUCH;
    }
    if status & 0x04 != 0 {
        buttons |= ButtonFlags::LEFT_PAD_CLICK;
    }

    let left_trigger = read_u16(report, offset::LEFT_TRIGGER)?;
    let right_trigger = read_u16(report, offset::RIGHT_TRIGGER)?;

    let left_stick = StickAxis {
        x: read_i16(report, offset::LEFT_STICK_X)?,
        y: read_i16(report, offset::LEFT_STICK_Y)?,
    };
    let right_stick = StickAxis {
        x: read_i16(report, offset::RIGHT_STICK_X)?,
        y: read_i16(report, offset::RIGHT_STICK_Y)?,
    };
    let left_pad = StickAxis {
        x: read_i16(report, offset::LEFT_PAD_X)?,
        y: read_i16(report, offset::LEFT_PAD_Y)?,
    };
    if left_pad.x != 0 || left_pad.y != 0 {
        buttons |= ButtonFlags::LEFT_PAD_TOUCH;
    }
    let right_pad = StickAxis {
        x: read_i16(report, offset::RIGHT_PAD_X)?,
        y: read_i16(report, offset::RIGHT_PAD_Y)?,
    };

    let unknown_counter_32 = report[offset::UNKNOWN_COUNTER_32];

    let imu = ImuSample {
        accel: read_i16_triplet(report, offset::ACCEL)?,
        gyro: read_i16_triplet(report, offset::GYRO)?,
    };

    Ok(PadState {
        sequence,
        buttons,
        left_trigger,
        right_trigger,
        left_stick,
        right_stick,
        left_pad,
        right_pad,
        unknown_counter_32,
        imu,
    })
}

/// Decode the separate telemetry report (`0x7b`) — see [`Telemetry`].
///
/// `report` should be the full report as read from the device, including
/// the leading report-ID byte.
pub fn decode_telemetry(report: &[u8]) -> Result<Telemetry, DecodeError> {
    if report.is_empty() {
        return Err(DecodeError::TooShort);
    }
    if report[0] != TELEMETRY_REPORT_ID {
        return Err(DecodeError::UnexpectedReportId(report[0]));
    }
    if report.len() < TELEMETRY_MIN_LEN {
        return Err(DecodeError::TooShort);
    }
    Ok(Telemetry { signal_dbm: report[9] as i8 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_report() -> Vec<u8> {
        let mut r = vec![0u8; MIN_REPORT_LEN];
        r[0] = INPUT_REPORT_ID;
        r[offset::SEQUENCE] = 42;
        r[offset::BUTTONS_0] = ButtonFlags::A.bits() as u8;
        r[offset::BUTTONS_1] =
            ((ButtonFlags::LEFT_STICK_CLICK | ButtonFlags::RIGHT_BUMPER).bits() >> 8) as u8;
        r[offset::BUTTONS_2] =
            ((ButtonFlags::LEFT_BUMPER | ButtonFlags::RIGHT_STICK_CAP_TOUCH).bits() >> 16) as u8;
        r[offset::LEFT_TRIGGER..offset::LEFT_TRIGGER + 2]
            .copy_from_slice(&30000u16.to_le_bytes());
        r[offset::RIGHT_TRIGGER..offset::RIGHT_TRIGGER + 2]
            .copy_from_slice(&15000u16.to_le_bytes());
        r[offset::LEFT_STICK_X..offset::LEFT_STICK_X + 2]
            .copy_from_slice(&(-1000i16).to_le_bytes());
        r[offset::LEFT_STICK_Y..offset::LEFT_STICK_Y + 2]
            .copy_from_slice(&(2000i16).to_le_bytes());
        r[offset::RIGHT_STICK_X..offset::RIGHT_STICK_X + 2]
            .copy_from_slice(&(-1500i16).to_le_bytes());
        r[offset::RIGHT_STICK_Y..offset::RIGHT_STICK_Y + 2]
            .copy_from_slice(&(2500i16).to_le_bytes());
        r[offset::LEFT_PAD_X..offset::LEFT_PAD_X + 2].copy_from_slice(&(300i16).to_le_bytes());
        r[offset::LEFT_PAD_Y..offset::LEFT_PAD_Y + 2].copy_from_slice(&(-300i16).to_le_bytes());
        r[offset::RIGHT_PAD_X..offset::RIGHT_PAD_X + 2]
            .copy_from_slice(&(500i16).to_le_bytes());
        r[offset::RIGHT_PAD_Y..offset::RIGHT_PAD_Y + 2]
            .copy_from_slice(&(-500i16).to_le_bytes());
        r[offset::UNKNOWN_COUNTER_32] = 130;
        for (i, v) in [10i16, 20, 30].iter().enumerate() {
            r[offset::ACCEL + i * 2..offset::ACCEL + i * 2 + 2]
                .copy_from_slice(&v.to_le_bytes());
        }
        for (i, v) in [40i16, 50, 60].iter().enumerate() {
            r[offset::GYRO + i * 2..offset::GYRO + i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        r
    }

    #[test]
    fn decodes_sequence_buttons_and_triggers() {
        let state = decode(&synthetic_report()).unwrap();
        assert_eq!(state.sequence, 42);
        assert!(state.buttons.contains(ButtonFlags::A));
        assert!(state.buttons.contains(ButtonFlags::LEFT_STICK_CLICK));
        assert!(state.buttons.contains(ButtonFlags::LEFT_BUMPER));
        assert!(state.buttons.contains(ButtonFlags::RIGHT_BUMPER));
        assert!(state.buttons.contains(ButtonFlags::RIGHT_STICK_CAP_TOUCH));
        assert!(!state.buttons.contains(ButtonFlags::RIGHT_STICK_CLICK));
        assert!(!state.buttons.contains(ButtonFlags::B));
        assert_eq!(state.left_trigger, 30000);
        assert_eq!(state.right_trigger, 15000);
    }

    #[test]
    fn decodes_sticks_and_pads() {
        let state = decode(&synthetic_report()).unwrap();
        assert_eq!(state.left_stick, StickAxis { x: -1000, y: 2000 });
        assert_eq!(state.right_stick, StickAxis { x: -1500, y: 2500 });
        assert_eq!(state.left_pad, StickAxis { x: 300, y: -300 });
        assert_eq!(state.right_pad, StickAxis { x: 500, y: -500 });
    }

    #[test]
    fn decodes_unknown_counter_32_and_imu() {
        let state = decode(&synthetic_report()).unwrap();
        assert_eq!(state.unknown_counter_32, 130);
        assert_eq!(state.imu.accel, [10, 20, 30]);
        assert_eq!(state.imu.gyro, [40, 50, 60]);
    }

    #[test]
    fn left_stick_cap_touch_synthesized_from_status_byte() {
        let mut r = synthetic_report();
        assert!(!decode(&r).unwrap().buttons.contains(ButtonFlags::LEFT_STICK_CAP_TOUCH));

        r[offset::STATUS] = 0x01;
        assert!(decode(&r).unwrap().buttons.contains(ButtonFlags::LEFT_STICK_CAP_TOUCH));
    }

    /// Regression test for a real bug: squeezing the left grip while
    /// touching the left stick was observed on real hardware to produce
    /// status byte `0x21` (`0x01 | 0x20`), not a mutually-exclusive code —
    /// an equality check against `0x01` alone missed this combination.
    #[test]
    fn left_stick_cap_touch_fires_alongside_grip_bit() {
        let mut r = synthetic_report();

        // Grip bit alone (0x20) must NOT be mistaken for stick cap-touch.
        r[offset::STATUS] = 0x20;
        assert!(!decode(&r).unwrap().buttons.contains(ButtonFlags::LEFT_STICK_CAP_TOUCH));

        // Grip bit combined with the stick cap-touch bit (0x21) must still
        // register the stick cap-touch flag.
        r[offset::STATUS] = 0x21;
        assert!(decode(&r).unwrap().buttons.contains(ButtonFlags::LEFT_STICK_CAP_TOUCH));
    }

    /// Regression test for a real-hardware finding (2026-07-26): byte 2 bit
    /// 0x20, previously never observed to fire, is the real right-*stick*
    /// click — distinct from [`ButtonFlags::RIGHT_PAD_CLICK`] (the right
    /// *touchpad*'s press-force click). Isolated via a dedicated capture
    /// plus a touchpad-click negative control that never set this bit.
    #[test]
    fn right_stick_click_distinct_from_right_pad_click() {
        let mut r = synthetic_report();
        r[offset::BUTTONS_0] |= 0x20;
        let state = decode(&r).unwrap();
        assert!(state.buttons.contains(ButtonFlags::RIGHT_STICK_CLICK));
        assert!(!state.buttons.contains(ButtonFlags::RIGHT_PAD_CLICK));
    }

    #[test]
    fn decodes_left_pad_touch_and_click() {
        let mut r = synthetic_report();
        // Clear synthetic non-zero pad position
        r[offset::LEFT_PAD_X..offset::LEFT_PAD_X + 2].copy_from_slice(&0i16.to_le_bytes());
        r[offset::LEFT_PAD_Y..offset::LEFT_PAD_Y + 2].copy_from_slice(&0i16.to_le_bytes());

        // Status byte 0x02 maps to LEFT_PAD_TOUCH
        r[offset::STATUS] = 0x02;
        let state_touch = decode(&r).unwrap();
        assert!(state_touch.buttons.contains(ButtonFlags::LEFT_PAD_TOUCH));
        assert!(!state_touch.buttons.contains(ButtonFlags::LEFT_PAD_CLICK));

        // Status byte 0x04 (or 0x06 = 0x02 | 0x04) maps to LEFT_PAD_CLICK
        r[offset::STATUS] = 0x06;
        let state_click = decode(&r).unwrap();
        assert!(state_click.buttons.contains(ButtonFlags::LEFT_PAD_TOUCH));
        assert!(state_click.buttons.contains(ButtonFlags::LEFT_PAD_CLICK));
    }

    #[test]
    fn rejects_wrong_report_id() {
        let mut r = synthetic_report();
        r[0] = 0x01;
        assert!(matches!(
            decode(&r),
            Err(DecodeError::UnexpectedReportId(0x01))
        ));
    }

    #[test]
    fn rejects_too_short() {
        assert!(matches!(decode(&[0x42, 0x00]), Err(DecodeError::TooShort)));
    }

    /// Regression test for a real-hardware finding (2026-07-26): report
    /// 0x7b's byte 9, interpreted as a signed byte, tracks a physically
    /// plausible RSSI range (observed ~-66..-39 dBm) — see
    /// [`crate::Telemetry::signal_dbm`].
    #[test]
    fn decodes_telemetry_signal_strength() {
        let mut r = [0u8; TELEMETRY_MIN_LEN];
        r[0] = TELEMETRY_REPORT_ID;
        r[9] = 0xce; // 206 unsigned -> -50 signed
        let telemetry = decode_telemetry(&r).unwrap();
        assert_eq!(telemetry.signal_dbm, -50);
    }

    #[test]
    fn telemetry_rejects_wrong_report_id() {
        let mut r = [0u8; TELEMETRY_MIN_LEN];
        r[0] = 0x42;
        assert!(matches!(
            decode_telemetry(&r),
            Err(DecodeError::UnexpectedReportId(0x42))
        ));
    }

    #[test]
    fn telemetry_rejects_too_short() {
        assert!(matches!(
            decode_telemetry(&[TELEMETRY_REPORT_ID, 0x00]),
            Err(DecodeError::TooShort)
        ));
    }
}
