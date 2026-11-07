//! Level 1 (format / safety) and level 2 (sufficiency) validation of an input package.
//!
//! The validator never panics: every failure is turned into a diagnostic in
//! the report. Resource files are only opened after the path has been proven
//! to stay inside the canonicalized package directory.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::{
    AcceptanceStatus, AuthorOrigin, InputPackage, MaskForeground, MaskFormat, Projection,
    SupplementKind, ViewKind, MANIFEST_FILE_NAME, MANIFEST_SCHEMA,
};
use crate::png::{self, PngHeader};
use crate::report::{
    Completeness, CompletenessStatus, Diagnostic, InputPackageReport, LevelSummary, ReportStatus,
    ResourceReport, Severity, ValidationLevels, REPORT_SCHEMA,
};
use crate::sha256::sha256_hex;

/// Maximum size of the manifest file (1 MiB).
pub const MAX_MANIFEST_BYTES: u64 = 1_048_576;
/// Maximum size of one referenced resource (64 MiB).
pub const MAX_RESOURCE_BYTES: u64 = 67_108_864;
/// Maximum total size of all referenced resources (512 MiB).
pub const MAX_TOTAL_RESOURCE_BYTES: u64 = 536_870_912;
/// Maximum length of a resource path in characters.
pub const MAX_PATH_CHARS: usize = 1024;
/// Maximum image dimension in pixels.
pub const MAX_DIMENSION_PX: u32 = 16_384;
/// Maximum number of views in one package.
pub const MAX_VIEWS: usize = 64;
/// Maximum number of parts in one package.
pub const MAX_PARTS: usize = 256;
/// Maximum number of supplements in one package.
pub const MAX_SUPPLEMENTS: usize = 256;
/// Maximum number of unknowns in one package.
pub const MAX_UNKNOWNS: usize = 256;
/// Maximum number of measurements in one package.
pub const MAX_MEASUREMENTS: usize = 64;
/// Maximum number of acceptance entries in one package.
pub const MAX_ACCEPTANCE: usize = 512;
/// Maximum number of completion rules in the uncertainty policy.
pub const MAX_COMPLETION_RULES: usize = 8;
/// Maximum nesting depth of supplement geometry.
pub const MAX_GEOMETRY_DEPTH: usize = 8;
/// Maximum serialized size of supplement geometry (64 KiB).
pub const MAX_GEOMETRY_BYTES: usize = 65_536;
/// Absolute bound for a single coordinate or length component.
pub const MAX_COORDINATE_ABS: f64 = 1_000_000.0;
/// Absolute bound for the lens shift components.
pub const MAX_SHIFT_ABS: f64 = 10.0;
/// Highest accepted revision number.
pub const MAX_REVISION: u32 = 1_000_000;
/// Two directions with a larger absolute dot product are duplicates.
pub const DUPLICATE_DIRECTION_DOT: f64 = 0.999;
/// Two directions with a larger absolute dot product are not independent.
pub const DISTINCT_DIRECTION_DOT: f64 = 0.9;

/// Options that steer the validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidateOptions {
    /// Treat an incomplete package as an error.
    pub require_complete: bool,
    /// Verify resource hashes (a mismatch is an error when enabled).
    pub verify_hashes: bool,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        ValidateOptions {
            require_complete: false,
            verify_hashes: true,
        }
    }
}

/// Validate an input package directory and return the report.
///
/// The function never panics. A missing or unreadable manifest produces an
/// invalid report with a diagnostic instead of an error value.
pub fn validate_package(package_root: &Path, options: &ValidateOptions) -> InputPackageReport {
    let mut validator = Validator::new(package_root, options);
    validator.run();
    validator.finish()
}

/// Load and strictly deserialize the manifest of a package directory.
pub fn load_manifest(package_root: &Path) -> Result<InputPackage, crate::error::InputError> {
    use crate::error::InputError;

    let root = fs::canonicalize(package_root).map_err(|err| InputError::PackageRoot {
        path: package_root.to_path_buf(),
        message: err.to_string(),
    })?;
    let manifest_path = root.join(MANIFEST_FILE_NAME);
    let meta = fs::metadata(&manifest_path).map_err(|err| InputError::io(&manifest_path, err))?;
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err(InputError::ManifestTooLarge {
            path: manifest_path,
            bytes: meta.len(),
        });
    }
    let bytes = fs::read(&manifest_path).map_err(|err| InputError::io(&manifest_path, err))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|err| {
        InputError::invalid(&manifest_path, format!("manifest is not valid JSON: {err}"))
    })?;
    serde_json::from_value(value)
        .map_err(|err| InputError::invalid(&manifest_path, err.to_string()))
}

enum PathIssue {
    Invalid(String),
    Outside(String),
}

enum ResourceKind {
    Image,
    Mask,
}

struct ResourceOutcome {
    ok: bool,
    header: Option<PngHeader>,
}

struct Validator<'a> {
    root: PathBuf,
    options: &'a ValidateOptions,
    canonical_root: Option<PathBuf>,
    diagnostics: Vec<Diagnostic>,
    resources: Vec<ResourceReport>,
    package_id: String,
    revision: u32,
    total_exceeded: bool,
    directions: Vec<(usize, [f64; 3])>,
    completeness: Completeness,
    manifest_mtime: Option<SystemTime>,
    max_resource_mtime: Option<SystemTime>,
}

impl<'a> Validator<'a> {
    fn new(root: &Path, options: &'a ValidateOptions) -> Self {
        Validator {
            root: root.to_path_buf(),
            options,
            canonical_root: None,
            diagnostics: Vec::new(),
            resources: Vec::new(),
            package_id: String::new(),
            revision: 0,
            total_exceeded: false,
            directions: Vec::new(),
            completeness: Completeness {
                status: CompletenessStatus::Partial,
                calibrated_views: 0,
                distinct_directions: 0,
                axes_covered: Vec::new(),
                missing: Vec::new(),
            },
            manifest_mtime: None,
            max_resource_mtime: None,
        }
    }

    fn push_diag(
        &mut self,
        level: u8,
        code: &str,
        severity: Severity,
        address: &str,
        message: String,
    ) {
        self.diagnostics.push(Diagnostic {
            level,
            code: code.to_string(),
            severity,
            address: address.to_string(),
            message,
        });
    }

    fn error(&mut self, level: u8, code: &str, address: &str, message: String) {
        self.push_diag(level, code, Severity::Error, address, message);
    }

    fn warning(&mut self, level: u8, code: &str, address: &str, message: String) {
        self.push_diag(level, code, Severity::Warning, address, message);
    }

    fn run(&mut self) {
        let manifest_path = self.root.join(MANIFEST_FILE_NAME);
        match fs::canonicalize(&self.root) {
            Ok(path) => self.canonical_root = Some(path),
            Err(err) => {
                self.error(
                    1,
                    "RESOURCE_MISSING",
                    "package",
                    format!("package directory cannot be resolved: {err}"),
                );
                return;
            }
        }

        let meta = match fs::metadata(&manifest_path) {
            Ok(meta) => meta,
            Err(err) => {
                self.error(
                    1,
                    "RESOURCE_MISSING",
                    MANIFEST_FILE_NAME,
                    format!("manifest cannot be read: {err}"),
                );
                return;
            }
        };
        if meta.len() > MAX_MANIFEST_BYTES {
            self.error(
                1,
                "MANIFEST_TOO_LARGE",
                MANIFEST_FILE_NAME,
                format!(
                    "manifest is {} bytes, the limit is {MAX_MANIFEST_BYTES} bytes",
                    meta.len()
                ),
            );
            return;
        }
        self.manifest_mtime = meta.modified().ok();

        let bytes = match fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.error(
                    1,
                    "RESOURCE_MISSING",
                    MANIFEST_FILE_NAME,
                    format!("manifest cannot be read: {err}"),
                );
                return;
            }
        };

        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(err) => {
                self.error(
                    1,
                    "SCHEMA_TYPE",
                    MANIFEST_FILE_NAME,
                    format!("manifest is not valid JSON: {err}"),
                );
                return;
            }
        };

        let package: InputPackage = match serde_json::from_value(value) {
            Ok(package) => package,
            Err(err) => {
                let message = err.to_string();
                let code = classify_serde_error(&message);
                let address =
                    serde_error_address(&message).unwrap_or_else(|| MANIFEST_FILE_NAME.to_string());
                self.error(1, code, &address, message);
                return;
            }
        };

        self.package_id = package.package_id.clone();
        self.revision = package.revision;
        self.validate_manifest(&package);
    }

    fn validate_manifest(&mut self, package: &InputPackage) {
        self.validate_header(package);
        self.validate_object(package);
        self.validate_coordinate_system(package);
        self.validate_parts(package);
        self.validate_supplements(package);
        self.validate_unknowns(package);
        self.validate_measurements(package);
        self.validate_uncertainty_policy(package);
        self.validate_views(package);
        self.validate_acceptance(package);
        self.validate_sufficiency(package);
    }

    fn validate_header(&mut self, package: &InputPackage) {
        if package.schema != MANIFEST_SCHEMA {
            self.error(
                1,
                "SCHEMA_ENUM",
                "schema",
                format!(
                    "schema must be '{MANIFEST_SCHEMA}', found '{}'",
                    package.schema
                ),
            );
        }
        if !is_valid_package_id(&package.package_id) {
            self.error(
                1,
                "ID_INVALID",
                "package_id",
                "package_id must match ^[a-z0-9][a-z0-9._-]{0,63}$".to_string(),
            );
        }
        if package.revision == 0 || package.revision > MAX_REVISION {
            self.error(
                1,
                "SCHEMA_RANGE",
                "revision",
                format!("revision must be in 1..={MAX_REVISION}"),
            );
        }
        if let Some(detail) = &package.provenance.detail {
            if detail.chars().count() > 2000 {
                self.error(
                    1,
                    "SCHEMA_RANGE",
                    "provenance.detail",
                    "provenance.detail must be at most 2000 characters".to_string(),
                );
            }
        }
    }

    fn validate_object(&mut self, package: &InputPackage) {
        let name_len = package.object.name.chars().count();
        if name_len == 0 || name_len > 200 {
            self.error(
                1,
                "SCHEMA_RANGE",
                "object.name",
                "object.name must be 1..=200 characters".to_string(),
            );
        }
        if let Some(notes) = &package.object.notes {
            if notes.chars().count() > 2000 {
                self.error(
                    1,
                    "SCHEMA_RANGE",
                    "object.notes",
                    "object.notes must be at most 2000 characters".to_string(),
                );
            }
        }
    }

    fn validate_coordinate_system(&mut self, package: &InputPackage) {
        let cs = &package.coordinate_system;
        if cs.up == cs.right || cs.up == cs.forward || cs.right == cs.forward {
            self.error(
                1,
                "SCHEMA_RANGE",
                "coordinate_system",
                "up, right and forward must be pairwise distinct axes".to_string(),
            );
        }
    }

    fn validate_parts(&mut self, package: &InputPackage) {
        if package.parts.len() > MAX_PARTS {
            self.error(
                1,
                "SCHEMA_RANGE",
                "parts",
                format!("at most {MAX_PARTS} parts are allowed"),
            );
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for (index, part) in package.parts.iter().enumerate() {
            let address = format!("parts[{index}]");
            if !is_valid_part_token(&part.part_id) {
                self.error(
                    1,
                    "ID_INVALID",
                    &format!("{address}.part_id"),
                    "part_id must match ^[A-Za-z0-9][A-Za-z0-9 _.-]{0,63}$".to_string(),
                );
            }
            if !seen.insert(part.part_id.as_str()) {
                self.error(
                    1,
                    "ID_DUPLICATE",
                    &format!("{address}.part_id"),
                    format!("duplicate part_id '{}'", part.part_id),
                );
            }
            if let Some(label) = &part.label {
                if !is_valid_part_token(label) {
                    self.error(
                        1,
                        "ID_INVALID",
                        &format!("{address}.label"),
                        "label must match ^[A-Za-z0-9][A-Za-z0-9 _.-]{0,63}$".to_string(),
                    );
                }
            }
        }
        for (index, part) in package.parts.iter().enumerate() {
            if let Some(parent) = &part.parent {
                if !package.parts.iter().any(|p| p.part_id == *parent) {
                    self.error(
                        1,
                        "REFERENCE_UNKNOWN",
                        &format!("parts[{index}].parent"),
                        format!("parent part '{parent}' does not exist"),
                    );
                }
            }
        }
        for (index, part) in package.parts.iter().enumerate() {
            let mut visited: HashSet<&str> = HashSet::new();
            let mut current = part.parent.as_deref();
            while let Some(id) = current {
                if !visited.insert(id) {
                    self.error(
                        1,
                        "PARENT_CYCLE",
                        &format!("parts[{index}].parent"),
                        format!("part '{}' takes part in a parent cycle", part.part_id),
                    );
                    break;
                }
                current = package
                    .parts
                    .iter()
                    .find(|p| p.part_id == id)
                    .and_then(|p| p.parent.as_deref());
            }
        }
    }

    fn validate_supplements(&mut self, package: &InputPackage) {
        if package.supplements.len() > MAX_SUPPLEMENTS {
            self.error(
                1,
                "SCHEMA_RANGE",
                "supplements",
                format!("at most {MAX_SUPPLEMENTS} supplements are allowed"),
            );
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for (index, supplement) in package.supplements.iter().enumerate() {
            let address = format!("supplements[{index}]");
            if supplement.supplement_id.is_empty() {
                self.error(
                    1,
                    "ID_INVALID",
                    &format!("{address}.supplement_id"),
                    "supplement_id must not be empty".to_string(),
                );
            }
            if !seen.insert(supplement.supplement_id.as_str()) {
                self.error(
                    1,
                    "ID_DUPLICATE",
                    &format!("{address}.supplement_id"),
                    format!("duplicate supplement_id '{}'", supplement.supplement_id),
                );
            }
            if !package
                .parts
                .iter()
                .any(|p| p.part_id == supplement.part_id)
            {
                self.error(
                    1,
                    "SUPPLEMENT_PART_UNKNOWN",
                    &format!("{address}.part_id"),
                    format!("part '{}' does not exist", supplement.part_id),
                );
            }
            self.validate_geometry(&address, supplement);
        }
    }

    fn validate_geometry(&mut self, address: &str, supplement: &crate::manifest::Supplement) {
        let geometry = &supplement.geometry;
        let object = match geometry.as_object() {
            Some(object) => object,
            None => {
                self.error(
                    1,
                    "GEOMETRY_INVALID",
                    &format!("{address}.geometry"),
                    "geometry must be a JSON object".to_string(),
                );
                return;
            }
        };
        if object.is_empty() {
            self.error(
                1,
                "GEOMETRY_INVALID",
                &format!("{address}.geometry"),
                "geometry must not be empty".to_string(),
            );
            return;
        }
        let serialized = geometry.to_string();
        if serialized.len() > MAX_GEOMETRY_BYTES {
            self.error(
                1,
                "GEOMETRY_INVALID",
                &format!("{address}.geometry"),
                format!(
                    "geometry is {} bytes, the limit is {MAX_GEOMETRY_BYTES} bytes",
                    serialized.len()
                ),
            );
            return;
        }
        if json_depth(geometry) > MAX_GEOMETRY_DEPTH {
            self.error(
                1,
                "GEOMETRY_INVALID",
                &format!("{address}.geometry"),
                format!("geometry nesting depth exceeds {MAX_GEOMETRY_DEPTH}"),
            );
            return;
        }
        if !json_numbers_in_range(geometry) {
            self.error(
                1,
                "GEOMETRY_INVALID",
                &format!("{address}.geometry"),
                format!("geometry numbers must be finite and within +-{MAX_COORDINATE_ABS}"),
            );
            return;
        }
        self.validate_supplement_geometry(address, supplement.kind, geometry);
    }

    fn validate_supplement_geometry(
        &mut self,
        address: &str,
        kind: SupplementKind,
        geometry: &serde_json::Value,
    ) {
        let object = match geometry.as_object() {
            Some(object) => object,
            None => return,
        };
        let geometry_address = format!("{address}.geometry");
        let mut reported = false;

        match kind {
            SupplementKind::Thickness => {
                let value = object.get("value_m").and_then(|v| v.as_f64());
                match value {
                    Some(value) if value > 0.0 => {}
                    _ => self.report_geometry(
                        &mut reported,
                        &geometry_address,
                        "a thickness supplement requires a positive 'value_m'".to_string(),
                    ),
                }
            }
            SupplementKind::Curve => match object.get("points_m").and_then(|v| v.as_array()) {
                Some(points) if (2..=256).contains(&points.len()) => {
                    for (index, point) in points.iter().enumerate() {
                        if !is_finite_point(point, 3) {
                            self.report_geometry(
                                &mut reported,
                                &geometry_address,
                                format!("points_m[{index}] must be a finite [x, y, z] point"),
                            );
                            break;
                        }
                    }
                }
                _ => self.report_geometry(
                    &mut reported,
                    &geometry_address,
                    "a curve supplement requires 'points_m' with 2..=256 points".to_string(),
                ),
            },
            SupplementKind::Section | SupplementKind::Profile => {
                match object.get("points_m").and_then(|v| v.as_array()) {
                    Some(points) if (2..=256).contains(&points.len()) => {
                        for (index, point) in points.iter().enumerate() {
                            if !is_finite_point(point, 2) && !is_finite_point(point, 3) {
                                self.report_geometry(
                                    &mut reported,
                                    &geometry_address,
                                    format!("points_m[{index}] must be a finite 2D or 3D point"),
                                );
                                break;
                            }
                        }
                    }
                    _ => self.report_geometry(
                        &mut reported,
                        &geometry_address,
                        "a section or profile supplement requires 'points_m' with 2..=256 points"
                            .to_string(),
                    ),
                }
                if let Some(plane) = object.get("plane") {
                    if !matches!(plane.as_str(), Some("xy") | Some("yz") | Some("xz")) {
                        self.report_geometry(
                            &mut reported,
                            &geometry_address,
                            "'plane' must be one of xy, yz, xz".to_string(),
                        );
                    }
                }
            }
            SupplementKind::Cavity => {
                let bbox_ok = object
                    .get("bbox_m")
                    .and_then(|v| v.as_array())
                    .map(|values| {
                        values.len() == 3
                            && values
                                .iter()
                                .all(|v| v.as_f64().map(|f| f > 0.0).unwrap_or(false))
                    })
                    .unwrap_or(false);
                let points_ok = object
                    .get("points_m")
                    .and_then(|v| v.as_array())
                    .map(|values| values.len() >= 4)
                    .unwrap_or(false);
                if !(bbox_ok || points_ok) {
                    self.report_geometry(
                        &mut reported,
                        &geometry_address,
                        "a cavity supplement requires a positive 'bbox_m' or 'points_m' with at least 4 points"
                            .to_string(),
                    );
                }
            }
            SupplementKind::Closure | SupplementKind::Connection => {
                for key in ["from", "to"] {
                    let ok = object
                        .get(key)
                        .map(|point| is_finite_point(point, 3))
                        .unwrap_or(false);
                    if !ok {
                        self.report_geometry(
                            &mut reported,
                            &geometry_address,
                            format!("a finite [x, y, z] '{key}' is required"),
                        );
                    }
                }
            }
            SupplementKind::Symmetry => {
                let axis_ok = matches!(
                    object.get("axis").and_then(|v| v.as_str()),
                    Some("x") | Some("y") | Some("z")
                );
                if !axis_ok {
                    self.report_geometry(
                        &mut reported,
                        &geometry_address,
                        "'axis' must be one of x, y, z".to_string(),
                    );
                }
                if let Some(offset) = object.get("plane_offset_m") {
                    let ok = offset.as_f64().map(|f| f.is_finite()).unwrap_or(false);
                    if !ok {
                        self.report_geometry(
                            &mut reported,
                            &geometry_address,
                            "'plane_offset_m' must be a finite number".to_string(),
                        );
                    }
                }
            }
        }
    }

    fn report_geometry(&mut self, reported: &mut bool, address: &str, message: String) {
        if !*reported {
            *reported = true;
            self.error(1, "SUPPLEMENT_GEOMETRY_INVALID", address, message);
        }
    }

    fn validate_unknowns(&mut self, package: &InputPackage) {
        if package.unknowns.len() > MAX_UNKNOWNS {
            self.error(
                1,
                "SCHEMA_RANGE",
                "unknowns",
                format!("at most {MAX_UNKNOWNS} unknowns are allowed"),
            );
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for (index, unknown) in package.unknowns.iter().enumerate() {
            let address = format!("unknowns[{index}]");
            if !is_valid_lower_id(&unknown.id) {
                self.error(
                    1,
                    "ID_INVALID",
                    &format!("{address}.id"),
                    "id must match ^[a-z][a-z0-9_]{0,63}$".to_string(),
                );
            }
            if !seen.insert(unknown.id.as_str()) {
                self.error(
                    1,
                    "ID_DUPLICATE",
                    &format!("{address}.id"),
                    format!("duplicate unknown id '{}'", unknown.id),
                );
            }
            let description_len = unknown.description.chars().count();
            if description_len == 0 || description_len > 500 {
                self.error(
                    1,
                    "SCHEMA_RANGE",
                    &format!("{address}.description"),
                    "description must be 1..=500 characters".to_string(),
                );
            }
            for (part_index, part_id) in unknown.affected_parts.iter().enumerate() {
                if !package.parts.iter().any(|p| p.part_id == *part_id) {
                    self.error(
                        1,
                        "REFERENCE_UNKNOWN",
                        &format!("{address}.affected_parts[{part_index}]"),
                        format!("part '{part_id}' does not exist"),
                    );
                }
            }
            if unknown.policy.is_none() {
                self.warning(
                    2,
                    "UNKNOWN_WITHOUT_POLICY",
                    &format!("{address}.policy"),
                    format!("unknown '{}' has no resolution policy", unknown.id),
                );
            }
        }
    }

    fn validate_measurements(&mut self, package: &InputPackage) {
        if package.measurements.len() > MAX_MEASUREMENTS {
            self.error(
                1,
                "SCHEMA_RANGE",
                "measurements",
                format!("at most {MAX_MEASUREMENTS} measurements are allowed"),
            );
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for (index, measurement) in package.measurements.iter().enumerate() {
            let address = format!("measurements[{index}]");
            if !is_valid_lower_id(&measurement.id) {
                self.error(
                    1,
                    "ID_INVALID",
                    &format!("{address}.id"),
                    "id must match ^[a-z][a-z0-9_]{0,63}$".to_string(),
                );
            }
            if !seen.insert(measurement.id.as_str()) {
                self.error(
                    1,
                    "ID_DUPLICATE",
                    &format!("{address}.id"),
                    format!("duplicate measurement id '{}'", measurement.id),
                );
            }
            if !measurement.value_m.is_finite() || measurement.value_m <= 0.0 {
                self.error(
                    1,
                    "MEASUREMENT_INVALID",
                    &format!("{address}.value_m"),
                    "value_m must be a finite number greater than zero".to_string(),
                );
            }
            if let Some(tolerance) = measurement.tolerance_m {
                if !tolerance.is_finite() || tolerance < 0.0 {
                    self.error(
                        1,
                        "MEASUREMENT_INVALID",
                        &format!("{address}.tolerance_m"),
                        "tolerance_m must be a finite number not below zero".to_string(),
                    );
                }
            }
        }
    }

    fn validate_uncertainty_policy(&mut self, package: &InputPackage) {
        let policy = match &package.uncertainty_policy {
            Some(policy) => policy,
            None => return,
        };
        if policy.allowed_completion_rules.len() > MAX_COMPLETION_RULES {
            self.error(
                1,
                "SCHEMA_RANGE",
                "uncertainty_policy.allowed_completion_rules",
                format!("at most {MAX_COMPLETION_RULES} completion rules are allowed"),
            );
        }
        for (index, rule) in policy.allowed_completion_rules.iter().enumerate() {
            let known = matches!(
                rule.as_str(),
                "thickness"
                    | "curve"
                    | "section"
                    | "profile"
                    | "closure"
                    | "cavity"
                    | "connection"
                    | "symmetry"
                    | "primitive_fill"
            );
            if !known {
                self.error(
                    1,
                    "SCHEMA_ENUM",
                    &format!("uncertainty_policy.allowed_completion_rules[{index}]"),
                    format!("unknown completion rule '{rule}'"),
                );
            }
        }
    }

    fn validate_views(&mut self, package: &InputPackage) {
        self.plan_total_bytes(package);
        if package.views.is_empty() {
            self.error(
                1,
                "SCHEMA_RANGE",
                "views",
                "at least one view is required".to_string(),
            );
        }
        if package.views.len() > MAX_VIEWS {
            self.error(
                1,
                "SCHEMA_RANGE",
                "views",
                format!("at most {MAX_VIEWS} views are allowed"),
            );
        }

        let mut seen: HashSet<&str> = HashSet::new();
        for (index, view) in package.views.iter().enumerate() {
            let address = format!("views[{index}]");
            if !is_valid_view_id(&view.view_id) {
                self.error(
                    1,
                    "ID_INVALID",
                    &format!("{address}.view_id"),
                    "view_id must match ^[a-z][a-z0-9_]{0,31}$".to_string(),
                );
            }
            if !seen.insert(view.view_id.as_str()) {
                self.error(
                    1,
                    "ID_DUPLICATE",
                    &format!("{address}.view_id"),
                    format!("duplicate view_id '{}'", view.view_id),
                );
            }
            if view.image.width_px == 0 || view.image.width_px > MAX_DIMENSION_PX {
                self.error(
                    1,
                    "SCHEMA_RANGE",
                    &format!("{address}.image.width_px"),
                    format!("width_px must be in 1..={MAX_DIMENSION_PX}"),
                );
            }
            if view.image.height_px == 0 || view.image.height_px > MAX_DIMENSION_PX {
                self.error(
                    1,
                    "SCHEMA_RANGE",
                    &format!("{address}.image.height_px"),
                    format!("height_px must be in 1..={MAX_DIMENSION_PX}"),
                );
            }
            let mask_pair_ok = matches!(
                (view.mask.format, view.mask.foreground),
                (MaskFormat::Grayscale8, MaskForeground::White)
                    | (MaskFormat::Rgba8, MaskForeground::Alpha)
            );
            if !mask_pair_ok {
                self.error(
                    1,
                    "SCHEMA_ENUM",
                    &format!("{address}.mask.format"),
                    "mask format and foreground must be grayscale8/white or rgba8/alpha"
                        .to_string(),
                );
            }
            if let Some(detail) = view.provenance.as_ref().and_then(|p| p.detail.as_ref()) {
                if detail.chars().count() > 1000 {
                    self.error(
                        1,
                        "SCHEMA_RANGE",
                        &format!("{address}.provenance.detail"),
                        "provenance.detail must be at most 1000 characters".to_string(),
                    );
                }
            }

            let direction = match &view.projection {
                Some(projection) => {
                    self.validate_projection(&format!("{address}.projection"), projection)
                }
                None => {
                    if view.kind == ViewKind::Calibrated {
                        self.error(
                            1,
                            "SCHEMA_TYPE",
                            &format!("{address}.projection"),
                            "a calibrated view requires a projection".to_string(),
                        );
                    }
                    None
                }
            };

            let image = self.validate_resource(
                &format!("{address}.image"),
                &view.image.path,
                &view.image.sha256,
                ResourceKind::Image,
                Some((view.image.width_px, view.image.height_px)),
                &format!("{address}.image.width_px"),
            );
            let expected_mask_dims = image.header.map(|header| (header.width, header.height));
            let mask = self.validate_resource(
                &format!("{address}.mask"),
                &view.mask.path,
                &view.mask.sha256,
                ResourceKind::Mask,
                expected_mask_dims,
                &format!("{address}.mask"),
            );

            if view.kind == ViewKind::Calibrated {
                if let Some(direction) = direction {
                    if image.ok && mask.ok {
                        self.directions.push((index, direction));
                    }
                }
            }
        }
    }

    fn validate_projection(&mut self, address: &str, projection: &Projection) -> Option<[f64; 3]> {
        let mut ok = true;

        for (name, vector) in [
            ("location_m", projection.location_m),
            ("look_at_m", projection.look_at_m),
            ("up_world", projection.up_world),
        ] {
            for (index, component) in vector.iter().enumerate() {
                if !component.is_finite() || component.abs() > MAX_COORDINATE_ABS {
                    self.error(
                        1,
                        "SCHEMA_RANGE",
                        &format!("{address}.{name}[{index}]"),
                        format!("{name}[{index}] must be finite and within +-{MAX_COORDINATE_ABS}"),
                    );
                    ok = false;
                }
            }
        }

        let up_length = vec_length(&projection.up_world);
        if !(0.99..=1.01).contains(&up_length) {
            self.error(
                1,
                "SCHEMA_RANGE",
                &format!("{address}.up_world"),
                "the length of up_world must be within [0.99, 1.01]".to_string(),
            );
            ok = false;
        }

        let direction = vec_sub(&projection.look_at_m, &projection.location_m);
        let direction_length = vec_length(&direction);
        if direction_length <= f64::EPSILON {
            self.error(
                1,
                "SCHEMA_RANGE",
                &format!("{address}.look_at_m"),
                "look_at_m must differ from location_m".to_string(),
            );
            ok = false;
        } else if up_length > f64::EPSILON {
            let alignment =
                (vec_dot(&direction, &projection.up_world) / (direction_length * up_length)).abs();
            if alignment >= 0.999 {
                self.error(
                    1,
                    "SCHEMA_RANGE",
                    &format!("{address}.up_world"),
                    "the view direction must not be parallel to up_world".to_string(),
                );
                ok = false;
            }
        }

        if !projection.ortho_scale_m.is_finite()
            || projection.ortho_scale_m <= 0.0
            || projection.ortho_scale_m > MAX_COORDINATE_ABS
        {
            self.error(
                1,
                "SCHEMA_RANGE",
                &format!("{address}.ortho_scale_m"),
                format!("ortho_scale_m must be > 0 and <= {MAX_COORDINATE_ABS}"),
            );
            ok = false;
        }
        if !projection.shift_x.is_finite() || projection.shift_x.abs() > MAX_SHIFT_ABS {
            self.error(
                1,
                "SCHEMA_RANGE",
                &format!("{address}.shift_x"),
                format!("shift_x must be within +-{MAX_SHIFT_ABS}"),
            );
            ok = false;
        }
        if !projection.shift_y.is_finite() || projection.shift_y.abs() > MAX_SHIFT_ABS {
            self.error(
                1,
                "SCHEMA_RANGE",
                &format!("{address}.shift_y"),
                format!("shift_y must be within +-{MAX_SHIFT_ABS}"),
            );
            ok = false;
        }
        if !projection.clip_start_m.is_finite() || projection.clip_start_m <= 0.0 {
            self.error(
                1,
                "SCHEMA_RANGE",
                &format!("{address}.clip_start_m"),
                "clip_start_m must be a finite number greater than zero".to_string(),
            );
            ok = false;
        }
        if !projection.clip_end_m.is_finite() || projection.clip_end_m <= projection.clip_start_m {
            self.error(
                1,
                "SCHEMA_RANGE",
                &format!("{address}.clip_end_m"),
                "clip_end_m must be greater than clip_start_m".to_string(),
            );
            ok = false;
        }

        if ok {
            Some(unit_vector(&direction))
        } else {
            None
        }
    }

    /// Sum the sizes of all referenced resources before any file is read.
    ///
    /// The total byte budget of 512 MiB is therefore checked without pulling
    /// hundreds of megabytes through the validator.
    fn plan_total_bytes(&mut self, package: &InputPackage) {
        let root = match &self.canonical_root {
            Some(root) => root.clone(),
            None => return,
        };
        let mut total: u64 = 0;
        for view in &package.views {
            for path in [&view.image.path, &view.mask.path] {
                if classify_path(path).is_err() {
                    continue;
                }
                let resolved = match fs::canonicalize(root.join(path)) {
                    Ok(resolved) => resolved,
                    Err(_) => continue,
                };
                if !resolved.starts_with(&root) {
                    continue;
                }
                if let Ok(meta) = fs::metadata(&resolved) {
                    if meta.is_file() {
                        total = total.saturating_add(meta.len());
                    }
                }
            }
        }
        if total > MAX_TOTAL_RESOURCE_BYTES {
            self.total_exceeded = true;
            self.error(
                1,
                "TOTAL_BYTES_EXCEEDED",
                "views",
                format!(
                    "the referenced resources total {total} bytes, the limit is {MAX_TOTAL_RESOURCE_BYTES} bytes"
                ),
            );
        }
    }

    fn validate_resource(
        &mut self,
        address: &str,
        path: &str,
        declared_hash: &str,
        kind: ResourceKind,
        expected_dims: Option<(u32, u32)>,
        dimension_address: &str,
    ) -> ResourceOutcome {
        let mut ok = true;

        if let Err(issue) = classify_path(path) {
            match issue {
                PathIssue::Invalid(message) => {
                    self.error(1, "PATH_INVALID", &format!("{address}.path"), message)
                }
                PathIssue::Outside(message) => self.error(
                    1,
                    "PATH_OUTSIDE_PACKAGE",
                    &format!("{address}.path"),
                    message,
                ),
            }
            self.push_resource(address, path, String::new(), 0, false);
            return ResourceOutcome {
                ok: false,
                header: None,
            };
        }

        let root = match &self.canonical_root {
            Some(root) => root.clone(),
            None => {
                self.error(
                    1,
                    "RESOURCE_MISSING",
                    &format!("{address}.path"),
                    "the package directory could not be resolved".to_string(),
                );
                self.push_resource(address, path, String::new(), 0, false);
                return ResourceOutcome {
                    ok: false,
                    header: None,
                };
            }
        };

        let candidate = root.join(path);
        let resolved = match fs::canonicalize(&candidate) {
            Ok(resolved) => resolved,
            Err(err) => {
                self.error(
                    1,
                    "RESOURCE_MISSING",
                    &format!("{address}.path"),
                    format!("resource cannot be resolved: {err}"),
                );
                self.push_resource(address, path, String::new(), 0, false);
                return ResourceOutcome {
                    ok: false,
                    header: None,
                };
            }
        };

        if !resolved.starts_with(&root) {
            self.error(
                1,
                "PATH_OUTSIDE_PACKAGE",
                &format!("{address}.path"),
                "the resource resolves outside the package directory".to_string(),
            );
            self.push_resource(address, path, String::new(), 0, false);
            return ResourceOutcome {
                ok: false,
                header: None,
            };
        }

        let meta = match fs::metadata(&resolved) {
            Ok(meta) => meta,
            Err(err) => {
                self.error(
                    1,
                    "RESOURCE_MISSING",
                    &format!("{address}.path"),
                    format!("resource cannot be read: {err}"),
                );
                self.push_resource(address, path, String::new(), 0, false);
                return ResourceOutcome {
                    ok: false,
                    header: None,
                };
            }
        };

        if !meta.is_file() {
            self.error(
                1,
                "RESOURCE_NOT_REGULAR",
                &format!("{address}.path"),
                "the resource is not a regular file".to_string(),
            );
            self.push_resource(address, path, String::new(), 0, false);
            return ResourceOutcome {
                ok: false,
                header: None,
            };
        }

        let bytes_len = meta.len();
        if bytes_len > MAX_RESOURCE_BYTES {
            self.error(
                1,
                "RESOURCE_TOO_LARGE",
                &format!("{address}.path"),
                format!(
                    "the resource is {bytes_len} bytes, the limit is {MAX_RESOURCE_BYTES} bytes"
                ),
            );
            self.push_resource(address, path, String::new(), bytes_len, false);
            return ResourceOutcome {
                ok: false,
                header: None,
            };
        }

        if self.total_exceeded {
            // The package is above the total byte budget: the remaining files
            // are not read, so that a huge package cannot stall the validator.
            self.push_resource(address, path, String::new(), bytes_len, false);
            return ResourceOutcome {
                ok: false,
                header: None,
            };
        }

        if let Ok(modified) = meta.modified() {
            self.max_resource_mtime = Some(match self.max_resource_mtime {
                Some(current) => {
                    if modified > current {
                        modified
                    } else {
                        current
                    }
                }
                None => modified,
            });
        }

        let bytes = match fs::read(&resolved) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.error(
                    1,
                    "RESOURCE_MISSING",
                    &format!("{address}.path"),
                    format!("resource cannot be read: {err}"),
                );
                self.push_resource(address, path, String::new(), bytes_len, false);
                return ResourceOutcome {
                    ok: false,
                    header: None,
                };
            }
        };

        let computed_hash = sha256_hex(&bytes);
        let hash_format_ok = is_lowercase_hex64(declared_hash);
        if !hash_format_ok {
            self.error(
                1,
                "HASH_INVALID",
                &format!("{address}.sha256"),
                "sha256 must be 64 lowercase hexadecimal characters".to_string(),
            );
            ok = false;
        }
        let verified = hash_format_ok && computed_hash == declared_hash;
        if hash_format_ok && !verified {
            let message = format!(
                "the manifest declares {declared_hash}, the file hashes to {computed_hash}"
            );
            if self.options.verify_hashes {
                self.error(1, "HASH_MISMATCH", &format!("{address}.sha256"), message);
                ok = false;
            } else {
                self.warning(1, "HASH_MISMATCH", &format!("{address}.sha256"), message);
            }
        }

        let header = match png::parse_png_header(&bytes) {
            Ok(header) => header,
            Err(err) => {
                let code = png_error_code(&err);
                self.error(
                    1,
                    code,
                    &format!("{address}.path"),
                    format!("invalid PNG resource: {err}"),
                );
                self.push_resource(address, path, computed_hash, bytes_len, verified);
                return ResourceOutcome {
                    ok: false,
                    header: None,
                };
            }
        };

        let color_type_ok = match kind {
            ResourceKind::Image => png::is_image_color_type(header.color_type),
            ResourceKind::Mask => png::is_mask_color_type(header.color_type),
        };
        if !color_type_ok {
            self.error(
                1,
                "PNG_COLOR_TYPE",
                &format!("{address}.path"),
                format!(
                    "PNG color type {} is not allowed for this resource kind",
                    header.color_type
                ),
            );
            ok = false;
        }

        if let Some((width, height)) = expected_dims {
            if header.width != width || header.height != height {
                self.error(
                    1,
                    "PNG_DIMENSION_MISMATCH",
                    dimension_address,
                    format!(
                        "the manifest declares {width}x{height}, the PNG is {}x{}",
                        header.width, header.height
                    ),
                );
                ok = false;
            }
        }

        self.push_resource(address, path, computed_hash, bytes_len, verified);
        ResourceOutcome {
            ok,
            header: Some(header),
        }
    }

    fn push_resource(
        &mut self,
        address: &str,
        path: &str,
        sha256: String,
        bytes: u64,
        verified: bool,
    ) {
        self.resources.push(ResourceReport {
            address: address.to_string(),
            path: path.to_string(),
            sha256,
            bytes,
            verified,
        });
    }

    fn validate_acceptance(&mut self, package: &InputPackage) {
        if package.acceptance.len() > MAX_ACCEPTANCE {
            self.error(
                1,
                "SCHEMA_RANGE",
                "acceptance",
                format!("at most {MAX_ACCEPTANCE} acceptance entries are allowed"),
            );
        }

        let mut groups: HashMap<&str, Vec<(usize, AcceptanceStatus)>> = HashMap::new();
        for (index, acceptance) in package.acceptance.iter().enumerate() {
            let address = format!("acceptance[{index}]");
            let known = match acceptance.target.split_once(':') {
                Some((collection, id)) => match collection {
                    "views" => package.views.iter().any(|v| v.view_id == id),
                    "parts" => package.parts.iter().any(|p| p.part_id == id),
                    "supplements" => package.supplements.iter().any(|s| s.supplement_id == id),
                    "unknowns" => package.unknowns.iter().any(|u| u.id == id),
                    "measurements" => package.measurements.iter().any(|m| m.id == id),
                    _ => false,
                },
                None => false,
            };
            if !known {
                self.error(
                    1,
                    "ACCEPTANCE_TARGET_UNKNOWN",
                    &format!("{address}.target"),
                    format!(
                        "target '{}' does not reference an existing element",
                        acceptance.target
                    ),
                );
            }
            let by_len = acceptance.by.chars().count();
            if by_len == 0 || by_len > 128 {
                self.error(
                    1,
                    "SCHEMA_RANGE",
                    &format!("{address}.by"),
                    "by must be 1..=128 characters".to_string(),
                );
            }
            if !is_rfc3339(&acceptance.at) {
                self.error(
                    1,
                    "SCHEMA_TYPE",
                    &format!("{address}.at"),
                    "at must be an RFC 3339 timestamp".to_string(),
                );
            }
            groups
                .entry(acceptance.target.as_str())
                .or_default()
                .push((index, acceptance.status));
        }

        for entries in groups.values() {
            if entries.len() < 2 {
                continue;
            }
            let (last_index, last_status) = match entries.last() {
                Some(entry) => *entry,
                None => continue,
            };
            for (index, status) in entries.iter().take(entries.len() - 1) {
                if *status == AcceptanceStatus::Accepted
                    && last_status != AcceptanceStatus::Accepted
                {
                    self.warning(
                        2,
                        "ACCEPTANCE_SUPERSEDED",
                        &format!("acceptance[{index}].status"),
                        format!(
                            "the acceptance of this target is superseded by acceptance[{last_index}]"
                        ),
                    );
                }
            }
        }

        let require_acceptance = package
            .uncertainty_policy
            .as_ref()
            .map(|policy| policy.require_author_acceptance)
            .unwrap_or(true);
        if require_acceptance {
            for (index, supplement) in package.supplements.iter().enumerate() {
                if supplement.provenance.origin != AuthorOrigin::AiAuthor {
                    continue;
                }
                let target = format!("supplements:{}", supplement.supplement_id);
                let accepted = groups
                    .get(target.as_str())
                    .and_then(|entries| entries.last())
                    .map(|(_, status)| *status == AcceptanceStatus::Accepted)
                    .unwrap_or(false);
                if !accepted {
                    self.warning(
                        2,
                        "SUPPLEMENT_NOT_ACCEPTED",
                        &format!("supplements[{index}].supplement_id"),
                        format!(
                            "supplement '{}' is authored by AI and has no accepted acceptance entry",
                            supplement.supplement_id
                        ),
                    );
                }
            }
        }
    }

    fn validate_sufficiency(&mut self, package: &InputPackage) {
        for (index, view) in package.views.iter().enumerate() {
            if view.kind == ViewKind::Reference && view.projection.is_some() {
                self.warning(
                    2,
                    "REFERENCE_PROJECTION_IGNORED",
                    &format!("views[{index}].projection"),
                    "the projection of a reference view is ignored".to_string(),
                );
            }
        }

        for i in 0..self.directions.len() {
            for j in 0..i {
                let (index, direction) = self.directions[i];
                let (other_index, other_direction) = self.directions[j];
                if vec_dot(&direction, &other_direction).abs() > DUPLICATE_DIRECTION_DOT {
                    self.warning(
                        2,
                        "DUPLICATE_VIEW_DIRECTION",
                        &format!("views[{index}].projection"),
                        format!("the view has the same direction as views[{other_index}]"),
                    );
                    break;
                }
            }
        }

        let mut clusters: Vec<[f64; 3]> = Vec::new();
        for (_, direction) in &self.directions {
            // Directions belong to the same cluster when they are nearly
            // parallel; only directions that are independent start a new one.
            let known = clusters
                .iter()
                .any(|cluster| vec_dot(cluster, direction).abs() > DISTINCT_DIRECTION_DOT);
            if !known {
                clusters.push(*direction);
            }
        }

        let mut axes: BTreeSet<String> = BTreeSet::new();
        for cluster in &clusters {
            axes.insert(dominant_axis(cluster).to_string());
        }

        let calibrated_views = self.directions.len() as u32;
        let distinct_directions = clusters.len() as u32;
        let complete = calibrated_views >= 2 && distinct_directions >= 2 && axes.len() >= 2;

        let mut missing: Vec<String> = Vec::new();
        if calibrated_views == 0 {
            missing.push("INSUFFICIENT_CALIBRATION".to_string());
        }
        if distinct_directions < 2 || axes.len() < 2 {
            missing.push("INSUFFICIENT_VIEW_DIRECTIONS".to_string());
        }

        for code in &missing {
            let severity = if self.options.require_complete {
                Severity::Error
            } else {
                Severity::Warning
            };
            self.push_diag(
                2,
                code,
                severity,
                "completeness",
                format!("the package is incomplete: {code}"),
            );
        }

        self.completeness = Completeness {
            status: if complete {
                CompletenessStatus::Complete
            } else {
                CompletenessStatus::Partial
            },
            calibrated_views,
            distinct_directions,
            axes_covered: axes.into_iter().collect(),
            missing,
        };

        if let Some(name) = self.root.file_name().and_then(|name| name.to_str()) {
            match parse_revision_dir(name) {
                Some(revision) => {
                    if revision != self.revision {
                        self.warning(
                            2,
                            "REVISION_DIR_MISMATCH",
                            "revision",
                            format!(
                                "the package directory '{name}' does not match revision {}",
                                self.revision
                            ),
                        );
                    }
                }
                None => {
                    if self.manifest_is_newer_than_resources() {
                        self.warning(
                            2,
                            "PACKAGE_DIR_WRITABLE",
                            "package",
                            "the package directory is writable and the manifest is newer than all resources"
                                .to_string(),
                        );
                    }
                }
            }
        }
    }

    fn manifest_is_newer_than_resources(&self) -> bool {
        let manifest = match self.manifest_mtime {
            Some(time) => time,
            None => return false,
        };
        let resources = match self.max_resource_mtime {
            Some(time) => time,
            None => return false,
        };
        epoch_seconds(manifest) > epoch_seconds(resources)
    }

    fn finish(mut self) -> InputPackageReport {
        self.diagnostics.sort_by(|a, b| {
            (a.level, a.address.as_str(), a.code.as_str()).cmp(&(
                b.level,
                b.address.as_str(),
                b.code.as_str(),
            ))
        });

        let mut level1_errors = 0u32;
        let mut level1_warnings = 0u32;
        let mut level2_errors = 0u32;
        let mut level2_warnings = 0u32;
        for diagnostic in &self.diagnostics {
            match (diagnostic.level, diagnostic.severity) {
                (1, Severity::Error) => level1_errors += 1,
                (1, Severity::Warning) => level1_warnings += 1,
                (_, Severity::Error) => level2_errors += 1,
                (_, Severity::Warning) => level2_warnings += 1,
            }
        }

        let status = if level1_errors > 0 || level2_errors > 0 {
            ReportStatus::Invalid
        } else if self.completeness.status == CompletenessStatus::Complete {
            ReportStatus::ValidComplete
        } else {
            ReportStatus::ValidPartial
        };

        InputPackageReport {
            schema: REPORT_SCHEMA.to_string(),
            package_id: self.package_id,
            revision: self.revision,
            status,
            levels: ValidationLevels {
                format_safety: LevelSummary {
                    ok: level1_errors == 0,
                    errors: level1_errors,
                    warnings: level1_warnings,
                },
                sufficiency: LevelSummary {
                    ok: level2_errors == 0,
                    errors: level2_errors,
                    warnings: level2_warnings,
                },
            },
            completeness: self.completeness,
            diagnostics: self.diagnostics,
            resources: self.resources,
        }
    }
}

fn epoch_seconds(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i64,
        Err(_) => 0,
    }
}

fn parse_revision_dir(name: &str) -> Option<u32> {
    let digits = name.strip_prefix('r')?;
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok()
}

fn dominant_axis(direction: &[f64; 3]) -> &'static str {
    let x = direction[0].abs();
    let y = direction[1].abs();
    let z = direction[2].abs();
    if x >= y && x >= z {
        "x"
    } else if y >= z {
        "y"
    } else {
        "z"
    }
}

fn vec_sub(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn vec_dot(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn vec_length(a: &[f64; 3]) -> f64 {
    vec_dot(a, a).sqrt()
}

fn unit_vector(a: &[f64; 3]) -> [f64; 3] {
    let length = vec_length(a);
    if length <= f64::EPSILON {
        [0.0, 0.0, 0.0]
    } else {
        [a[0] / length, a[1] / length, a[2] / length]
    }
}

fn classify_path(path: &str) -> Result<(), PathIssue> {
    if path.is_empty() {
        return Err(PathIssue::Invalid("the path is empty".to_string()));
    }
    if path.chars().count() > MAX_PATH_CHARS {
        return Err(PathIssue::Invalid(format!(
            "the path is longer than {MAX_PATH_CHARS} characters"
        )));
    }
    if path.as_bytes().contains(&0) {
        return Err(PathIssue::Invalid(
            "the path contains a NUL byte".to_string(),
        ));
    }
    if path.contains('\\') {
        return Err(PathIssue::Invalid(
            "the path contains a backslash".to_string(),
        ));
    }
    if path.contains(':') {
        return Err(PathIssue::Invalid(
            "the path contains a drive letter or colon".to_string(),
        ));
    }
    if path.starts_with('/') {
        return Err(PathIssue::Outside("the path is absolute".to_string()));
    }
    if path.split('/').any(|component| component == "..") {
        return Err(PathIssue::Outside(
            "the path contains a parent traversal".to_string(),
        ));
    }
    Ok(())
}

fn png_error_code(error: &png::PngError) -> &'static str {
    match error {
        png::PngError::Signature => "PNG_SIGNATURE",
        png::PngError::Truncated => "PNG_TRUNCATED",
        png::PngError::MissingIend => "PNG_TRUNCATED",
        png::PngError::ColorType(_) => "PNG_COLOR_TYPE",
        png::PngError::BitDepth(_) => "PNG_IHDR_INVALID",
        png::PngError::IhdrInvalid(_) => "PNG_IHDR_INVALID",
        png::PngError::UnknownCriticalChunk(_) => "PNG_IHDR_INVALID",
        png::PngError::CrcMismatch(_) => "PNG_IHDR_INVALID",
        png::PngError::Io(_) => "RESOURCE_MISSING",
    }
}

fn is_lowercase_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_valid_package_id(value: &str) -> bool {
    let mut chars = value.chars();
    let first = match chars.next() {
        Some(first) => first,
        None => return false,
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    if rest.len() > 63 {
        return false;
    }
    rest.iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn is_valid_view_id(value: &str) -> bool {
    let mut chars = value.chars();
    let first = match chars.next() {
        Some(first) => first,
        None => return false,
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    if rest.len() > 31 {
        return false;
    }
    rest.iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
}

fn is_valid_part_token(value: &str) -> bool {
    let mut chars = value.chars();
    let first = match chars.next() {
        Some(first) => first,
        None => return false,
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    if rest.len() > 63 {
        return false;
    }
    rest.iter()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | '-'))
}

fn is_valid_lower_id(value: &str) -> bool {
    let mut chars = value.chars();
    let first = match chars.next() {
        Some(first) => first,
        None => return false,
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    if rest.len() > 63 {
        return false;
    }
    rest.iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
}

fn json_depth(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => 1 + map.values().map(json_depth).max().unwrap_or(0),
        serde_json::Value::Array(items) => 1 + items.iter().map(json_depth).max().unwrap_or(0),
        _ => 0,
    }
}

fn json_numbers_in_range(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.values().all(json_numbers_in_range),
        serde_json::Value::Array(items) => items.iter().all(json_numbers_in_range),
        serde_json::Value::Number(number) => number
            .as_f64()
            .map(|f| f.is_finite() && f.abs() <= MAX_COORDINATE_ABS)
            .unwrap_or(false),
        _ => true,
    }
}

fn is_finite_point(value: &serde_json::Value, dimensions: usize) -> bool {
    let items = match value.as_array() {
        Some(items) => items,
        None => return false,
    };
    if items.len() != dimensions {
        return false;
    }
    items
        .iter()
        .all(|item| item.as_f64().map(|f| f.is_finite()).unwrap_or(false))
}

fn is_rfc3339(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20 {
        return false;
    }
    let digits = |mut range: std::ops::Range<usize>| range.all(|i| bytes[i].is_ascii_digit());
    if !digits(0..4) || bytes[4] != b'-' || !digits(5..7) || bytes[7] != b'-' || !digits(8..10) {
        return false;
    }
    if bytes[10] != b'T' && bytes[10] != b't' {
        return false;
    }
    if !digits(11..13)
        || bytes[13] != b':'
        || !digits(14..16)
        || bytes[16] != b':'
        || !digits(17..19)
    {
        return false;
    }
    let mut index = 19;
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    if index >= bytes.len() {
        return false;
    }
    match bytes[index] {
        b'Z' | b'z' => index + 1 == bytes.len(),
        b'+' | b'-' => {
            index += 1;
            if index + 5 != bytes.len() {
                return false;
            }
            digits(index..index + 2) && bytes[index + 2] == b':' && digits(index + 3..index + 5)
        }
        _ => false,
    }
}

fn classify_serde_error(message: &str) -> &'static str {
    if message.contains("unknown field") {
        "SCHEMA_UNKNOWN_FIELD"
    } else if message.contains("unknown variant") {
        "SCHEMA_ENUM"
    } else {
        "SCHEMA_TYPE"
    }
}

fn serde_error_address(message: &str) -> Option<String> {
    for marker in ["unknown field ", "missing field ", "unknown variant "] {
        if let Some(index) = message.find(marker) {
            let rest = &message[index + marker.len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            if end > 0 {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}
