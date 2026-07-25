//! ViGEmBus-backed virtual Xbox 360 controller (Windows only).
//!
//! Requires the [ViGEmBus driver](https://github.com/ViGEm/ViGEmBus) to be
//! installed on the target machine.
//!
//! Purely mechanical: [`XInputState`] is already fully mapped/shaped by
//! `sc-config`, so this just copies bits/values across to ViGEm's types.

use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};

use crate::{VirtualPad, XInputButtons, XInputState};

pub struct VigemPad {
    target: Xbox360Wired<Client>,
}

impl VigemPad {
    /// Connects to ViGEmBus and plugs in a new virtual Xbox 360 controller.
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::connect()?;
        let mut target = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
        target.plugin()?;
        target.wait_ready()?;
        tracing::info!("ViGEmBus virtual Xbox 360 controller plugged in");
        Ok(Self { target })
    }
}

impl VirtualPad for VigemPad {
    fn update(&mut self, state: &XInputState) -> anyhow::Result<()> {
        let b = state.buttons;
        let mut raw = 0u16;
        let mut set = |flag: XInputButtons, bit: u16| {
            if b.contains(flag) {
                raw |= bit;
            }
        };
        set(XInputButtons::A, XButtons::A);
        set(XInputButtons::B, XButtons::B);
        set(XInputButtons::X, XButtons::X);
        set(XInputButtons::Y, XButtons::Y);
        set(XInputButtons::LEFT_BUMPER, XButtons::LB);
        set(XInputButtons::RIGHT_BUMPER, XButtons::RB);
        set(XInputButtons::LEFT_THUMB, XButtons::LTHUMB);
        set(XInputButtons::RIGHT_THUMB, XButtons::RTHUMB);
        set(XInputButtons::START, XButtons::START);
        set(XInputButtons::BACK, XButtons::BACK);
        set(XInputButtons::GUIDE, XButtons::GUIDE);
        set(XInputButtons::DPAD_UP, XButtons::UP);
        set(XInputButtons::DPAD_DOWN, XButtons::DOWN);
        set(XInputButtons::DPAD_LEFT, XButtons::LEFT);
        set(XInputButtons::DPAD_RIGHT, XButtons::RIGHT);

        let gamepad = XGamepad {
            buttons: XButtons(raw),
            left_trigger: state.left_trigger,
            right_trigger: state.right_trigger,
            thumb_lx: state.thumb_lx,
            thumb_ly: state.thumb_ly,
            thumb_rx: state.thumb_rx,
            thumb_ry: state.thumb_ry,
        };

        self.target.update(&gamepad)?;
        Ok(())
    }
}

impl Drop for VigemPad {
    fn drop(&mut self) {
        let _ = self.target.unplug();
    }
}
