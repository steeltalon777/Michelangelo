//! Camera basis and Blender Euler-XYZ conversion.
//!
//! The calibrated orthographic views of an input package carry a location, a
//! look-at point and a world up vector. Blender expects an object rotation, so
//! the pipeline builds the camera basis itself and converts it into the
//! Euler-XYZ angles that `create_ortho_camera` takes.
//!
//! # Convention (verified against Blender 5.2.3)
//!
//! * `direction = normalize(look_at - location)` — the camera looks along
//!   `direction`,
//! * `right = normalize(cross(direction, up_world))`,
//! * `camera_up = normalize(cross(right, direction))`,
//! * the rotation matrix `R` maps camera-local coordinates to world
//!   coordinates and has the **columns** `(right, camera_up, -direction)`,
//! * the Blender Euler angles (XYZ order, degrees) are extracted with
//!   `y = asin(-R[2][0])`, `x = atan2(R[2][1], R[2][2])`,
//!   `z = atan2(R[1][0], R[0][0])`.
//!
//! Verified reference poses:
//!
//! | view | location | look_at | up_world | rotation_deg |
//! |---|---|---|---|---|
//! | front | (0, -4, 0) | origin | +Z | (90, 0, 0) |
//! | right | (4, 0, 0) | origin | +Z | (90, 0, 90) |
//! | top | (0, 0, 4) | origin | +Y | (0, 0, 0) |
//!
//! A transposed matrix (row instead of column convention) produces empty
//! renders, so the column convention is covered by an explicit test.

use std::fmt;

/// Tolerance used for degeneracy checks of the camera basis.
pub const EPSILON: f64 = 1e-9;

/// Errors of the camera math.
#[derive(Debug, Clone, PartialEq)]
pub enum CameraError {
    /// One of the inputs is not a finite number.
    NonFinite {
        /// The field that carries the value.
        field: &'static str,
    },
    /// The basis is degenerate (zero length or parallel vectors).
    Degenerate {
        /// What went wrong.
        message: String,
    },
    /// The rotation cannot be represented as Euler XYZ angles.
    GimbalLock,
}

impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { field } => write!(f, "'{field}' is not a finite number"),
            Self::Degenerate { message } => write!(f, "{message}"),
            Self::GimbalLock => write!(
                f,
                "the rotation is gimbal-locked and has no unique Euler XYZ"
            ),
        }
    }
}

impl std::error::Error for CameraError {}

/// Dot product of two 3D vectors.
pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Cross product of two 3D vectors.
pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Length of a 3D vector.
pub fn length(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

/// Normalize a 3D vector.
pub fn normalize(v: [f64; 3]) -> Result<[f64; 3], CameraError> {
    if !v.iter().all(|value| value.is_finite()) {
        return Err(CameraError::NonFinite { field: "vector" });
    }
    let len = length(v);
    if len < EPSILON {
        return Err(CameraError::Degenerate {
            message: "cannot normalize a zero-length vector".to_string(),
        });
    }
    Ok([v[0] / len, v[1] / len, v[2] / len])
}

/// The orthonormal camera basis of a calibrated view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraBasis {
    /// The unit vector the camera looks along (camera-local -Z).
    pub direction: [f64; 3],
    /// The unit vector of the camera's right axis (camera-local +X).
    pub right: [f64; 3],
    /// The unit vector of the camera's up axis (camera-local +Y).
    pub up: [f64; 3],
}

/// Build the camera basis of a calibrated orthographic view.
///
/// The location and the look-at point must differ and the world up vector
/// must not be parallel to the view direction.
pub fn camera_basis(
    location: &[f64; 3],
    look_at: &[f64; 3],
    up_world: &[f64; 3],
) -> Result<CameraBasis, CameraError> {
    for (field, value) in [
        ("location", location),
        ("look_at", look_at),
        ("up_world", up_world),
    ] {
        if !value.iter().all(|component| component.is_finite()) {
            return Err(CameraError::NonFinite { field });
        }
    }

    let delta = [
        look_at[0] - location[0],
        look_at[1] - location[1],
        look_at[2] - location[2],
    ];
    if length(delta) < EPSILON {
        return Err(CameraError::Degenerate {
            message: "the look-at point coincides with the camera location".to_string(),
        });
    }
    let direction = normalize(delta)?;

    let up = normalize(*up_world)?;
    if dot(direction, up).abs() > 0.999 {
        return Err(CameraError::Degenerate {
            message: "the world up vector is parallel to the view direction".to_string(),
        });
    }

    let right = normalize(cross(direction, up))?;
    let camera_up = normalize(cross(right, direction))?;

    Ok(CameraBasis {
        direction,
        right,
        up: camera_up,
    })
}

/// The camera-local to world rotation matrix of a basis.
///
/// The matrix has the **columns** `(right, camera_up, -direction)`.
pub fn basis_matrix(basis: &CameraBasis) -> [[f64; 3]; 3] {
    [
        [basis.right[0], basis.up[0], -basis.direction[0]],
        [basis.right[1], basis.up[1], -basis.direction[1]],
        [basis.right[2], basis.up[2], -basis.direction[2]],
    ]
}

/// Extract the Blender Euler XYZ angles (degrees) from a rotation matrix.
pub fn euler_xyz_degrees(matrix: &[[f64; 3]; 3]) -> Result<[f64; 3], CameraError> {
    let r20 = matrix[2][0];
    if !r20.is_finite() {
        return Err(CameraError::NonFinite { field: "rotation" });
    }
    if r20.abs() > 1.0 - 1e-9 {
        return Err(CameraError::GimbalLock);
    }
    let y = (-r20).clamp(-1.0, 1.0).asin();
    let x = matrix[2][1].atan2(matrix[2][2]);
    let z = matrix[1][0].atan2(matrix[0][0]);
    Ok([x.to_degrees(), y.to_degrees(), z.to_degrees()])
}

/// Build a rotation matrix from Blender Euler XYZ angles (degrees).
pub fn matrix_from_euler_xyz_degrees(degrees: [f64; 3]) -> [[f64; 3]; 3] {
    let (x, y, z) = (
        degrees[0].to_radians(),
        degrees[1].to_radians(),
        degrees[2].to_radians(),
    );
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();

    // R = Rz * Ry * Rx (Blender's XYZ Euler order).
    [
        [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
        [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
        [-sy, cy * sx, cy * cx],
    ]
}

/// The Blender Euler XYZ angles (degrees) of a calibrated view.
pub fn rotation_degrees(
    location: &[f64; 3],
    look_at: &[f64; 3],
    up_world: &[f64; 3],
) -> Result<[f64; 3], CameraError> {
    let basis = camera_basis(location, look_at, up_world)?;
    euler_xyz_degrees(&basis_matrix(&basis))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
        for (index, value) in actual.iter().enumerate() {
            assert!(
                (value - expected[index]).abs() < tolerance,
                "expected {expected:?}, got {actual:?}"
            );
        }
    }

    #[test]
    fn front_view_matches_reference_rotation() {
        let rotation = rotation_degrees(&[0.0, -4.0, 0.0], &[0.0, 0.0, 0.0], &[0.0, 0.0, 1.0])
            .expect("front view is valid");
        assert_close(rotation, [90.0, 0.0, 0.0], 1e-6);
    }

    #[test]
    fn right_view_matches_reference_rotation() {
        let rotation = rotation_degrees(&[4.0, 0.0, 0.0], &[0.0, 0.0, 0.0], &[0.0, 0.0, 1.0])
            .expect("right view is valid");
        assert_close(rotation, [90.0, 0.0, 90.0], 1e-6);
    }

    #[test]
    fn top_view_matches_reference_rotation() {
        let rotation = rotation_degrees(&[0.0, 0.0, 4.0], &[0.0, 0.0, 0.0], &[0.0, 1.0, 0.0])
            .expect("top view is valid");
        assert_close(rotation, [0.0, 0.0, 0.0], 1e-6);
    }

    #[test]
    fn oblique_view_rotation_is_consistent_with_the_matrix() {
        let location = [1.5, -2.0, 3.0];
        let look_at = [0.0, 0.0, 0.5];
        let up_world = [0.0, 0.0, 1.0];
        let basis = camera_basis(&location, &look_at, &up_world).expect("oblique view is valid");
        let matrix = basis_matrix(&basis);
        let rotation = euler_xyz_degrees(&matrix).expect("oblique view is not gimbal-locked");
        let rebuilt = matrix_from_euler_xyz_degrees(rotation);
        for row in 0..3 {
            for column in 0..3 {
                assert!(
                    (rebuilt[row][column] - matrix[row][column]).abs() < 1e-9,
                    "matrix mismatch at [{row}][{column}]"
                );
            }
        }
    }

    #[test]
    fn basis_matrix_uses_columns_not_rows() {
        let basis = camera_basis(&[0.0, -4.0, 0.0], &[0.0, 0.0, 0.0], &[0.0, 0.0, 1.0])
            .expect("front view is valid");
        let matrix = basis_matrix(&basis);
        assert_close(matrix[0], [1.0, 0.0, 0.0], 1e-12);
        assert_close(matrix[1], [0.0, 0.0, -1.0], 1e-12);
        assert_close(matrix[2], [0.0, 1.0, 0.0], 1e-12);
        // The transposed matrix would be the row convention and is wrong.
        assert!((matrix[1][2] - matrix[2][1]).abs() > 1e-6);
    }

    #[test]
    fn basis_is_orthonormal() {
        let basis = camera_basis(&[2.0, 1.0, -3.0], &[0.0, 0.0, 0.0], &[0.0, 0.0, 1.0])
            .expect("basis is valid");
        for vector in [basis.direction, basis.right, basis.up] {
            assert!((length(vector) - 1.0).abs() < 1e-12);
        }
        assert!(dot(basis.right, basis.up).abs() < 1e-12);
        assert!(dot(basis.right, basis.direction).abs() < 1e-12);
        assert!(dot(basis.up, basis.direction).abs() < 1e-12);
    }

    #[test]
    fn look_at_equal_to_location_is_rejected() {
        let error = rotation_degrees(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], &[0.0, 0.0, 1.0])
            .expect_err("a degenerate view must be rejected");
        assert!(matches!(error, CameraError::Degenerate { .. }));
    }

    #[test]
    fn parallel_up_vector_is_rejected() {
        let error = rotation_degrees(&[0.0, 0.0, 4.0], &[0.0, 0.0, 0.0], &[0.0, 0.0, 1.0])
            .expect_err("a parallel up vector must be rejected");
        assert!(matches!(error, CameraError::Degenerate { .. }));
    }

    #[test]
    fn non_finite_input_is_rejected() {
        let error = rotation_degrees(&[f64::NAN, 0.0, 0.0], &[0.0, 0.0, 0.0], &[0.0, 0.0, 1.0])
            .expect_err("a non-finite location must be rejected");
        assert!(matches!(error, CameraError::NonFinite { .. }));
    }

    #[test]
    fn gimbal_lock_is_reported() {
        let matrix = matrix_from_euler_xyz_degrees([0.0, 90.0, 0.0]);
        let error = euler_xyz_degrees(&matrix).expect_err("gimbal lock must be reported");
        assert_eq!(error, CameraError::GimbalLock);
    }

    #[test]
    fn euler_matrix_round_trip_is_stable() {
        for degrees in [
            [90.0, 0.0, 0.0],
            [90.0, 0.0, 90.0],
            [0.0, 0.0, 0.0],
            [35.0, 20.0, -50.0],
        ] {
            let matrix = matrix_from_euler_xyz_degrees(degrees);
            let extracted = euler_xyz_degrees(&matrix).expect("no gimbal lock");
            assert_close(extracted, degrees, 1e-9);
        }
    }
}
