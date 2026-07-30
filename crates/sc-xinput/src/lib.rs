//! Virtual XInput-compatible controller output.
//!
//! [`VirtualPad`] is the platform-agnostic sink; [`windows_vigem`] and
//! [`linux_uinput`] are the concrete backends, each gated behind a Cargo
//! feature plus the matching `cfg(windows)` / `cfg(unix)` target. Backends
//! are pure mechanical translators from [`XInputState`] to the target
//! API's own button/axis names — they know nothing about the physical
//! controller. Deciding which physical control maps to which
//! [`XInputButtons`] bit (and any deadzone/curve shaping) is
//! `sc-config`'s job, not this crate's.

use bitflags::bitflags;

bitflags! {
    /// The classic Xbox 360 gamepad's digital buttons — the full set both
    /// ViGEm's `Xbox360Wired` target and a uinput Xbox-360-shaped device
    /// support. There is deliberately no slot here for paddles, a guide
    /// analog, touchpads, grip, or IMU data — the classic XInput surface
    /// has none.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct XInputButtons: u16 {
        const A            = 1 << 0;
        const B            = 1 << 1;
        const X            = 1 << 2;
        const Y            = 1 << 3;
        const LEFT_BUMPER  = 1 << 4;
        const RIGHT_BUMPER = 1 << 5;
        const BACK         = 1 << 6;
        const START        = 1 << 7;
        const GUIDE        = 1 << 8;
        const LEFT_THUMB   = 1 << 9;
        const RIGHT_THUMB  = 1 << 10;
        const DPAD_UP      = 1 << 11;
        const DPAD_DOWN    = 1 << 12;
        const DPAD_LEFT    = 1 << 13;
        const DPAD_RIGHT   = 1 << 14;
    }
}

/// A fully mapped, ready-to-send XInput gamepad state — the output of
/// `sc-config`'s `Profile::map`, and the only thing a [`VirtualPad`]
/// backend needs to know how to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputState {
    pub buttons: XInputButtons,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

/// Something that can present an [`XInputState`] to the OS as a gamepad.
pub trait VirtualPad {
    fn update(&mut self, state: &XInputState) -> anyhow::Result<()>;
}

#[cfg(all(windows, feature = "vigem"))]
pub mod windows_vigem;

#[cfg(all(unix, feature = "uinput"))]
pub mod linux_uinput;
