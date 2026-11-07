//! Strict manifest types for the Michelangelo input package (contract v0).
//!
//! Every struct denies unknown fields: a manifest that carries a field that is
//! not part of contract v0 is rejected instead of being silently ignored.

use serde::{Deserialize, Serialize};

/// The only manifest schema accepted by this crate.
pub const MANIFEST_SCHEMA: &str = "michelangelo.input-package/0";

/// File name of the manifest inside a package directory.
pub const MANIFEST_FILE_NAME: &str = "package.json";

/// A complete input package manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputPackage {
    /// Schema identifier, must equal [MANIFEST_SCHEMA].
    pub schema: String,
    /// Stable identifier of the packaged object.
    pub package_id: String,
    /// Description of the packaged object.
    pub object: ObjectInfo,
    /// Revision number of the package.
    pub revision: u32,
    /// Unit of all lengths in the manifest.
    pub units: Units,
    /// Coordinate system convention of the package.
    pub coordinate_system: CoordinateSystem,
    /// Views (calibrated or reference).
    pub views: Vec<View>,
    /// Optional part decomposition.
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Optional authored supplements.
    #[serde(default)]
    pub supplements: Vec<Supplement>,
    /// Optional list of unknowns.
    #[serde(default)]
    pub unknowns: Vec<UnknownInfo>,
    /// Optional known dimensions.
    #[serde(default)]
    pub measurements: Vec<Measurement>,
    /// Optional uncertainty policy.
    #[serde(default)]
    pub uncertainty_policy: Option<UncertaintyPolicy>,
    /// Provenance of the package as a whole.
    pub provenance: Provenance,
    /// Acceptance decisions.
    #[serde(default)]
    pub acceptance: Vec<Acceptance>,
}

/// Description of the packaged object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectInfo {
    /// Human readable name, 1..=200 characters.
    pub name: String,
    /// Optional free-form notes, at most 2000 characters.
    #[serde(default)]
    pub notes: Option<String>,
}

/// Length unit of the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    /// Metres.
    #[serde(rename = "m")]
    M,
    /// Centimetres.
    #[serde(rename = "cm")]
    Cm,
    /// Millimetres.
    #[serde(rename = "mm")]
    Mm,
}

/// Handedness of the coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Handedness {
    /// Right handed coordinate systems only.
    #[serde(rename = "right")]
    Right,
}

/// A signed coordinate axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    /// +X.
    #[serde(rename = "+X")]
    PlusX,
    /// -X.
    #[serde(rename = "-X")]
    MinusX,
    /// +Y.
    #[serde(rename = "+Y")]
    PlusY,
    /// -Y.
    #[serde(rename = "-Y")]
    MinusY,
    /// +Z.
    #[serde(rename = "+Z")]
    PlusZ,
    /// -Z.
    #[serde(rename = "-Z")]
    MinusZ,
}

/// Coordinate system convention.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoordinateSystem {
    /// Handedness of the system.
    pub handedness: Handedness,
    /// Up axis.
    pub up: Axis,
    /// Right axis.
    pub right: Axis,
    /// Forward axis.
    pub forward: Axis,
}

/// Kind of a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewKind {
    /// Usable as a metric constraint.
    #[serde(rename = "calibrated")]
    Calibrated,
    /// Visual reference only.
    #[serde(rename = "reference")]
    Reference,
}

/// A single view of the object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// Unique identifier of the view.
    pub view_id: String,
    /// Calibrated or reference.
    pub kind: ViewKind,
    /// The rendered image.
    pub image: ImageResource,
    /// The object mask of the image.
    pub mask: MaskResource,
    /// Camera projection; required for calibrated views.
    #[serde(default)]
    pub projection: Option<Projection>,
    /// Optional provenance of the view.
    #[serde(default)]
    pub provenance: Option<Provenance>,
}

/// A PNG image resource of a view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageResource {
    /// Package relative path of the PNG file.
    pub path: String,
    /// Lowercase hex SHA-256 of the file content.
    pub sha256: String,
    /// Image width in pixels, must match the PNG IHDR.
    pub width_px: u32,
    /// Image height in pixels, must match the PNG IHDR.
    pub height_px: u32,
}

/// Mask encoding of a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskFormat {
    /// 8 bit grayscale PNG.
    #[serde(rename = "grayscale8")]
    Grayscale8,
    /// RGBA PNG whose alpha channel carries the mask.
    #[serde(rename = "rgba8")]
    Rgba8,
}

/// Foreground convention of a mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskForeground {
    /// High luminance means object.
    #[serde(rename = "white")]
    White,
    /// High alpha means object.
    #[serde(rename = "alpha")]
    Alpha,
}

/// A PNG mask resource of a view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskResource {
    /// Package relative path of the PNG file.
    pub path: String,
    /// Lowercase hex SHA-256 of the file content.
    pub sha256: String,
    /// Mask encoding.
    pub format: MaskFormat,
    /// Foreground convention.
    pub foreground: MaskForeground,
}

/// Projection type of a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionType {
    /// Orthographic projection.
    #[serde(rename = "orthographic")]
    Orthographic,
}

/// Camera projection of a view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    /// Projection type, always orthographic in v0.
    #[serde(rename = "type")]
    pub projection_type: ProjectionType,
    /// Camera location in package units.
    pub location_m: [f64; 3],
    /// Point the camera looks at, in package units.
    pub look_at_m: [f64; 3],
    /// World space up vector of the camera.
    pub up_world: [f64; 3],
    /// Visible world width of the image.
    pub ortho_scale_m: f64,
    /// Horizontal lens shift, default 0.
    #[serde(default)]
    pub shift_x: f64,
    /// Vertical lens shift, default 0.
    #[serde(default)]
    pub shift_y: f64,
    /// Near clip distance, default 0.1.
    #[serde(default = "default_clip_start")]
    pub clip_start_m: f64,
    /// Far clip distance, default 1000.
    #[serde(default = "default_clip_end")]
    pub clip_end_m: f64,
}

fn default_clip_start() -> f64 {
    0.1
}

fn default_clip_end() -> f64 {
    1000.0
}

/// Origin of a package or view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenanceOrigin {
    /// Authored by a human.
    #[serde(rename = "human")]
    Human,
    /// Authored by an AI system.
    #[serde(rename = "ai_author")]
    AiAuthor,
    /// Mixed human and AI authorship.
    #[serde(rename = "mixed")]
    Mixed,
}

/// Provenance of a package or a view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// Who authored the content.
    pub origin: ProvenanceOrigin,
    /// Optional free-form detail.
    #[serde(default)]
    pub detail: Option<String>,
}

/// Origin of a supplement or a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorOrigin {
    /// Authored by a human.
    #[serde(rename = "human")]
    Human,
    /// Authored by an AI system.
    #[serde(rename = "ai_author")]
    AiAuthor,
    /// Computed from the package.
    #[serde(rename = "computed")]
    Computed,
}

/// Provenance of a supplement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorProvenance {
    /// Who authored the supplement.
    pub origin: AuthorOrigin,
    /// Optional free-form detail.
    #[serde(default)]
    pub detail: Option<String>,
}

/// A part of the object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// Unique identifier of the part.
    pub part_id: String,
    /// Human readable label.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional parent part identifier.
    #[serde(default)]
    pub parent: Option<String>,
    /// Whether the part must not be modified.
    #[serde(default)]
    pub protected: bool,
}

/// Kind of an authored supplement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupplementKind {
    /// Wall thickness.
    #[serde(rename = "thickness")]
    Thickness,
    /// Curve geometry.
    #[serde(rename = "curve")]
    Curve,
    /// Cross section.
    #[serde(rename = "section")]
    Section,
    /// Profile geometry.
    #[serde(rename = "profile")]
    Profile,
    /// Closure of an opening.
    #[serde(rename = "closure")]
    Closure,
    /// Cavity geometry.
    #[serde(rename = "cavity")]
    Cavity,
    /// Connection between two points.
    #[serde(rename = "connection")]
    Connection,
    /// Symmetry axis.
    #[serde(rename = "symmetry")]
    Symmetry,
}

/// An authored completion of hidden shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Supplement {
    /// Unique identifier of the supplement.
    pub supplement_id: String,
    /// Part the supplement belongs to.
    pub part_id: String,
    /// Kind of the supplement.
    pub kind: SupplementKind,
    /// Geometry payload, a non-empty JSON object.
    pub geometry: serde_json::Value,
    /// Provenance of the supplement.
    pub provenance: AuthorProvenance,
}

/// Policy for an unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnknownPolicy {
    /// Only a human may resolve the unknown.
    #[serde(rename = "author_required")]
    AuthorRequired,
    /// A deterministic rule may resolve the unknown.
    #[serde(rename = "rule_allowed")]
    RuleAllowed,
    /// A geometric stub may resolve the unknown.
    #[serde(rename = "stub_allowed")]
    StubAllowed,
}

/// An explicitly unknown aspect of the object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownInfo {
    /// Unique identifier of the unknown.
    pub id: String,
    /// Human readable description.
    pub description: String,
    /// Parts affected by the unknown.
    #[serde(default)]
    pub affected_parts: Vec<String>,
    /// Optional resolution policy.
    #[serde(default)]
    pub policy: Option<UnknownPolicy>,
}

/// Kind of a known dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasurementKind {
    /// Overall height.
    #[serde(rename = "height")]
    Height,
    /// Overall width.
    #[serde(rename = "width")]
    Width,
    /// Overall depth.
    #[serde(rename = "depth")]
    Depth,
    /// Distance between two points.
    #[serde(rename = "distance")]
    Distance,
    /// Bounding box dimension.
    #[serde(rename = "bbox")]
    Bbox,
}

/// A known or declared dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    /// Unique identifier of the measurement.
    pub id: String,
    /// Human readable label.
    #[serde(default)]
    pub label: Option<String>,
    /// Kind of the dimension.
    pub kind: MeasurementKind,
    /// Value in package units.
    pub value_m: f64,
    /// Tolerance in package units, default 0.
    #[serde(default)]
    pub tolerance_m: Option<f64>,
    /// Who authored the measurement.
    pub origin: AuthorOrigin,
}

/// Policy that describes how uncertainty may be resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UncertaintyPolicy {
    /// Completion rules the package allows.
    #[serde(default)]
    pub allowed_completion_rules: Vec<String>,
    /// Whether stubs may be used.
    pub stub_policy: StubPolicy,
    /// Whether an author acceptance is required, default true.
    #[serde(default = "default_true")]
    pub require_author_acceptance: bool,
}

fn default_true() -> bool {
    true
}

/// Policy for geometric stubs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StubPolicy {
    /// Stubs are allowed.
    #[serde(rename = "allowed")]
    Allowed,
    /// Stubs are forbidden.
    #[serde(rename = "forbidden")]
    Forbidden,
}

/// Status of an acceptance decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcceptanceStatus {
    /// The target was proposed.
    #[serde(rename = "proposed")]
    Proposed,
    /// The target was accepted.
    #[serde(rename = "accepted")]
    Accepted,
    /// The target needs revision.
    #[serde(rename = "needs_revision")]
    NeedsRevision,
}

/// An acceptance decision for one package element.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acceptance {
    /// Target reference, for example "parts:lid".
    pub target: String,
    /// Status of the decision.
    pub status: AcceptanceStatus,
    /// Who made the decision.
    pub by: String,
    /// RFC 3339 timestamp of the decision.
    pub at: String,
}
