//! Michelangelo Blender — Blender subprocess adapter.
//!
//! Launches Blender as a bounded background subprocess, writes job specs,
//! captures output, and parses worker results.
//!
//! Supports fake executable mode for deterministic integration tests
//! without a real Blender installation.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use michelangelo_protocol::{BlenderJobResult, BlenderJobSpec};

/// Errors that can occur during Blender subprocess execution.
#[derive(Debug)]
pub enum BlenderError {
    /// The Blender binary was not found at the configured path.
    BinaryNotFound(String),
    /// The subprocess exited with a non-zero code.
    NonZeroExit {
        exit_code: Option<i32>,
        stderr: String,
    },
    /// The subprocess did not complete within the configured timeout.
    Timeout,
    /// The expected result file was not created.
    MissingResultFile(String),
    /// The result file contains invalid JSON.
    MalformedResult(String),
    /// An I/O error occurred.
    Io(std::io::Error),
}

impl std::fmt::Display for BlenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlenderError::BinaryNotFound(path) => {
                write!(f, "Blender binary not found at '{}'", path)
            }
            BlenderError::NonZeroExit { exit_code, stderr } => {
                write!(
                    f,
                    "Blender exited with code {:?}: {}",
                    exit_code,
                    stderr.trim()
                )
            }
            BlenderError::Timeout => write!(f, "Blender subprocess timed out"),
            BlenderError::MissingResultFile(path) => {
                write!(f, "Missing result file: {}", path)
            }
            BlenderError::MalformedResult(msg) => {
                write!(f, "Malformed Blender result: {}", msg)
            }
            BlenderError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for BlenderError {}

impl From<std::io::Error> for BlenderError {
    fn from(e: std::io::Error) -> Self {
        BlenderError::Io(e)
    }
}

impl From<serde_json::Error> for BlenderError {
    fn from(e: serde_json::Error) -> Self {
        BlenderError::MalformedResult(e.to_string())
    }
}

/// Adapter for launching Blender as a bounded subprocess.
pub struct HeadlessBlenderAdapter {
    /// Path to the Blender executable (or name for PATH lookup).
    blender_bin: String,
    /// Path to the Python worker script.
    worker_script: PathBuf,
}

impl HeadlessBlenderAdapter {
    /// Create a new adapter using `BLENDER_BIN` env var or default `"blender"`.
    ///
    /// The worker script is resolved relative to `CARGO_MANIFEST_DIR` or the
    /// current directory, looking for `blender/worker.py`.
    pub fn new() -> Self {
        let blender_bin = std::env::var("BLENDER_BIN").unwrap_or_else(|_| "blender".into());

        // Resolve worker script path: try common locations
        let worker_script = resolve_worker_script();

        Self {
            blender_bin,
            worker_script,
        }
    }

    /// Create an adapter with explicit configuration (primarily for testing).
    pub fn with_config(blender_bin: impl Into<String>, worker_script: impl Into<PathBuf>) -> Self {
        Self {
            blender_bin: blender_bin.into(),
            worker_script: worker_script.into(),
        }
    }

    /// Return the configured Blender binary path.
    pub fn blender_bin(&self) -> &str {
        &self.blender_bin
    }

    /// Check whether the Blender binary exists and is executable.
    pub fn check_binary(&self) -> Result<String, BlenderError> {
        let output = Command::new(&self.blender_bin)
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|_| BlenderError::BinaryNotFound(self.blender_bin.clone()))?;

        if !output.status.success() {
            return Err(BlenderError::BinaryNotFound(self.blender_bin.clone()));
        }

        let version = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("unknown")
            .to_string();

        Ok(version)
    }

    /// Run a Blender job synchronously with optional timeout.
    ///
    /// Writes `job.json` to `output_dir/<job_id>/`, executes Blender,
    /// and parses `result.json` from the same directory.
    pub fn run_job(
        &self,
        job_id: &str,
        job_spec: &BlenderJobSpec,
        workspace_root: &Path,
        output_dir: &Path,
        timeout_ms: Option<u64>,
    ) -> Result<BlenderJobResult, BlenderError> {
        // Validate spec paths before any I/O
        validate_job_spec_paths(job_spec)?;

        // Create job-specific output directory
        let job_dir = output_dir.join(job_id);
        std::fs::create_dir_all(&job_dir)?;

        // Write job.json
        let job_json_path = job_dir.join("job.json");
        let result_json_path = job_dir.join("result.json");

        // Build the worker input with resolved paths
        let worker_input = serde_json::json!({
            "job_id": job_id,
            "spec": job_spec.spec,
            "workspace_root": workspace_root.display().to_string(),
        });

        let mut file = std::fs::File::create(&job_json_path)?;
        serde_json::to_writer(&mut file, &worker_input)?;
        file.flush()?;

        // Verify worker script exists
        if !self.worker_script.exists() {
            return Err(BlenderError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Worker script not found: {}", self.worker_script.display()),
            )));
        }

        // Build the Blender command
        let mut cmd = Command::new(&self.blender_bin);
        cmd.arg("--background")
            .arg("--python")
            .arg(&self.worker_script)
            .arg("--")
            .arg(&job_json_path)
            .arg(&result_json_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Launch the subprocess (with retry for ETXTBUSY under parallel test load)
        let mut child = spawn_with_retry(&mut cmd, 3, Duration::from_millis(20)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BlenderError::BinaryNotFound(self.blender_bin.clone())
            } else {
                BlenderError::Io(e)
            }
        })?;

        // Wait with optional timeout
        let result = if let Some(ms) = timeout_ms {
            wait_with_timeout(&mut child, Duration::from_millis(ms))?
        } else {
            wait_for_completion(&mut child)?
        };

        // Check exit status
        if let Some(code) = result.exit_code {
            if code != 0 {
                return Err(BlenderError::NonZeroExit {
                    exit_code: Some(code),
                    stderr: result.stderr,
                });
            }
        }

        // Read result.json
        if !result_json_path.exists() {
            return Err(BlenderError::MissingResultFile(
                result_json_path.display().to_string(),
            ));
        }

        let result_content = std::fs::read_to_string(&result_json_path)?;
        let blender_result: BlenderJobResult =
            serde_json::from_str(&result_content).map_err(|e| {
                BlenderError::MalformedResult(format!(
                    "Failed to parse result JSON: {} — content (first 200 chars): {}",
                    e,
                    &result_content[..result_content.len().min(200)]
                ))
            })?;

        // Validate artifact paths are relative
        for artifact in blender_result.artifacts.iter().flatten() {
            validate_artifact_path(&artifact.path)?;
        }

        Ok(blender_result)
    }

    /// Execute a fake/canned Blender invocation for testing.
    /// Uses `fake_blender_path` instead of the real Blender binary.
    pub fn run_job_with_fake(
        &self,
        fake_blender: &Path,
        job_id: &str,
        job_spec: &BlenderJobSpec,
        workspace_root: &Path,
        output_dir: &Path,
        timeout_ms: Option<u64>,
    ) -> Result<BlenderJobResult, BlenderError> {
        // Validate spec paths before any I/O
        validate_job_spec_paths(job_spec)?;

        let job_dir = output_dir.join(job_id);
        std::fs::create_dir_all(&job_dir)?;

        let job_json_path = job_dir.join("job.json");
        let result_json_path = job_dir.join("result.json");

        let worker_input = serde_json::json!({
            "job_id": job_id,
            "spec": job_spec.spec,
            "workspace_root": workspace_root.display().to_string(),
        });

        let mut file = std::fs::File::create(&job_json_path)?;
        serde_json::to_writer(&mut file, &worker_input)?;
        file.flush()?;

        let mut cmd = Command::new(fake_blender);
        cmd.arg("--background")
            .arg("--python")
            .arg(&self.worker_script)
            .arg("--")
            .arg(&job_json_path)
            .arg(&result_json_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = spawn_with_retry(&mut cmd, 3, Duration::from_millis(20))?;

        let result = if let Some(ms) = timeout_ms {
            wait_with_timeout(&mut child, Duration::from_millis(ms))?
        } else {
            wait_for_completion(&mut child)?
        };

        if let Some(code) = result.exit_code {
            if code != 0 {
                let stderr = result.stderr;
                return Err(BlenderError::NonZeroExit {
                    exit_code: Some(code),
                    stderr,
                });
            }
        }

        if !result_json_path.exists() {
            return Err(BlenderError::MissingResultFile(
                result_json_path.display().to_string(),
            ));
        }

        let result_content = std::fs::read_to_string(&result_json_path)?;
        let blender_result: BlenderJobResult = serde_json::from_str(&result_content)
            .map_err(|e| BlenderError::MalformedResult(format!("{e}")))?;

        Ok(blender_result)
    }
}

impl Default for HeadlessBlenderAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

struct SubprocessResult {
    exit_code: Option<i32>,
    stderr: String,
}

fn wait_for_completion(child: &mut Child) -> Result<SubprocessResult, BlenderError> {
    let stderr_output = child
        .stderr
        .take()
        .map(|s| std::io::read_to_string(s).unwrap_or_default())
        .unwrap_or_default();

    let status = child.wait()?;
    Ok(SubprocessResult {
        exit_code: status.code(),
        stderr: stderr_output,
    })
}

fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> Result<SubprocessResult, BlenderError> {
    let stderr_handle = child.stderr.take();

    let start = std::time::Instant::now();
    loop {
        if start.elapsed() >= timeout {
            // Kill the process
            let _ = child.kill();
            let _ = child.wait();
            return Err(BlenderError::Timeout);
        }

        // Try waiting with a short poll interval
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = stderr_handle
                    .map(|s| std::io::read_to_string(s).unwrap_or_default())
                    .unwrap_or_default();
                return Ok(SubprocessResult {
                    exit_code: status.code(),
                    stderr,
                });
            }
            Ok(None) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(BlenderError::Io(e));
            }
        }
    }
}

/// Validate all file paths in a job spec BEFORE any I/O.
///
/// Checks that `save_blend`, `export_glb`, and `render_views` paths
/// are relative and have no parent-traversal components.
fn validate_job_spec_paths(spec: &BlenderJobSpec) -> Result<(), BlenderError> {
    for op in &spec.spec.operations {
        if op.op == "save_blend" || op.op == "export_glb" {
            if let Some(ref path) = op.path {
                validate_spec_path(path)?;
            }
        }
        if op.op == "render_views" {
            // base_path defaults to "blender/renders" (safe)
            if let Some(ref base) = op.path {
                validate_spec_path(base)?;
            }
        }
    }
    Ok(())
}

/// Validate a single spec path (must be relative, no parent-traversal).
fn validate_spec_path(path: &str) -> Result<(), BlenderError> {
    if Path::new(path).is_absolute() {
        return Err(BlenderError::MalformedResult(format!(
            "Absolute path in job spec rejected: '{}'",
            path
        )));
    }
    if path.contains("..") {
        return Err(BlenderError::MalformedResult(format!(
            "Parent-traversal path in job spec rejected: '{}'",
            path
        )));
    }
    Ok(())
}

/// Validate that an artifact path is workspace-relative (no absolute or parent-traversal).
/// Spawn a child process with a single retry on `ExecutableFileBusy` (ETXTBUSY).
///
/// Under parallel `cargo test --workspace` the filesystem may briefly report a
/// freshly-written script as "text file busy". Retrying once with a short delay
/// avoids flaky failures without needing nightly CommandExt.
fn spawn_with_retry(
    cmd: &mut Command,
    _max_attempts: u32,
    delay: Duration,
) -> Result<Child, std::io::Error> {
    // First attempt
    match cmd.spawn() {
        Ok(child) => return Ok(child),
        Err(e) if e.raw_os_error() == Some(26) => {
            // ETXTBUSY — wait briefly and try again
            std::thread::sleep(delay);
        }
        Err(e) => return Err(e),
    }
    // Second (final) attempt
    cmd.spawn()
}

fn validate_artifact_path(path: &str) -> Result<(), BlenderError> {
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(BlenderError::MalformedResult(format!(
            "Absolute artifact path rejected: '{}'",
            path
        )));
    }
    let components: Vec<_> = p.components().collect();
    if components
        .iter()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(BlenderError::MalformedResult(format!(
            "Parent-traversal artifact path rejected: '{}'",
            path
        )));
    }
    Ok(())
}

/// Resolve the worker.py script path by checking common locations.
fn resolve_worker_script() -> PathBuf {
    // Try relative to CARGO_MANIFEST_DIR (for crate tests)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        // From michelangelo_blender, go up two levels to workspace root
        let from_manifest = Path::new(&manifest_dir)
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("blender").join("worker.py"));
        if let Some(path) = from_manifest {
            if path.exists() {
                return path;
            }
        }
    }

    // Try relative to current directory
    let cwd_path = Path::new("blender/worker.py");
    if cwd_path.exists() {
        return cwd_path.to_path_buf();
    }

    // Fallback: use a reasonable default
    PathBuf::from("blender/worker.py")
}

// ---------------------------------------------------------------------------
// Fake Blender executable helpers
// ---------------------------------------------------------------------------

/// Создаёт временный shell-скрипт для детерминированного тестирования.
///
/// Поддерживаемые режимы:
/// - `"success"` — пишет валидный result.json
/// - `"fail"` — exit 1
/// - `"malformed"` — пишет мусор в result.json
/// - `"noresult"` — не пишет result.json
/// - `"timeout"` — спит 2 секунды (достаточно для теста таймаута)
pub fn create_fake_blender(dir: &Path, mode: &str) -> PathBuf {
    use std::io::Write;

    let script = dir.join("fake_blender.sh");
    let contents = match mode {
        "fail" => b"#!/bin/bash\necho 'Fake Blender: fail' >&2\nexit 1\n" as &[u8],
        "malformed" => b"#!/bin/bash\nresult_json=\"${@: -1}\"\necho '{this is garbage' > \"$result_json\"\nexit 0\n",
        "noresult" => b"#!/bin/bash\necho 'Fake Blender: no result' >&2\nexit 0\n",
        "timeout" => b"#!/bin/bash\necho 'Fake Blender: timeout' >&2\nsleep 2\nexit 0\n",
        _ => {
            // success mode script
            b"#!/bin/bash\njob_json=\"${@: -2:1}\"; result_json=\"${@: -1}\"\n\
            job_id=$(python3 -c \"import json,sys; print(json.load(open(sys.argv[1])).get('job_id','unknown'))\" \"$job_json\" 2>/dev/null || echo unknown)\n\
            cat > \"$result_json\" <<'RES'\n\
{\"job_id\":\"${job_id}\",\"status\":\"completed\",\"message\":\"Fake Blender success\",\"artifacts\":[{\"path\":\"blender/scenes/smoke.blend\",\"type\":\"blend\",\"size_bytes\":1024}],\"scene_summary\":{\"objects\":1,\"meshes\":1,\"triangles\":12,\"vertices\":24}}\n\
RES\n\
exit 0\n"
        }
    };

    // Write with explicit sync to avoid ETXTBUSY in parallel runs
    {
        let mut f = std::fs::File::create(&script).expect("create fake blender script");
        f.write_all(contents).expect("write fake blender script");
        f.sync_all().expect("sync fake blender script");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make fake blender executable");
    }

    // Also sync the parent directory so the new file is durable
    if let Some(parent) = script.parent() {
        if let Ok(f) = std::fs::File::open(parent) {
            let _ = f.sync_all();
        }
    }

    script
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use michelangelo_protocol::{BlenderJobSpec, BlenderOperation, BlenderSpecBody};
    use tempfile::TempDir;

    fn sample_job_spec() -> BlenderJobSpec {
        BlenderJobSpec {
            job_type: "test_smoke".into(),
            wait: true,
            timeout_ms: Some(30_000),
            spec: BlenderSpecBody {
                operations: vec![BlenderOperation {
                    op: "create_box".into(),
                    name: Some("test_cube".into()),
                    size: Some(vec![2.0, 1.0, 0.5]),
                    location: Some(vec![0.0, 0.0, 0.0]),
                    color: None,
                    object: None,
                    views: None,
                    path: None,
                }],
            },
        }
    }

    #[test]
    fn test_adapter_defaults_to_blender() {
        let adapter = HeadlessBlenderAdapter::new();
        // Without BLENDER_BIN set, defaults to "blender"
        assert_eq!(adapter.blender_bin(), "blender");
    }

    #[test]
    fn test_blender_bin_env_var() {
        // Temporarily set BLENDER_BIN
        std::env::set_var("BLENDER_BIN", "/custom/blender");
        let adapter = HeadlessBlenderAdapter::new();
        assert_eq!(adapter.blender_bin(), "/custom/blender");
        std::env::remove_var("BLENDER_BIN");
    }

    #[test]
    fn test_adapter_with_config() {
        let adapter =
            HeadlessBlenderAdapter::with_config("my_blender", PathBuf::from("blender/worker.py"));
        assert_eq!(adapter.blender_bin(), "my_blender");
    }

    #[test]
    fn test_fake_success() {
        let dir = TempDir::new().unwrap();
        let fake = create_fake_blender(dir.path(), "success");

        let adapter =
            HeadlessBlenderAdapter::with_config("blender", PathBuf::from("blender/worker.py"));
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let result = adapter.run_job_with_fake(
            &fake,
            "test_job_001",
            &sample_job_spec(),
            workspace.path(),
            output.path(),
            Some(10_000),
        );

        assert!(result.is_ok(), "fake success failed: {:?}", result.err());
        let r = result.unwrap();
        assert_eq!(r.status, "completed");
        assert!(r.message.unwrap_or_default().contains("success"));
    }

    #[test]
    fn test_fake_non_zero_exit() {
        let dir = TempDir::new().unwrap();
        let fake = create_fake_blender(dir.path(), "fail");

        let adapter =
            HeadlessBlenderAdapter::with_config("blender", PathBuf::from("blender/worker.py"));
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let result = adapter.run_job_with_fake(
            &fake,
            "test_job_fail",
            &sample_job_spec(),
            workspace.path(),
            output.path(),
            Some(10_000),
        );

        assert!(result.is_err());
        match result.unwrap_err() {
            BlenderError::NonZeroExit { exit_code, .. } => {
                assert_eq!(exit_code, Some(1));
            }
            other => panic!("Expected NonZeroExit, got {:?}", other),
        }
    }

    #[test]
    fn test_fake_timeout() {
        let dir = TempDir::new().unwrap();
        let fake = create_fake_blender(dir.path(), "timeout");

        let adapter =
            HeadlessBlenderAdapter::with_config("blender", PathBuf::from("blender/worker.py"));
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let result = adapter.run_job_with_fake(
            &fake,
            "test_job_timeout",
            &sample_job_spec(),
            workspace.path(),
            output.path(),
            Some(500), // very short timeout
        );

        assert!(result.is_err());
        match result.unwrap_err() {
            BlenderError::Timeout => {} // expected
            other => panic!("Expected Timeout, got {:?}", other),
        }
    }

    #[test]
    fn test_fake_malformed_result() {
        let dir = TempDir::new().unwrap();
        let fake = create_fake_blender(dir.path(), "malformed");

        let adapter =
            HeadlessBlenderAdapter::with_config("blender", PathBuf::from("blender/worker.py"));
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let result = adapter.run_job_with_fake(
            &fake,
            "test_job_malformed",
            &sample_job_spec(),
            workspace.path(),
            output.path(),
            Some(10_000),
        );

        assert!(result.is_err());
        match result.unwrap_err() {
            BlenderError::MalformedResult(_) => {} // expected
            other => panic!("Expected MalformedResult, got {:?}", other),
        }
    }

    #[test]
    fn test_fake_no_result_file() {
        let dir = TempDir::new().unwrap();
        let fake = create_fake_blender(dir.path(), "noresult");

        let adapter =
            HeadlessBlenderAdapter::with_config("blender", PathBuf::from("blender/worker.py"));
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let result = adapter.run_job_with_fake(
            &fake,
            "test_job_noresult",
            &sample_job_spec(),
            workspace.path(),
            output.path(),
            Some(10_000),
        );

        assert!(result.is_err());
        match result.unwrap_err() {
            BlenderError::MissingResultFile(_) => {} // expected
            other => panic!("Expected MissingResultFile, got {:?}", other),
        }
    }

    #[test]
    fn test_path_validation_rejects_absolute() {
        let err = validate_artifact_path("/absolute/path/file.blend").unwrap_err();
        match err {
            BlenderError::MalformedResult(msg) => {
                assert!(msg.contains("Absolute"));
            }
            other => panic!("Expected MalformedResult, got {:?}", other),
        }
    }

    #[test]
    fn test_path_validation_rejects_parent_traversal() {
        let err = validate_artifact_path("../../etc/passwd").unwrap_err();
        match err {
            BlenderError::MalformedResult(msg) => {
                assert!(msg.contains("Parent-traversal"));
            }
            other => panic!("Expected MalformedResult, got {:?}", other),
        }
    }

    #[test]
    fn test_path_validation_accepts_relative() {
        assert!(validate_artifact_path("blender/scenes/test.blend").is_ok());
        assert!(validate_artifact_path("jobs/job_001/output.glb").is_ok());
        assert!(validate_artifact_path("file.blend").is_ok());
    }

    #[test]
    fn test_write_job_json_structure() {
        let dir = TempDir::new().unwrap();
        let job_dir = dir.path().join("test_job_write");
        std::fs::create_dir_all(&job_dir).unwrap();

        let job_json_path = job_dir.join("job.json");
        let spec = sample_job_spec();

        let worker_input = serde_json::json!({
            "job_id": "test_write_001",
            "spec": spec.spec,
            "workspace_root": "/tmp/test",
        });

        let mut file = std::fs::File::create(&job_json_path).unwrap();
        serde_json::to_writer(&mut file, &worker_input).unwrap();
        file.flush().unwrap();

        // Read back and verify
        let content = std::fs::read_to_string(&job_json_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["job_id"], "test_write_001");
        assert!(parsed["spec"]["operations"].is_array());
        assert_eq!(parsed["spec"]["operations"][0]["op"], "create_box");
    }

    #[test]
    fn test_check_binary_not_found() {
        let adapter =
            HeadlessBlenderAdapter::with_config("nonexistent_blender_xyzzy", PathBuf::new());
        let result = adapter.check_binary();
        assert!(result.is_err());
        match result.unwrap_err() {
            BlenderError::BinaryNotFound(_) => {} // expected
            other => panic!("Expected BinaryNotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_validate_spec_path_absolute_rejected() {
        let err = validate_spec_path("/etc/passwd").unwrap_err();
        assert!(format!("{err}").contains("Absolute"));
    }

    #[test]
    fn test_validate_spec_path_parent_traversal_rejected() {
        let err = validate_spec_path("../outside/file.blend").unwrap_err();
        assert!(format!("{err}").contains("Parent-traversal"));
    }

    #[test]
    fn test_validate_spec_path_relative_accepted() {
        assert!(validate_spec_path("blender/scenes/test.blend").is_ok());
        assert!(validate_spec_path("jobs/job_001/output.glb").is_ok());
    }

    fn spec_with_op(op_name: &str, path: Option<&str>) -> BlenderJobSpec {
        BlenderJobSpec {
            job_type: "test".into(),
            wait: true,
            timeout_ms: None,
            spec: BlenderSpecBody {
                operations: vec![BlenderOperation {
                    op: op_name.into(),
                    name: None,
                    size: None,
                    location: None,
                    color: None,
                    object: None,
                    views: None,
                    path: path.map(|s| s.to_string()),
                }],
            },
        }
    }

    #[test]
    fn test_validate_job_spec_rejects_absolute_path() {
        let spec = spec_with_op("save_blend", Some("/absolute/path.blend"));
        let err = validate_job_spec_paths(&spec).unwrap_err();
        assert!(format!("{err}").contains("Absolute"));
    }

    #[test]
    fn test_validate_job_spec_rejects_parent_traversal() {
        let spec = spec_with_op("export_glb", Some("../../outside.glb"));
        let err = validate_job_spec_paths(&spec).unwrap_err();
        assert!(format!("{err}").contains("Parent-traversal"));
    }

    #[test]
    fn test_validate_job_spec_accepts_relative() {
        let spec = spec_with_op("save_blend", Some("blender/scenes/test.blend"));
        assert!(validate_job_spec_paths(&spec).is_ok());
    }

    #[test]
    fn test_validate_job_spec_skips_non_file_ops() {
        let spec = spec_with_op("create_box", None);
        assert!(validate_job_spec_paths(&spec).is_ok());
    }

    #[test]
    fn test_validate_job_spec_export_rejects_absolute() {
        let spec = spec_with_op("export_glb", Some("/etc/out.glb"));
        let err = validate_job_spec_paths(&spec).unwrap_err();
        assert!(format!("{err}").contains("Absolute"));
    }

    #[test]
    fn test_validate_job_spec_fake_rejects_absolute_before_run() {
        // Verify that run_job_with_fake rejects before launching fake binary
        let dir = TempDir::new().unwrap();
        let fake = create_fake_blender(dir.path(), "success");

        let adapter =
            HeadlessBlenderAdapter::with_config("blender", PathBuf::from("blender/worker.py"));

        let spec = spec_with_op("save_blend", Some("/etc/malicious.blend"));
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let result = adapter.run_job_with_fake(
            &fake,
            "job_spec_abs",
            &spec,
            workspace.path(),
            output.path(),
            Some(10_000),
        );

        assert!(result.is_err(), "expected spec validation error");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Absolute"),
            "expected Absolute rejection, got: {err_msg}"
        );
    }

    /// Real Blender smoke test — runs only if `blender` CLI is available.
    /// Verifies worker.py can be executed and produces a valid result.json.
    #[test]
    fn test_real_blender_worker_smoke() {
        // Check blender availability
        let blender_check = Command::new("blender")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if blender_check.is_err() || !blender_check.unwrap().success() {
            eprintln!("[skip] real_blender_worker_smoke: blender not available");
            return;
        }

        // Need the actual worker.py script
        let worker_script = resolve_worker_script();
        if !worker_script.exists() {
            eprintln!(
                "[skip] real_blender_worker_smoke: worker.py not found at {:?}",
                worker_script
            );
            return;
        }

        let adapter = HeadlessBlenderAdapter::with_config("blender", worker_script);
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let spec = BlenderJobSpec {
            job_type: "blender_smoke_scene".into(),
            wait: true,
            timeout_ms: Some(60_000),
            spec: BlenderSpecBody {
                operations: vec![
                    BlenderOperation {
                        op: "create_box".into(),
                        name: Some("smoke_cube".into()),
                        size: Some(vec![2.0, 1.0, 0.5]),
                        location: Some(vec![0.0, 0.0, 0.0]),
                        color: None,
                        object: None,
                        views: None,
                        path: None,
                    },
                    BlenderOperation {
                        op: "save_blend".into(),
                        name: None,
                        size: None,
                        location: None,
                        color: None,
                        object: None,
                        views: None,
                        path: Some("blender/scenes/smoke_test.blend".into()),
                    },
                ],
            },
        };

        let result = adapter.run_job(
            "real_blender_smoke_001",
            &spec,
            workspace.path(),
            output.path(),
            Some(60_000),
        );

        let r = result.expect("Real Blender smoke should succeed");
        assert_eq!(r.status, "completed", "Blender job failed: {:?}", r.error);
        assert!(
            r.artifacts.is_some() && !r.artifacts.as_ref().unwrap().is_empty(),
            "Expected at least one artifact"
        );

        // Verify result.json was created in the output directory
        let result_json_path = output
            .path()
            .join("real_blender_smoke_001")
            .join("result.json");
        assert!(
            result_json_path.exists(),
            "result.json should exist at {:?}",
            result_json_path
        );
        let content = std::fs::read_to_string(&result_json_path).expect("Should read result.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("result.json should be valid JSON");
        assert_eq!(parsed["status"], "completed");
        eprintln!("[info] Real Blender smoke artifacts: {:?}", r.artifacts);
    }
}
