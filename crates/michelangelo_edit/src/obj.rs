//! A strict OBJ parser for the edit boundary.
//!
//! The parser accepts the subset of the Wavefront OBJ format that the
//! Michelangelo solver writes and that an edit candidate may carry:
//!
//! * `v x y z` — a vertex (an optional fourth coordinate is accepted and
//!   ignored, it must be finite and not zero),
//! * `f a b c ...` — a face with at least three vertices; a vertex reference
//!   may carry texture and normal indices (`v`, `v/vt`, `v//vn`, `v/vt/vn`),
//!   only the vertex index is used.
//!
//! The keywords `vn`, `vt`, `g`, `o`, `s`, `usemtl` and `mtllib`
//! carry no geometry and are ignored; comments (`#`) and blank lines are
//! ignored. Every other keyword is rejected: the parser never guesses what a
//! statement means.
//!
//! The parser rejects
//!
//! * non-finite coordinates (`GEOMETRY_INVALID`),
//! * vertex indices that do not exist (`GEOMETRY_INVALID`),
//! * faces with fewer than three vertices or a repeated vertex
//!   (`GEOMETRY_INVALID`),
//! * faces with a zero area (`GEOMETRY_INVALID`),
//! * every malformed statement (`OBJ_PARSE`).

use std::path::Path;

use michelangelo_input::sha256::sha256_hex;

/// The keywords that carry no geometry and are ignored by the parser.
const IGNORED_KEYWORDS: &[&str] = &["vn", "vt", "g", "o", "s", "usemtl", "mtllib"];

/// The smallest face area that is still considered a real surface.
///
/// The area is computed in the unit of the mesh (metres for the Michelangelo
/// packages), so the threshold is expressed in square metres.
const MIN_FACE_AREA: f64 = 1.0e-12;

/// The axis aligned bounding box of a mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bbox {
    /// The minimum corner.
    pub min: [f64; 3],
    /// The maximum corner.
    pub max: [f64; 3],
}

impl Bbox {
    /// An empty bounding box (the inverse of every point).
    pub fn empty() -> Self {
        Bbox {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }

    /// Grow the box so that it contains the given point.
    pub fn add_point(&mut self, point: [f64; 3]) {
        self.min = [
            self.min[0].min(point[0]),
            self.min[1].min(point[1]),
            self.min[2].min(point[2]),
        ];
        self.max = [
            self.max[0].max(point[0]),
            self.max[1].max(point[1]),
            self.max[2].max(point[2]),
        ];
    }

    /// True when the box contains at least one point.
    pub fn is_valid(&self) -> bool {
        self.min[0] <= self.max[0] && self.min[1] <= self.max[1] && self.min[2] <= self.max[2]
    }

    /// True when the box contains the other box.
    pub fn contains(&self, other: &Bbox) -> bool {
        if !self.is_valid() || !other.is_valid() {
            return false;
        }
        for axis in 0..3 {
            if other.min[axis] < self.min[axis] || other.max[axis] > self.max[axis] {
                return false;
            }
        }
        true
    }

    /// The smallest box that contains both boxes.
    pub fn union(&self, other: &Bbox) -> Bbox {
        let mut result = *self;
        for axis in 0..3 {
            if other.min[axis] < result.min[axis] {
                result.min[axis] = other.min[axis];
            }
            if other.max[axis] > result.max[axis] {
                result.max[axis] = other.max[axis];
            }
        }
        result
    }

    /// The centre of the box.
    pub fn center(&self) -> [f64; 3] {
        let mut center = [0.0; 3];
        for (axis, slot) in center.iter_mut().enumerate() {
            *slot = (self.min[axis] + self.max[axis]) / 2.0;
        }
        center
    }
}

/// A failure of the OBJ parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjError {
    /// The file is not a well formed OBJ document.
    Parse {
        /// The one-based line of the offending statement.
        line: usize,
        /// The human readable reason.
        message: String,
    },
    /// The file is well formed but does not describe a usable geometry.
    Geometry {
        /// The one-based line of the offending statement.
        line: usize,
        /// The human readable reason.
        message: String,
    },
}

impl ObjError {
    /// The diagnostic code of the failure.
    pub fn code(&self) -> &'static str {
        match self {
            ObjError::Parse { .. } => "OBJ_PARSE",
            ObjError::Geometry { .. } => "GEOMETRY_INVALID",
        }
    }

    /// The one-based line of the offending statement.
    pub fn line(&self) -> usize {
        match self {
            ObjError::Parse { line, .. } => *line,
            ObjError::Geometry { line, .. } => *line,
        }
    }

    /// The human readable reason.
    pub fn message(&self) -> &str {
        match self {
            ObjError::Parse { message, .. } => message,
            ObjError::Geometry { message, .. } => message,
        }
    }
}

impl std::fmt::Display for ObjError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "line {}: {} ({})",
            self.line(),
            self.message(),
            self.code()
        )
    }
}

impl std::error::Error for ObjError {}

/// A parsed OBJ mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjMesh {
    /// The vertices of the mesh.
    pub vertices: Vec<[f64; 3]>,
    /// The faces of the mesh; every face holds the zero based indices of its
    /// vertices.
    pub faces: Vec<Vec<usize>>,
    /// The bounding box of the vertices.
    pub bbox: Bbox,
}

impl ObjMesh {
    /// The number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// The number of faces.
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// The number of triangles the faces describe (an n-gon counts as n-2).
    pub fn triangle_count(&self) -> usize {
        self.faces
            .iter()
            .map(|face| face.len().saturating_sub(2))
            .sum::<usize>()
    }
}

/// A parsed OBJ file with the digest of its bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjFile {
    /// The parsed mesh.
    pub mesh: ObjMesh,
    /// The SHA-256 of the raw bytes of the file.
    pub sha256: String,
}

/// Parse an OBJ document.
pub fn parse_obj(text: &str) -> Result<ObjMesh, ObjError> {
    let mut vertices: Vec<[f64; 3]> = Vec::new();
    let mut faces: Vec<Vec<usize>> = Vec::new();
    let mut bbox = Bbox::empty();

    for (number, raw_line) in text.lines().enumerate() {
        let line = number + 1;
        let statement = raw_line.trim();
        if statement.is_empty() || statement.starts_with('#') {
            continue;
        }
        let (keyword, rest) = match statement.split_once(char::is_whitespace) {
            Some((keyword, rest)) => (keyword, rest.trim()),
            None => (statement, ""),
        };

        match keyword {
            "v" => {
                let point = parse_vertex(rest, line)?;
                bbox.add_point(point);
                vertices.push(point);
            }
            "f" => {
                let face = parse_face(rest, vertices.len(), line)?;
                if face_is_degenerate(&vertices, &face) {
                    return Err(ObjError::Geometry {
                        line,
                        message: "the face has a zero area".to_string(),
                    });
                }
                faces.push(face);
            }
            other if IGNORED_KEYWORDS.contains(&other) => {}
            other => {
                return Err(ObjError::Parse {
                    line,
                    message: format!("the keyword '{other}' is not supported"),
                })
            }
        }
    }

    if vertices.is_empty() {
        return Err(ObjError::Geometry {
            line: 0,
            message: "the mesh has no vertices".to_string(),
        });
    }
    if faces.is_empty() {
        return Err(ObjError::Geometry {
            line: 0,
            message: "the mesh has no faces".to_string(),
        });
    }

    Ok(ObjMesh {
        vertices,
        faces,
        bbox,
    })
}

/// Parse an OBJ file and compute the digest of its bytes.
pub fn parse_obj_file(path: &Path) -> Result<ObjFile, ObjError> {
    let bytes = std::fs::read(path).map_err(|err| ObjError::Parse {
        line: 0,
        message: format!("the file cannot be read: {err}"),
    })?;
    let text = String::from_utf8(bytes).map_err(|_| ObjError::Parse {
        line: 0,
        message: "the file is not valid UTF-8".to_string(),
    })?;
    let mesh = parse_obj(&text)?;
    Ok(ObjFile {
        mesh,
        sha256: sha256_hex(text.as_bytes()),
    })
}

/// Parse a vertex statement.
fn parse_vertex(rest: &str, line: usize) -> Result<[f64; 3], ObjError> {
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 3 {
        return Err(ObjError::Parse {
            line,
            message: format!("a vertex needs three coordinates, found {}", fields.len()),
        });
    }
    if fields.len() > 4 {
        return Err(ObjError::Parse {
            line,
            message: format!("a vertex needs at most four values, found {}", fields.len()),
        });
    }
    let mut point = [0.0; 3];
    for (axis, field) in fields.iter().take(3).enumerate() {
        let value: f64 = field.parse().map_err(|_| ObjError::Parse {
            line,
            message: format!("'{field}' is not a number"),
        })?;
        if !value.is_finite() {
            return Err(ObjError::Geometry {
                line,
                message: format!("the coordinate '{field}' is not finite"),
            });
        }
        point[axis] = value;
    }
    if let Some(field) = fields.get(3) {
        let weight: f64 = field.parse().map_err(|_| ObjError::Parse {
            line,
            message: format!("'{field}' is not a number"),
        })?;
        if !weight.is_finite() {
            return Err(ObjError::Geometry {
                line,
                message: format!("the weight '{field}' is not finite"),
            });
        }
        if weight == 0.0 {
            return Err(ObjError::Geometry {
                line,
                message: "the vertex weight must not be zero".to_string(),
            });
        }
    }
    Ok(point)
}

/// Parse a face statement.
fn parse_face(rest: &str, vertex_count: usize, line: usize) -> Result<Vec<usize>, ObjError> {
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 3 {
        return Err(ObjError::Parse {
            line,
            message: format!(
                "a face needs at least three vertices, found {}",
                fields.len()
            ),
        });
    }
    let mut face: Vec<usize> = Vec::with_capacity(fields.len());
    for field in &fields {
        let reference = field.split('/').next().unwrap_or("");
        if reference.is_empty() {
            return Err(ObjError::Parse {
                line,
                message: format!("'{field}' is not a vertex reference"),
            });
        }
        let raw: i64 = reference.parse().map_err(|_| ObjError::Parse {
            line,
            message: format!("'{reference}' is not a vertex index"),
        })?;
        if raw == 0 {
            return Err(ObjError::Parse {
                line,
                message: "the vertex index 0 does not exist".to_string(),
            });
        }
        let resolved = if raw < 0 {
            let offset = vertex_count as i64 + raw + 1;
            if offset < 1 {
                return Err(ObjError::Geometry {
                    line,
                    message: format!("the vertex index {raw} does not exist"),
                });
            }
            offset as usize
        } else {
            raw as usize
        };
        if resolved > vertex_count {
            return Err(ObjError::Geometry {
                line,
                message: format!("the vertex index {resolved} does not exist"),
            });
        }
        let index = resolved - 1;
        if face.contains(&index) {
            return Err(ObjError::Geometry {
                line,
                message: "the face repeats a vertex".to_string(),
            });
        }
        face.push(index);
    }
    Ok(face)
}

/// The area of a face in the unit of the mesh (Newell's method).
pub fn face_area(vertices: &[[f64; 3]], face: &[usize]) -> f64 {
    let mut normal = [0.0_f64; 3];
    for position in 0..face.len() {
        let current = vertices[face[position]];
        let next = vertices[face[(position + 1) % face.len()]];
        normal[0] += (current[1] - next[1]) * (current[2] + next[2]);
        normal[1] += (current[2] - next[2]) * (current[0] + next[0]);
        normal[2] += (current[0] - next[0]) * (current[1] + next[1]);
    }
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    length / 2.0
}

/// True when the face has a real area.
pub fn face_is_degenerate(vertices: &[[f64; 3]], face: &[usize]) -> bool {
    face_area(vertices, face) <= MIN_FACE_AREA
}
