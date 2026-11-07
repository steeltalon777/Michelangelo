//! Package unit handling.
//!
//! Every length of an input package manifest is expressed in the package
//! unit (`m`, `cm` or `mm`). Blender works in metres, so the pipeline
//! converts every length before it reaches a Blender operation.

use michelangelo_input::manifest::Units;

use crate::error::ReconError;

/// The number of metres in one package unit.
pub fn unit_scale(units: Units) -> f64 {
    match units {
        Units::M => 1.0,
        Units::Cm => 0.01,
        Units::Mm => 0.001,
    }
}

/// Convert one length from package units to metres.
///
/// Non-finite values are rejected: they cannot be represented in Blender.
pub fn to_metres(value: f64, units: Units) -> Result<f64, ReconError> {
    if !value.is_finite() {
        return Err(ReconError::NonFinite {
            field: "length".to_string(),
        });
    }
    Ok(value * unit_scale(units))
}

/// Convert one point from package units to metres.
pub fn point_to_metres(point: [f64; 3], units: Units) -> Result<[f64; 3], ReconError> {
    let scale = unit_scale(units);
    let mut converted = [0.0; 3];
    for (index, value) in point.iter().enumerate() {
        if !value.is_finite() {
            return Err(ReconError::NonFinite {
                field: format!("point[{index}]"),
            });
        }
        converted[index] = value * scale;
    }
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metres_are_unchanged() {
        assert!((to_metres(2.5, Units::M).expect("finite value") - 2.5).abs() < 1e-12);
    }

    #[test]
    fn centimetres_are_scaled() {
        assert!((to_metres(250.0, Units::Cm).expect("finite value") - 2.5).abs() < 1e-12);
    }

    #[test]
    fn millimetres_are_scaled() {
        assert!((to_metres(2500.0, Units::Mm).expect("finite value") - 2.5).abs() < 1e-12);
    }

    #[test]
    fn non_finite_length_is_rejected() {
        let error = to_metres(f64::INFINITY, Units::M).expect_err("infinity must be rejected");
        assert!(matches!(error, ReconError::NonFinite { .. }));
    }

    #[test]
    fn point_conversion_scales_every_axis() {
        let point = point_to_metres([100.0, -200.0, 300.0], Units::Cm).expect("finite point");
        assert!((point[0] - 1.0).abs() < 1e-12);
        assert!((point[1] + 2.0).abs() < 1e-12);
        assert!((point[2] - 3.0).abs() < 1e-12);
    }

    #[test]
    fn non_finite_point_component_is_rejected() {
        let error = point_to_metres([0.0, f64::NAN, 0.0], Units::M)
            .expect_err("a NaN component must be rejected");
        assert!(matches!(error, ReconError::NonFinite { .. }));
    }
}
