//! `uinput`-backed virtual gamepad (Linux only).
//!
//! Creates a virtual evdev device that advertises the same name, vendor,
//! product id, and button/axis capabilities as a real wired Xbox 360
//! controller. This is the same trick SDL2's Linux joystick backend uses to
//! recognize a device as XInput-shaped without going through the kernel
//! `xpad` driver — no ViGEm-equivalent bus is needed on Linux, `uinput` is
//! kernel-native.
//!
//! Requires read/write access to `/dev/uinput` (see the README for a udev
//! rule to grant this without root).
//!
//! Purely mechanical: [`XInputState`] is already fully mapped/shaped by
//! `sc-config`, so this just forwards bits/values to uinput events.

use uinput::event::absolute::{Hat, Position};
use uinput::event::controller::GamePad;
use uinput::Device;

use crate::{VirtualPad, XInputButtons, XInputState};

/// `BUS_USB` from `linux/input.h` — stable kernel ABI constant.
const BUS_USB: u16 = 0x03;

pub struct UinputPad {
    device: Device,
}

impl UinputPad {
    pub fn new() -> anyhow::Result<Self> {
        let device = uinput::default()?
            .name("Microsoft X-Box 360 pad")?
            .bus(BUS_USB)
            .vendor(0x045e)
            .product(0x028e)
            .version(0x0110)
            .event(GamePad::A)?
            .event(GamePad::B)?
            .event(GamePad::X)?
            .event(GamePad::Y)?
            .event(GamePad::TL)?
            .event(GamePad::TR)?
            .event(GamePad::Select)?
            .event(GamePad::Start)?
            .event(GamePad::Mode)?
            .event(GamePad::ThumbL)?
            .event(GamePad::ThumbR)?
            .event(Position::X)?
            .min(i16::MIN as i32)
            .max(i16::MAX as i32)
            .event(Position::Y)?
            .min(i16::MIN as i32)
            .max(i16::MAX as i32)
            .event(Position::RX)?
            .min(i16::MIN as i32)
            .max(i16::MAX as i32)
            .event(Position::RY)?
            .min(i16::MIN as i32)
            .max(i16::MAX as i32)
            .event(Position::Z)?
            .min(0)
            .max(255)
            .event(Position::RZ)?
            .min(0)
            .max(255)
            .event(Hat::X0)?
            .min(-1)
            .max(1)
            .event(Hat::Y0)?
            .min(-1)
            .max(1)
            .create()?;

        tracing::info!("uinput virtual Xbox 360-shaped gamepad created");
        Ok(Self { device })
    }

    fn set_button(&mut self, btn: GamePad, pressed: bool) -> anyhow::Result<()> {
        if pressed {
            self.device.press(&btn)?;
        } else {
            self.device.release(&btn)?;
        }
        Ok(())
    }
}

impl VirtualPad for UinputPad {
    fn update(&mut self, state: &XInputState) -> anyhow::Result<()> {
        let b = state.buttons;
        self.set_button(GamePad::A, b.contains(XInputButtons::A))?;
        self.set_button(GamePad::B, b.contains(XInputButtons::B))?;
        self.set_button(GamePad::X, b.contains(XInputButtons::X))?;
        self.set_button(GamePad::Y, b.contains(XInputButtons::Y))?;
        self.set_button(GamePad::TL, b.contains(XInputButtons::LEFT_BUMPER))?;
        self.set_button(GamePad::TR, b.contains(XInputButtons::RIGHT_BUMPER))?;
        self.set_button(GamePad::Select, b.contains(XInputButtons::BACK))?;
        self.set_button(GamePad::Start, b.contains(XInputButtons::START))?;
        self.set_button(GamePad::Mode, b.contains(XInputButtons::GUIDE))?;
        self.set_button(GamePad::ThumbL, b.contains(XInputButtons::LEFT_THUMB))?;
        self.set_button(GamePad::ThumbR, b.contains(XInputButtons::RIGHT_THUMB))?;

        self.device.position(&Position::X, state.thumb_lx as i32)?;
        self.device.position(&Position::Y, state.thumb_ly as i32)?;
        self.device.position(&Position::RX, state.thumb_rx as i32)?;
        self.device.position(&Position::RY, state.thumb_ry as i32)?;
        self.device
            .position(&Position::Z, state.left_trigger as i32)?;
        self.device
            .position(&Position::RZ, state.right_trigger as i32)?;

        let hat_x = match (
            b.contains(XInputButtons::DPAD_LEFT),
            b.contains(XInputButtons::DPAD_RIGHT),
        ) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        let hat_y = match (
            b.contains(XInputButtons::DPAD_UP),
            b.contains(XInputButtons::DPAD_DOWN),
        ) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        self.device.position(&Hat::X0, hat_x)?;
        self.device.position(&Hat::Y0, hat_y)?;

        self.device.synchronize()?;
        Ok(())
    }
}
