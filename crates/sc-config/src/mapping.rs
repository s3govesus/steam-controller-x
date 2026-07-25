//! Deadzone + response-curve shaping for analog axes.
//!
//! Pure math, no I/O — takes normalized 0.0..=1.0 (or -1.0..=1.0 for the
//! signed 2D case) magnitudes and reshapes them. Standard "radial deadzone
//! with rescale" approach: values inside the deadzone become exactly zero,
//! and the remaining range is rescaled to fill 0.0..=1.0 rather than
//! jumping discontinuously at the deadzone boundary. `curve` is an
//! exponent applied after rescaling (1.0 = linear, >1.0 = more precision
//! near center/less near the edge, <1.0 = the opposite).

use serde::{Deserialize, Serialize};

/// Deadzone + curve settings for one analog axis (trigger or stick).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisSettings {
    /// Fraction of full scale (0.0..=1.0) below which the axis reads zero.
    pub deadzone: f32,
    /// Response curve exponent. 1.0 is linear.
    pub curve: f32,
}

impl Default for AxisSettings {
    fn default() -> Self {
        Self {
            deadzone: 0.0,
            curve: 1.0,
        }
    }
}

fn shape(normalized_magnitude: f32, settings: AxisSettings) -> f32 {
    let dz = settings.deadzone.clamp(0.0, 0.99);
    let mag = normalized_magnitude.clamp(0.0, 1.0);
    if mag <= dz {
        return 0.0;
    }
    let rescaled = (mag - dz) / (1.0 - dz);
    rescaled.powf(settings.curve.max(0.01))
}

/// Shape a 1D axis already normalized to `0..=32767` (triggers), returning
/// an XInput-ready `0..=255` byte.
pub fn shape_trigger(raw: u16, settings: AxisSettings) -> u8 {
    let normalized = raw as f32 / 32767.0;
    let shaped = shape(normalized, settings);
    (shaped * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Shape a signed 2D axis (stick), each component `-32767..=32767`,
/// applying a *radial* deadzone (a circle, not a per-axis square) and
/// curve, returning XInput-ready `i16` components.
pub fn shape_stick(x: i16, y: i16, settings: AxisSettings) -> (i16, i16) {
    let fx = x as f32 / 32767.0;
    let fy = y as f32 / 32767.0;
    let magnitude = (fx * fx + fy * fy).sqrt();
    let shaped_magnitude = shape(magnitude, settings);
    if magnitude < 0.0001 {
        return (0, 0);
    }
    let scale = shaped_magnitude / magnitude;
    let nx = (fx * scale * 32767.0).clamp(-32767.0, 32767.0) as i16;
    let ny = (fy * scale * 32767.0).clamp(-32767.0, 32767.0) as i16;
    (nx, ny)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_zero_inside_deadzone() {
        let settings = AxisSettings {
            deadzone: 0.2,
            curve: 1.0,
        };
        assert_eq!(shape_trigger(0, settings), 0);
        assert_eq!(shape_trigger((32767.0 * 0.1) as u16, settings), 0);
    }

    #[test]
    fn trigger_full_scale_at_max() {
        let settings = AxisSettings::default();
        assert_eq!(shape_trigger(32767, settings), 255);
    }

    #[test]
    fn trigger_linear_midpoint_no_deadzone() {
        let settings = AxisSettings::default();
        let half = shape_trigger(32767 / 2, settings);
        assert!((120..=135).contains(&half), "got {half}");
    }

    #[test]
    fn trigger_rescales_past_deadzone_instead_of_jumping() {
        let settings = AxisSettings {
            deadzone: 0.5,
            curve: 1.0,
        };
        // Just past the deadzone should be close to 0, not a big jump.
        let just_past = shape_trigger((32767.0 * 0.51) as u16, settings);
        assert!(just_past < 15, "got {just_past}");
    }

    #[test]
    fn stick_zero_inside_radial_deadzone() {
        let settings = AxisSettings {
            deadzone: 0.3,
            curve: 1.0,
        };
        // Small diagonal magnitude, well inside a 0.3 radius.
        assert_eq!(shape_stick(1000, 1000, settings), (0, 0));
    }

    #[test]
    fn stick_full_scale_at_extreme() {
        let settings = AxisSettings::default();
        let (x, y) = shape_stick(32767, 0, settings);
        assert_eq!((x, y), (32767, 0));
    }

    #[test]
    fn stick_preserves_direction() {
        let settings = AxisSettings::default();
        let (x, y) = shape_stick(-16000, 16000, settings);
        assert!(x < 0);
        assert!(y > 0);
    }

    #[test]
    fn stick_center_stays_center() {
        let settings = AxisSettings::default();
        assert_eq!(shape_stick(0, 0, settings), (0, 0));
    }
}
