//! Physical-control-to-XInput mapping and profile persistence.
//!
//! This is the layer that decides which physical button/axis drives which
//! XInput output, and by how much (deadzone/curve) — `sc-protocol` only
//! decodes raw hardware state, and `sc-xinput`'s backends only know how to
//! draw an already-decided [`sc_xinput::XInputState`]. Pure logic, no HID
//! or web I/O; only file I/O for loading/saving profiles.

mod mapping;

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context;
use sc_protocol::{ButtonFlags, PadState};
use sc_xinput::{XInputButtons, XInputState};
use serde::{Deserialize, Serialize};

pub use mapping::AxisSettings;

/// Every physical digital control that can be bound to an XInput button.
///
/// Mirrors [`sc_protocol::ButtonFlags`] 1:1. Kept as a separate enum (rather
/// than iterating `ButtonFlags` bits directly) so it serializes as plain
/// strings in profile files and is easy to list in a UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalButton {
    A,
    B,
    X,
    Y,
    Dots,
    Start,
    BackSelectView,
    Guide,
    LeftBumper,
    RightBumper,
    /// **Not really a button** — the right stick's capacitive touch
    /// sensor (fires while gripping/resting a thumb on the right stick).
    /// Left unbound by default; see [`sc_protocol::ButtonFlags::RIGHT_STICK_CAP_TOUCH`].
    RightStickCapTouch,
    /// The left stick's counterpart — see
    /// [`sc_protocol::ButtonFlags::LEFT_STICK_CAP_TOUCH`] for how
    /// differently it's encoded (and the resulting caveat about combining
    /// with simultaneous other input).
    LeftStickCapTouch,
    LeftStickClick,
    RightPadTouch,
    RightPadClick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    LeftPaddleUpper,
    LeftPaddleLower,
    RightPaddleUpper,
    RightPaddleLower,
}

impl LogicalButton {
    pub const ALL: [LogicalButton; 23] = [
        LogicalButton::A,
        LogicalButton::B,
        LogicalButton::X,
        LogicalButton::Y,
        LogicalButton::Dots,
        LogicalButton::Start,
        LogicalButton::BackSelectView,
        LogicalButton::Guide,
        LogicalButton::LeftBumper,
        LogicalButton::RightBumper,
        LogicalButton::RightStickCapTouch,
        LogicalButton::LeftStickCapTouch,
        LogicalButton::LeftStickClick,
        LogicalButton::RightPadTouch,
        LogicalButton::RightPadClick,
        LogicalButton::DpadUp,
        LogicalButton::DpadDown,
        LogicalButton::DpadLeft,
        LogicalButton::DpadRight,
        LogicalButton::LeftPaddleUpper,
        LogicalButton::LeftPaddleLower,
        LogicalButton::RightPaddleUpper,
        LogicalButton::RightPaddleLower,
    ];

    pub fn flag(self) -> ButtonFlags {
        match self {
            LogicalButton::A => ButtonFlags::A,
            LogicalButton::B => ButtonFlags::B,
            LogicalButton::X => ButtonFlags::X,
            LogicalButton::Y => ButtonFlags::Y,
            LogicalButton::Dots => ButtonFlags::DOTS,
            LogicalButton::Start => ButtonFlags::START,
            LogicalButton::BackSelectView => ButtonFlags::BACK_SELECT_VIEW,
            LogicalButton::Guide => ButtonFlags::GUIDE,
            LogicalButton::LeftBumper => ButtonFlags::LEFT_BUMPER,
            LogicalButton::RightBumper => ButtonFlags::RIGHT_BUMPER,
            LogicalButton::RightStickCapTouch => ButtonFlags::RIGHT_STICK_CAP_TOUCH,
            LogicalButton::LeftStickCapTouch => ButtonFlags::LEFT_STICK_CAP_TOUCH,
            LogicalButton::LeftStickClick => ButtonFlags::LEFT_STICK_CLICK,
            LogicalButton::RightPadTouch => ButtonFlags::RIGHT_PAD_TOUCH,
            LogicalButton::RightPadClick => ButtonFlags::RIGHT_PAD_CLICK,
            LogicalButton::DpadUp => ButtonFlags::DPAD_UP,
            LogicalButton::DpadDown => ButtonFlags::DPAD_DOWN,
            LogicalButton::DpadLeft => ButtonFlags::DPAD_LEFT,
            LogicalButton::DpadRight => ButtonFlags::DPAD_RIGHT,
            LogicalButton::LeftPaddleUpper => ButtonFlags::LEFT_PADDLE_UPPER,
            LogicalButton::LeftPaddleLower => ButtonFlags::LEFT_PADDLE_LOWER,
            LogicalButton::RightPaddleUpper => ButtonFlags::RIGHT_PADDLE_UPPER,
            LogicalButton::RightPaddleLower => ButtonFlags::RIGHT_PADDLE_LOWER,
        }
    }
}

/// Every XInput button a physical control can be bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XInputButton {
    A,
    B,
    X,
    Y,
    LeftBumper,
    RightBumper,
    Back,
    Start,
    Guide,
    LeftThumb,
    RightThumb,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

impl XInputButton {
    pub const ALL: [XInputButton; 15] = [
        XInputButton::A,
        XInputButton::B,
        XInputButton::X,
        XInputButton::Y,
        XInputButton::LeftBumper,
        XInputButton::RightBumper,
        XInputButton::Back,
        XInputButton::Start,
        XInputButton::Guide,
        XInputButton::LeftThumb,
        XInputButton::RightThumb,
        XInputButton::DpadUp,
        XInputButton::DpadDown,
        XInputButton::DpadLeft,
        XInputButton::DpadRight,
    ];

    pub fn bit(self) -> XInputButtons {
        match self {
            XInputButton::A => XInputButtons::A,
            XInputButton::B => XInputButtons::B,
            XInputButton::X => XInputButtons::X,
            XInputButton::Y => XInputButtons::Y,
            XInputButton::LeftBumper => XInputButtons::LEFT_BUMPER,
            XInputButton::RightBumper => XInputButtons::RIGHT_BUMPER,
            XInputButton::Back => XInputButtons::BACK,
            XInputButton::Start => XInputButtons::START,
            XInputButton::Guide => XInputButtons::GUIDE,
            XInputButton::LeftThumb => XInputButtons::LEFT_THUMB,
            XInputButton::RightThumb => XInputButtons::RIGHT_THUMB,
            XInputButton::DpadUp => XInputButtons::DPAD_UP,
            XInputButton::DpadDown => XInputButtons::DPAD_DOWN,
            XInputButton::DpadLeft => XInputButtons::DPAD_LEFT,
            XInputButton::DpadRight => XInputButtons::DPAD_RIGHT,
        }
    }
}

/// One physical-button-to-XInput-button binding. Absence from a profile's
/// `bindings` list means "unmapped" (e.g. paddles/dots by default, since
/// classic XInput has no natural slot for them).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ButtonBinding {
    pub button: LogicalButton,
    pub target: XInputButton,
}

/// A named, saveable/loadable adapter configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub left_stick: AxisSettings,
    #[serde(default)]
    pub right_stick: AxisSettings,
    #[serde(default)]
    pub left_trigger: AxisSettings,
    #[serde(default)]
    pub right_trigger: AxisSettings,
    #[serde(default = "Profile::default_bindings")]
    pub bindings: Vec<ButtonBinding>,
}

impl Profile {
    /// The bindings that reproduce this adapter's original hardcoded
    /// behavior, before remapping existed.
    fn default_bindings() -> Vec<ButtonBinding> {
        use LogicalButton::*;
        use XInputButton as X;
        vec![
            ButtonBinding { button: A, target: X::A },
            ButtonBinding { button: B, target: X::B },
            ButtonBinding { button: X, target: X::X },
            ButtonBinding { button: Y, target: X::Y },
            ButtonBinding { button: LeftBumper, target: X::LeftBumper },
            ButtonBinding { button: RightBumper, target: X::RightBumper },
            ButtonBinding { button: Start, target: X::Start },
            ButtonBinding { button: BackSelectView, target: X::Back },
            ButtonBinding { button: Guide, target: X::Guide },
            ButtonBinding { button: LeftStickClick, target: X::LeftThumb },
            ButtonBinding { button: RightPadClick, target: X::RightThumb },
            ButtonBinding { button: DpadUp, target: X::DpadUp },
            ButtonBinding { button: DpadDown, target: X::DpadDown },
            ButtonBinding { button: DpadLeft, target: X::DpadLeft },
            ButtonBinding { button: DpadRight, target: X::DpadRight },
        ]
    }

    /// Apply this profile's deadzone/curve/button-map settings to a raw
    /// decoded [`PadState`], producing a ready-to-send [`XInputState`].
    pub fn map(&self, state: &PadState) -> XInputState {
        let mut buttons = XInputButtons::empty();
        for binding in &self.bindings {
            if state.buttons.contains(binding.button.flag()) {
                buttons |= binding.target.bit();
            }
        }

        // The controller's raw Y axis reads positive when the stick is
        // pushed up (matches its native ADC direction, confirmed during
        // protocol RE — see `sc-protocol::report`), but XInput/SDL's
        // convention is the opposite: up is negative. Flip here, at the
        // raw-hardware -> XInput-target boundary, rather than in
        // `sc-protocol` (which should stay a faithful wire decode) or
        // `sc-xinput` (purely mechanical, no per-axis semantics).
        // `saturating_neg` avoids an overflow panic on `i16::MIN`.
        let (thumb_lx, thumb_ly) = mapping::shape_stick(
            state.left_stick.x,
            state.left_stick.y.saturating_neg(),
            self.left_stick,
        );
        let (thumb_rx, thumb_ry) = mapping::shape_stick(
            state.right_stick.x,
            state.right_stick.y.saturating_neg(),
            self.right_stick,
        );
        let left_trigger = mapping::shape_trigger(state.left_trigger, self.left_trigger);
        let right_trigger = mapping::shape_trigger(state.right_trigger, self.right_trigger);

        XInputState {
            buttons,
            left_trigger,
            right_trigger,
            thumb_lx,
            thumb_ly,
            thumb_rx,
            thumb_ry,
        }
    }

    /// A lookup from [`LogicalButton`] to its currently bound
    /// [`XInputButton`] (if any), for UI display.
    pub fn binding_map(&self) -> HashMap<LogicalButton, XInputButton> {
        self.bindings.iter().map(|b| (b.button, b.target)).collect()
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            left_stick: AxisSettings::default(),
            right_stick: AxisSettings::default(),
            left_trigger: AxisSettings::default(),
            right_trigger: AxisSettings::default(),
            bindings: Profile::default_bindings(),
        }
    }
}

/// Loads/saves [`Profile`]s as TOML files in the platform config directory
/// (e.g. `~/.config/steam-controller-x/profiles/*.toml` on Linux).
pub struct ProfileStore {
    dir: PathBuf,
}

impl ProfileStore {
    pub fn new() -> anyhow::Result<Self> {
        let proj = directories::ProjectDirs::from("", "", "steam-controller-x")
            .context("could not determine a config directory for this platform")?;
        let dir = proj.config_dir().join("profiles");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating profile directory {}", dir.display()))?;
        Ok(Self { dir })
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.toml"))
    }

    pub fn list(&self) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn load(&self, name: &str) -> anyhow::Result<Profile> {
        let text = std::fs::read_to_string(self.path_for(name))
            .with_context(|| format!("reading profile {name:?}"))?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self, profile: &Profile) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(profile)?;
        std::fs::write(self.path_for(&profile.name), text)
            .with_context(|| format!("writing profile {:?}", profile.name))
    }

    pub fn delete(&self, name: &str) -> anyhow::Result<()> {
        std::fs::remove_file(self.path_for(name))
            .with_context(|| format!("deleting profile {name:?}"))
    }

    fn active_marker(&self) -> PathBuf {
        self.dir.join(".active")
    }

    pub fn active_name(&self) -> Option<String> {
        std::fs::read_to_string(self.active_marker())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn set_active_name(&self, name: &str) -> anyhow::Result<()> {
        std::fs::write(self.active_marker(), name)
            .with_context(|| format!("recording active profile {name:?}"))
    }

    /// Loads whichever profile was last marked active, falling back to
    /// [`Profile::default`] if none is set or it fails to load.
    pub fn load_active_or_default(&self) -> Profile {
        self.active_name()
            .and_then(|name| self.load(&name).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_protocol::StickAxis;

    fn synthetic_state(buttons: ButtonFlags) -> PadState {
        PadState {
            sequence: 0,
            buttons,
            left_trigger: 0,
            right_trigger: 0,
            left_stick: Default::default(),
            right_stick: Default::default(),
            left_pad: Default::default(),
            right_pad: Default::default(),
            grip: 0,
            imu: Default::default(),
        }
    }

    #[test]
    fn default_profile_maps_a_to_a() {
        let profile = Profile::default();
        let state = synthetic_state(ButtonFlags::A);
        let mapped = profile.map(&state);
        assert!(mapped.buttons.contains(XInputButtons::A));
        assert!(!mapped.buttons.contains(XInputButtons::B));
    }

    #[test]
    fn paddles_unmapped_by_default() {
        let profile = Profile::default();
        let state = synthetic_state(ButtonFlags::LEFT_PADDLE_UPPER);
        let mapped = profile.map(&state);
        assert!(mapped.buttons.is_empty());
    }

    #[test]
    fn remapping_a_paddle_forwards_it() {
        let mut profile = Profile::default();
        profile.bindings.push(ButtonBinding {
            button: LogicalButton::LeftPaddleUpper,
            target: XInputButton::A,
        });
        let state = synthetic_state(ButtonFlags::LEFT_PADDLE_UPPER);
        let mapped = profile.map(&state);
        assert!(mapped.buttons.contains(XInputButtons::A));
    }

    /// Regression test for a real bug found via live testing: the
    /// controller's raw stick Y axis reads positive-up, but XInput/SDL's
    /// convention is negative-up, and games consuming the virtual pad
    /// (verified independently of Steam, via SDL2/pygame) saw the Y axis
    /// inverted as a result.
    #[test]
    fn stick_y_flipped_to_xinput_up_negative_convention() {
        let profile = Profile::default();
        let mut state = synthetic_state(ButtonFlags::empty());
        state.left_stick = StickAxis { x: 0, y: 16000 }; // raw "pushed up"
        state.right_stick = StickAxis { x: 0, y: -16000 }; // raw "pushed down"
        let mapped = profile.map(&state);
        assert!(mapped.thumb_ly < 0, "left stick up should map to negative Y");
        assert!(mapped.thumb_ry > 0, "right stick down should map to positive Y");
    }

    #[test]
    fn profile_round_trips_through_toml() {
        let profile = Profile::default();
        let text = toml::to_string_pretty(&profile).unwrap();
        let parsed: Profile = toml::from_str(&text).unwrap();
        assert_eq!(profile, parsed);
    }
}
