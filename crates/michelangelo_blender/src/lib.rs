//! Michelangelo Blender — Blender subprocess adapter.
//!
//! Launches Blender as a bounded background subprocess, writes job specs,
//! captures stdout/stderr into per-job log files (never pipes), and validates
//! the worker result file before handing it back to the core.
//!
//! Supports fake executable mode for deterministic integration tests without a
//! real Blender installation.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use michelangelo_protocol::{
    BlenderArtifactDto, BlenderCapabilitiesDto, BlenderJobResult, BlenderJobSpec, SAFE_WORKER_OPS,
};

/// Built-in default wall-clock timeout for a Blender job (explicit arg >
/// `MICHELANGELO_BLENDER_TIMEOUT_MS` env var > this value).
pub const DEFAULT_TIMEOUT_MS: u64 = 300_000;

/// Bounded timeout for the lightweight capabilities probe.
pub const CAPABILITIES_TIMEOUT_MS: u64 = 30_000;

/// Bounded timeout for the `blender --version` probe.
///
/// A wedged binary must never block the single-threaded core.
pub const VERSION_TIMEOUT_MS: u64 = 10_000;

/// Maximum bytes of `--version` stdout parsed for the version string.
const VERSION_OUTPUT_MAX_BYTES: usize = 64 * 1024;

/// Environment variable overriding the default job timeout.
pub const TIMEOUT_ENV_VAR: &str = "MICHELANGELO_BLENDER_TIMEOUT_MS";

/// Maximum bytes of stderr quoted in error diagnostics (TZ 5.1: <= 4 KiB).
pub const STDERR_TAIL_BYTES: usize = 4 * 1024;

/// Maximum bytes of stdout used for diagnostics (TZ 5.1: <= 2 KiB).
pub const STDOUT_TAIL_BYTES: usize = 2 * 1024;

/// Marker line printed by `blender/worker.py --capabilities`.
pub const CAPABILITIES_MARKER: &str = "MICHELANGELO_CAPABILITIES:";

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
    Timeout { timeout_ms: u64, stderr: String },
    /// The expected result file was not created.
    MissingResultFile(String),
    /// The result file contains invalid JSON or is otherwise unparsable.
    MalformedResult(String),
    /// `result.json` reports a job id different from the requested one.
    JobIdMismatch { expected: String, actual: String },
    /// `result.json` reports an unknown job status.
    InvalidStatus(String),
    /// A `completed` result references an artifact that does not exist.
    MissingArtifact(String),
    /// A result artifact is malformed (bad type/extension, empty, size mismatch, unsafe path).
    InvalidArtifact(String),
    /// The requested job id is not safe to use as a directory name.
    InvalidJobId(String),
    /// An I/O error occurred.
    Io(std::io::Error),
}

impl std::fmt::Display for BlenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlenderError::BinaryNotFound(path) => {
                write!(f, "Blender binary not found at '{path}'")
            }
            BlenderError::NonZeroExit { exit_code, stderr } => {
                if stderr.is_empty() {
                    write!(f, "Blender exited with code {exit_code:?}")
                } else {
                    write!(f, "Blender exited with code {exit_code:?}: {}", stderr.trim())
                }
            }
            BlenderError::Timeout { timeout_ms, stderr } => {
                if stderr.is_empty() {
                    write!(f, "Blender subprocess timed out after {timeout_ms} ms")
                } else {
                    write!(
                        f,
                        "Blender subprocess timed out after {timeout_ms} ms; stderr tail: {}",
                        stderr.trim()
                    )
                }
            }
            BlenderError::MissingResultFile(path) => {
                write!(f, "Missing result file: {path}")
            }
            BlenderError::MalformedResult(msg) => {
                write!(f, "Malformed Blender result: {msg}")
            }
            BlenderError::JobIdMismatch { expected, actual } => write!(
                f,
                "Blender result job_id mismatch: expected '{expected}', got '{actual}'"
            ),
            BlenderError::InvalidStatus(status) => write!(
                f,
                "Blender result status '{status}' is invalid (expected completed, failed or cancelled)"
            ),
            BlenderError::MissingArtifact(path) => {
                write!(f, "Blender result reports a missing artifact: '{path}'")
            }
            BlenderError::InvalidArtifact(msg) => {
                write!(f, "Invalid Blender artifact: {msg}")
            }
            BlenderError::InvalidJobId(job_id) => write!(
                f,
                "Invalid job id '{job_id}': must match [A-Za-z0-9_-]{{1,128}}"
            ),
            BlenderError::Io(e) => write!(f, "I/O error: {e}"),
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

/// Payload printed by the worker in `--capabilities` mode.
#[derive(Debug, serde::Deserialize)]
struct CapabilitiesPayload {
    #[serde(default)]
    operations: Vec<String>,
    #[serde(default)]
    blender_version: Option<String>,
    #[serde(default)]
    python_version: Option<String>,
    #[serde(default)]
    gltf_export_available: Option<bool>,
    #[serde(default)]
    render_engines: Option<Vec<String>>,
}

/// Adapter for launching Blender as a bounded subprocess.
pub struct HeadlessBlenderAdapter {
    /// Path to the Blender executable (or name for PATH lookup).
    blender_bin: String,
    /// Path to the Python worker script.
    worker_script: PathBuf,
    /// Default timeout used when neither an explicit arg nor the env var is set.
    default_timeout_ms: u64,
    /// Bounded timeout for the `--version` probe.
    version_timeout_ms: u64,
}

impl HeadlessBlenderAdapter {
    /// Create a new adapter using `BLENDER_BIN` env var or default `"blender"`.
    ///
    /// The worker script is resolved relative to `CARGO_MANIFEST_DIR` or the
    /// current directory, looking for `blender/worker.py`.
    pub fn new() -> Self {
        let blender_bin = std::env::var("BLENDER_BIN").unwrap_or_else(|_| "blender".into());
        let worker_script = resolve_worker_script();
        Self {
            blender_bin,
            worker_script,
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            version_timeout_ms: VERSION_TIMEOUT_MS,
        }
    }

    /// Create an adapter with explicit configuration (primarily for testing).
    pub fn with_config(blender_bin: impl Into<String>, worker_script: impl Into<PathBuf>) -> Self {
        Self {
            blender_bin: blender_bin.into(),
            worker_script: worker_script.into(),
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            version_timeout_ms: VERSION_TIMEOUT_MS,
        }
    }

    /// Override the built-in default timeout (used by tests and embedders).
    pub fn with_default_timeout(mut self, timeout_ms: u64) -> Self {
        self.default_timeout_ms = timeout_ms.max(1);
        self
    }

    /// Override the bounded `--version` probe timeout (used by tests/embedders).
    pub fn with_version_timeout(mut self, timeout_ms: u64) -> Self {
        self.version_timeout_ms = timeout_ms.max(1);
        self
    }

    /// Return the configured Blender binary path.
    pub fn blender_bin(&self) -> &str {
        &self.blender_bin
    }

    /// Return the resolved worker script path.
    pub fn worker_script(&self) -> &Path {
        &self.worker_script
    }

    /// Resolve the effective timeout: explicit arg > env var > built-in default.
    pub fn effective_timeout_ms(&self, explicit_ms: Option<u64>) -> u64 {
        let env_value = std::env::var(TIMEOUT_ENV_VAR).ok();
        resolve_timeout_ms(explicit_ms, env_value.as_deref(), self.default_timeout_ms)
    }

    /// Check whether the Blender binary exists and is executable.
    ///
    /// The probe is bounded ([`VERSION_TIMEOUT_MS`]) and uses the same
    /// spawn/wait/kill-tree/log-file machinery as the capabilities probe, so a
    /// wedged binary cannot block the core. On Windows, `.cmd`/`.bat` programs
    /// are launched through `cmd /C call`, so batch-based fake executables work
    /// in production code paths.
    pub fn check_binary(&self) -> Result<String, BlenderError> {
        let temp_dir = unique_temp_dir("michelangelo-version");
        std::fs::create_dir_all(&temp_dir)?;
        let stdout_path = temp_dir.join("stdout.log");
        let stderr_path = temp_dir.join("stderr.log");
        let result = self.execute_version_probe(&stdout_path, &stderr_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
        result
    }

    fn execute_version_probe(
        &self,
        stdout_path: &Path,
        stderr_path: &Path,
    ) -> Result<String, BlenderError> {
        let mut cmd = program_command(Path::new(&self.blender_bin));
        cmd.arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::from(std::fs::File::create(stdout_path)?))
            .stderr(Stdio::from(std::fs::File::create(stderr_path)?));

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        let mut child = spawn_with_retry(&mut cmd, 3, Duration::from_millis(20)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BlenderError::BinaryNotFound(self.blender_bin.clone())
            } else {
                BlenderError::Io(e)
            }
        })?;

        let status = wait_with_timeout(
            &mut child,
            Duration::from_millis(self.version_timeout_ms),
            stderr_path,
        )?;

        if !status.success() {
            return Err(BlenderError::BinaryNotFound(self.blender_bin.clone()));
        }

        let stdout_text = read_tail(stdout_path, VERSION_OUTPUT_MAX_BYTES);
        let version = stdout_text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("")
            .to_string();

        if version.is_empty() {
            return Err(BlenderError::BinaryNotFound(self.blender_bin.clone()));
        }

        Ok(version)
    }

    /// Run a Blender job synchronously with optional timeout.
    ///
    /// Writes `job.json` to `output_dir/<job_id>/`, executes Blender with
    /// per-job `stdout.log`/`stderr.log` files, and validates `result.json`.
    pub fn run_job(
        &self,
        job_id: &str,
        job_spec: &BlenderJobSpec,
        workspace_root: &Path,
        output_dir: &Path,
        timeout_ms: Option<u64>,
    ) -> Result<BlenderJobResult, BlenderError> {
        let cmd = self.base_command(Path::new(&self.blender_bin));
        self.run_command(
            cmd,
            job_id,
            job_spec,
            workspace_root,
            output_dir,
            timeout_ms,
        )
    }

    /// Execute a fake/canned Blender invocation for testing.
    /// Uses `fake_blender_path` instead of the real Blender binary but shares
    /// the complete production execution path with [`Self::run_job`].
    pub fn run_job_with_fake(
        &self,
        fake_blender: &Path,
        job_id: &str,
        job_spec: &BlenderJobSpec,
        workspace_root: &Path,
        output_dir: &Path,
        timeout_ms: Option<u64>,
    ) -> Result<BlenderJobResult, BlenderError> {
        let cmd = self.base_command(fake_blender);
        self.run_command(
            cmd,
            job_id,
            job_spec,
            workspace_root,
            output_dir,
            timeout_ms,
        )
    }

    /// Probe the configured binary and worker for actual capabilities.
    ///
    /// Missing binary and failed probe are normal capability responses, not
    /// request failures: the caller always gets a descriptor with an `error`
    /// field and the static operation whitelist as a fallback.
    pub fn probe_capabilities(&self) -> BlenderCapabilitiesDto {
        let version = match self.check_binary() {
            Ok(version) => version,
            Err(error @ BlenderError::BinaryNotFound(_)) => {
                return BlenderCapabilitiesDto::unavailable_with_static_fallback(
                    false,
                    format!("{error}"),
                );
            }
            Err(error) => {
                // The binary exists but the bounded version probe failed
                // (e.g. hung and was killed). Report a degraded capability set.
                return BlenderCapabilitiesDto {
                    blender_version: None,
                    bpy_available: false,
                    supported_operations: static_operation_list(),
                    binary_found: true,
                    python_version: None,
                    gltf_export_available: None,
                    render_engines: None,
                    error: Some(format!("{error}")),
                };
            }
        };

        match self.run_capabilities_probe() {
            Ok(payload) => BlenderCapabilitiesDto {
                blender_version: payload.blender_version.or(Some(version)),
                bpy_available: true,
                supported_operations: payload.operations,
                binary_found: true,
                python_version: payload.python_version,
                gltf_export_available: payload.gltf_export_available,
                render_engines: payload.render_engines,
                error: None,
            },
            Err(error) => BlenderCapabilitiesDto {
                blender_version: Some(version),
                bpy_available: false,
                supported_operations: static_operation_list(),
                binary_found: true,
                python_version: None,
                gltf_export_available: None,
                render_engines: None,
                error: Some(format!("{error}")),
            },
        }
    }

    // -- internal -----------------------------------------------------------

    /// Build the common Blender argv prefix for a job invocation.
    fn base_command(&self, program: &Path) -> Command {
        let mut cmd = program_command(program);
        cmd.arg("--background")
            .arg("--factory-startup")
            .arg("--python")
            .arg(&self.worker_script)
            .arg("--");
        cmd
    }

    /// Shared implementation for real and fake launches.
    fn run_command(
        &self,
        mut cmd: Command,
        job_id: &str,
        job_spec: &BlenderJobSpec,
        workspace_root: &Path,
        output_dir: &Path,
        timeout_ms: Option<u64>,
    ) -> Result<BlenderJobResult, BlenderError> {
        validate_job_id(job_id)?;
        validate_job_spec_paths(job_spec)?;

        if !self.worker_script.is_file() {
            return Err(BlenderError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Worker script not found: {}", self.worker_script.display()),
            )));
        }

        let workspace_root_str = workspace_root.to_str().ok_or_else(|| {
            BlenderError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "workspace root is not valid UTF-8: {}",
                    workspace_root.display()
                ),
            ))
        })?;

        let job_dir = output_dir.join(job_id);
        std::fs::create_dir_all(&job_dir)?;

        let job_json_path = job_dir.join("job.json");
        let result_json_path = job_dir.join("result.json");
        let stdout_path = job_dir.join("stdout.log");
        let stderr_path = job_dir.join("stderr.log");

        // Remove stale files from a previous attempt before launching.
        remove_if_exists(&result_json_path)?;
        remove_if_exists(&stdout_path)?;
        remove_if_exists(&stderr_path)?;

        let worker_input = serde_json::json!({
            "job_id": job_id,
            "spec": job_spec.spec,
            "workspace_root": workspace_root_str,
        });
        std::fs::write(&job_json_path, serde_json::to_vec(&worker_input)?)?;

        // Blender argv tail: <job.json> <result.json> (after the `--` separator
        // added by `base_command`).
        cmd.arg(&job_json_path).arg(&result_json_path);

        let stdout_file = std::fs::File::create(&stdout_path)?;
        let stderr_file = std::fs::File::create(&stderr_path)?;
        cmd.stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Make the child its own process-group leader so a timeout can
            // SIGKILL the whole tree (Blender spawns helper processes).
            cmd.process_group(0);
        }

        let mut child = spawn_with_retry(&mut cmd, 3, Duration::from_millis(20)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BlenderError::BinaryNotFound(self.blender_bin.clone())
            } else {
                BlenderError::Io(e)
            }
        })?;

        let effective_timeout = self.effective_timeout_ms(timeout_ms);
        let exit_status = wait_with_timeout(
            &mut child,
            Duration::from_millis(effective_timeout),
            &stderr_path,
        )?;
        let exit_code = exit_status.code();
        let stderr_tail = read_tail(&stderr_path, STDERR_TAIL_BYTES);
        let stdout_tail = read_tail(&stdout_path, STDOUT_TAIL_BYTES);

        let result_bytes = match std::fs::read(&result_json_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(BlenderError::Io(e)),
        };

        // TZ 5.1: absent + non-zero exit -> NonZeroExit; absent + zero exit ->
        // MissingResultFile.
        let Some(bytes) = result_bytes else {
            if exit_code != Some(0) {
                return Err(BlenderError::NonZeroExit {
                    exit_code,
                    stderr: stderr_tail,
                });
            }
            return Err(BlenderError::MissingResultFile(format!(
                "{} (stdout tail: {})",
                result_json_path.display(),
                if stdout_tail.is_empty() {
                    "<empty>"
                } else {
                    &stdout_tail
                }
            )));
        };

        let content = String::from_utf8_lossy(&bytes).into_owned();
        let mut result: BlenderJobResult = match serde_json::from_str(&content) {
            Ok(result) => result,
            Err(e) => {
                if exit_code != Some(0) {
                    return Err(BlenderError::NonZeroExit {
                        exit_code,
                        stderr: stderr_tail,
                    });
                }
                return Err(BlenderError::MalformedResult(format!(
                    "failed to parse result JSON: {e} (content starts with: {:?})",
                    truncate_chars(&content, 200)
                )));
            }
        };

        validate_result(&mut result, job_id, workspace_root, exit_code, &stderr_tail)?;
        Ok(result)
    }

    /// Run the bounded `--capabilities` probe, capturing output to temp files.
    fn run_capabilities_probe(&self) -> Result<CapabilitiesPayload, BlenderError> {
        let temp_dir = unique_temp_dir("michelangelo-capabilities");
        std::fs::create_dir_all(&temp_dir)?;
        let stdout_path = temp_dir.join("stdout.log");
        let stderr_path = temp_dir.join("stderr.log");
        let result = self.execute_capabilities_probe(&stdout_path, &stderr_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
        result
    }

    fn execute_capabilities_probe(
        &self,
        stdout_path: &Path,
        stderr_path: &Path,
    ) -> Result<CapabilitiesPayload, BlenderError> {
        let mut cmd = program_command(Path::new(&self.blender_bin));
        cmd.arg("--background")
            .arg("--factory-startup")
            .arg("--python")
            .arg(&self.worker_script)
            .arg("--")
            .arg("--capabilities")
            .stdin(Stdio::null())
            .stdout(Stdio::from(std::fs::File::create(stdout_path)?))
            .stderr(Stdio::from(std::fs::File::create(stderr_path)?));

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        let mut child = spawn_with_retry(&mut cmd, 3, Duration::from_millis(20)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BlenderError::BinaryNotFound(self.blender_bin.clone())
            } else {
                BlenderError::Io(e)
            }
        })?;

        let exit_status = wait_with_timeout(
            &mut child,
            Duration::from_millis(CAPABILITIES_TIMEOUT_MS),
            stderr_path,
        )?;
        let stderr_tail = read_tail(stderr_path, STDERR_TAIL_BYTES);

        if !exit_status.success() {
            return Err(BlenderError::NonZeroExit {
                exit_code: exit_status.code(),
                stderr: stderr_tail,
            });
        }

        let stdout_text = read_tail(stdout_path, 512 * 1024);
        let marker_line = stdout_text
            .lines()
            .rev()
            .find(|line| line.contains(CAPABILITIES_MARKER))
            .ok_or_else(|| {
                BlenderError::MalformedResult(format!(
                    "capabilities marker '{CAPABILITIES_MARKER}' not found in worker stdout; \
                     stderr tail: {stderr_tail}"
                ))
            })?;

        let json_part = marker_line
            .split_once(CAPABILITIES_MARKER)
            .map(|(_, rest)| rest.trim())
            .unwrap_or_else(|| marker_line.trim());
        serde_json::from_str(json_part)
            .map_err(|e| BlenderError::MalformedResult(format!("invalid capabilities JSON: {e}")))
    }
}

impl Default for HeadlessBlenderAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Program launch helpers (direct executable vs Windows batch wrapper)
// ---------------------------------------------------------------------------

/// How a program path must be launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgramLaunch {
    /// Launch the path directly (real Blender `.exe`, Unix scripts).
    Direct,
    /// Launch through `cmd /C call` (Windows `.cmd`/`.bat` scripts).
    #[cfg_attr(not(windows), allow(dead_code))]
    Batch,
}

/// Pure classification of a program path into a launch strategy.
///
/// Only Windows requires the batch wrapper: `CreateProcess` cannot execute
/// `.cmd`/`.bat` files directly. On other platforms every path is launched
/// directly (Unix scripts are executed via their shebang).
fn classify_program(program: &Path) -> ProgramLaunch {
    #[cfg(windows)]
    {
        if let Some(extension) = program.extension().and_then(|ext| ext.to_str()) {
            if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat") {
                return ProgramLaunch::Batch;
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = program;
    }
    ProgramLaunch::Direct
}

/// Build a `Command` for a program path, applying the batch wrapper on Windows.
///
/// This is used by every launch path (`--version`, capabilities probe, real
/// `run_job` and fake `run_job_with_fake`) so behavior is identical.
fn program_command(program: &Path) -> Command {
    #[cfg(windows)]
    {
        if classify_program(program) == ProgramLaunch::Batch {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg("call").arg(program);
            return cmd;
        }
    }
    Command::new(program)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Validate a job id: `[A-Za-z0-9_-]{1,128}` (safe as a directory name).
fn validate_job_id(job_id: &str) -> Result<(), BlenderError> {
    let valid = !job_id.is_empty()
        && job_id.len() <= 128
        && job_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(BlenderError::InvalidJobId(job_id.to_string()))
    }
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Read at most `max_bytes` from the tail of a file as lossy UTF-8.
fn read_tail(path: &Path, max_bytes: usize) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let start = length.saturating_sub(max_bytes as u64);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buffer = Vec::new();
    let mut limited = file.take(max_bytes as u64);
    if limited.read_to_end(&mut buffer).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&buffer).trim().to_string()
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()))
}

fn static_operation_list() -> Vec<String> {
    SAFE_WORKER_OPS.iter().map(|op| (*op).to_string()).collect()
}

/// Pure timeout resolution: explicit arg > env value > built-in default.
///
/// Values of `0` and unparsable env values are ignored (a bounded positive
/// timeout is always returned; the result is clamped to at least 1 ms).
fn resolve_timeout_ms(explicit_ms: Option<u64>, env_value: Option<&str>, default_ms: u64) -> u64 {
    explicit_ms
        .filter(|ms| *ms > 0)
        .or_else(|| {
            env_value
                .and_then(|value| value.trim().parse::<u64>().ok())
                .filter(|ms| *ms > 0)
        })
        .unwrap_or(default_ms)
        .max(1)
}

/// Wait for the child, polling `try_wait`; on timeout kill the whole tree and
/// wait for it to be reaped, then return a `Timeout` error with a stderr tail.
fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
    stderr_path: &Path,
) -> Result<ExitStatus, BlenderError> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => return Err(BlenderError::Io(e)),
        }

        if start.elapsed() >= timeout {
            kill_process_tree(child);
            let stderr = read_tail(stderr_path, STDERR_TAIL_BYTES);
            return Err(BlenderError::Timeout {
                timeout_ms: timeout.as_millis() as u64,
                stderr,
            });
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Kill the direct child and all of its descendants, then reap it.
fn kill_process_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let pid = child.id();
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .arg("/T")
            .arg("/F")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // The child was spawned as its own process-group leader (process_group(0)),
        // so killing the negative pid kills the whole tree.
        let pid = child.id() as libc::pid_t;
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    // Always reap the direct child, even if the tree kill above failed.
    let _ = child.kill();
    let _ = child.wait();
}

/// Validate and normalize a parsed worker result (TZ 5.1 / 5.7).
fn validate_result(
    result: &mut BlenderJobResult,
    requested_job_id: &str,
    workspace_root: &Path,
    exit_code: Option<i32>,
    stderr_tail: &str,
) -> Result<(), BlenderError> {
    if result.job_id != requested_job_id {
        return Err(BlenderError::JobIdMismatch {
            expected: requested_job_id.to_string(),
            actual: result.job_id.clone(),
        });
    }

    match result.status.as_str() {
        "completed" => {
            if exit_code != Some(0) {
                return Err(BlenderError::NonZeroExit {
                    exit_code,
                    stderr: stderr_tail.to_string(),
                });
            }
            validate_completed_artifacts(result, workspace_root)?;
        }
        "failed" | "cancelled" => {
            // A failed/cancelled job is a job-level result, not an adapter
            // error: sanitize partial artifact paths but never mask the error.
            sanitize_partial_artifacts(result, workspace_root);
        }
        other => return Err(BlenderError::InvalidStatus(other.to_string())),
    }

    Ok(())
}

fn validate_completed_artifacts(
    result: &BlenderJobResult,
    workspace_root: &Path,
) -> Result<(), BlenderError> {
    let Some(artifacts) = result.artifacts.as_deref() else {
        return Ok(());
    };

    let canonical_root = std::fs::canonicalize(workspace_root).map_err(BlenderError::Io)?;

    for artifact in artifacts {
        validate_artifact_path(&artifact.path)?;
        validate_artifact_type_and_extension(artifact)?;

        let candidate = workspace_root.join(&artifact.path);
        let canonical = std::fs::canonicalize(&candidate).map_err(|_| {
            BlenderError::MissingArtifact(format!(
                "reported artifact does not exist: '{}'",
                artifact.path
            ))
        })?;

        if !canonical.starts_with(&canonical_root) {
            return Err(BlenderError::InvalidArtifact(format!(
                "artifact '{}' resolves outside the workspace",
                artifact.path
            )));
        }

        let metadata = std::fs::metadata(&canonical)
            .map_err(|e| BlenderError::InvalidArtifact(format!("'{}': {e}", artifact.path)))?;
        if !metadata.is_file() {
            return Err(BlenderError::InvalidArtifact(format!(
                "artifact '{}' is not a regular file",
                artifact.path
            )));
        }
        if metadata.len() == 0 {
            return Err(BlenderError::InvalidArtifact(format!(
                "artifact '{}' is empty",
                artifact.path
            )));
        }
        if let Some(reported_size) = artifact.size_bytes {
            if reported_size != metadata.len() {
                return Err(BlenderError::InvalidArtifact(format!(
                    "artifact '{}' size mismatch: reported {reported_size}, actual {}",
                    artifact.path,
                    metadata.len()
                )));
            }
        }
    }

    Ok(())
}

/// Drop unsafe/malformed partial artifacts from failed/cancelled results.
fn sanitize_partial_artifacts(result: &mut BlenderJobResult, workspace_root: &Path) {
    let Some(artifacts) = result.artifacts.take() else {
        return;
    };
    let canonical_root = std::fs::canonicalize(workspace_root).ok();

    let safe: Vec<BlenderArtifactDto> = artifacts
        .into_iter()
        .filter(|artifact| {
            validate_artifact_path(&artifact.path).is_ok()
                && matches!(artifact.artifact_type.as_str(), "blend" | "glb" | "png")
                && {
                    let candidate = workspace_root.join(&artifact.path);
                    match std::fs::canonicalize(&candidate) {
                        Ok(canonical) => canonical_root
                            .as_deref()
                            .map(|root| canonical.starts_with(root))
                            .unwrap_or(false),
                        // Partial artifacts may legitimately not exist yet.
                        Err(_) => true,
                    }
                }
        })
        .collect();

    result.artifacts = if safe.is_empty() { None } else { Some(safe) };
}

/// Validate that an artifact path is workspace-relative and traversal-free.
fn validate_artifact_path(path: &str) -> Result<(), BlenderError> {
    if path.is_empty() {
        return Err(BlenderError::InvalidArtifact(
            "artifact path is empty".to_string(),
        ));
    }
    if path.contains('\0') {
        return Err(BlenderError::InvalidArtifact(format!(
            "artifact path contains NUL: '{path}'"
        )));
    }

    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(BlenderError::InvalidArtifact(format!(
            "Absolute artifact path rejected: '{path}'"
        )));
    }
    for component in parsed.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            _ => {
                return Err(BlenderError::InvalidArtifact(format!(
                    "unsafe artifact path rejected (must be workspace-relative without '..'): '{path}'"
                )));
            }
        }
    }
    Ok(())
}

/// Validate that the artifact type is known and matches its file extension.
fn validate_artifact_type_and_extension(artifact: &BlenderArtifactDto) -> Result<(), BlenderError> {
    let expected_extension = match artifact.artifact_type.as_str() {
        "blend" => "blend",
        "glb" => "glb",
        "png" => "png",
        other => {
            return Err(BlenderError::InvalidArtifact(format!(
                "artifact '{}' has unknown type '{other}' (expected blend, glb or png)",
                artifact.path
            )));
        }
    };

    match Path::new(&artifact.path)
        .extension()
        .and_then(|ext| ext.to_str())
    {
        Some(extension) if extension.eq_ignore_ascii_case(expected_extension) => Ok(()),
        _ => Err(BlenderError::InvalidArtifact(format!(
            "artifact '{}' type '{}' does not match its file extension",
            artifact.path, artifact.artifact_type
        ))),
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

/// Validate a single spec path (must be relative, no parent/root components).
///
/// Component-based (aligned with the router rule): a `..` *inside a file name*
/// (e.g. `model.v1..2.blend`) is a normal component and is accepted, while
/// `..`, root and prefix components are rejected.
fn validate_spec_path(path: &str) -> Result<(), BlenderError> {
    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(BlenderError::MalformedResult(format!(
            "Absolute path in job spec rejected: '{path}'"
        )));
    }
    for component in parsed.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(BlenderError::MalformedResult(format!(
                    "Parent-traversal path in job spec rejected: '{path}'"
                )));
            }
            _ => {
                return Err(BlenderError::MalformedResult(format!(
                    "Root/prefix path component in job spec rejected: '{path}'"
                )));
            }
        }
    }
    Ok(())
}

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
    match cmd.spawn() {
        Ok(child) => return Ok(child),
        Err(e) if e.raw_os_error() == Some(26) => {
            // ETXTBUSY — wait briefly and try again
            std::thread::sleep(delay);
        }
        Err(e) => return Err(e),
    }
    cmd.spawn()
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

/// Creates a temp executable for deterministic adapter testing.
///
/// Platform-specific: a Bash script (`fake_blender.sh`) on Unix, a batch
/// wrapper plus PowerShell implementation (`fake_blender.cmd`/`.ps1`) on
/// Windows. Both read the job's `job.json` (job id + workspace root), create
/// the artifact they report, and honour the worker result contract.
///
/// Supported modes:
/// - `"success"` — creates the artifact and writes a completed result
/// - `"fail"` — exits 1 without writing a result
/// - `"malformed"` — writes garbage to result.json and exits 0
/// - `"noresult"` — exits 0 without writing result.json
/// - `"failed_result"` — writes a failed result and exits 1
/// - `"cancelled_result"` — writes a cancelled result and exits 1
/// - `"failed_with_artifact"` — failed result with an existing partial artifact
/// - `"failed_unsafe_artifact"` — failed result listing a `..` artifact
/// - `"wrong_job_id"` — completed result for a different job id
/// - `"missing_artifact"` — completed result listing a missing artifact
/// - `"empty_artifact"` — completed result listing a zero-byte artifact
/// - `"wrong_size"` — completed result with a mismatching `size_bytes`
/// - `"bad_type"` — completed result with an unknown artifact type
/// - `"artifact_outside"` — completed result listing `../evil.blend`
/// - `"completed_nonzero"` — valid completed result but exit code 1
/// - `"malformed_nonzero"` — malformed result JSON and exit code 1
/// - `"huge_output"` — writes >2 MiB to both stdout and stderr, then succeeds
/// - `"fail_long_stderr"` — exits 1 with an 8 KiB stderr
/// - `"timeout"` — sleeps 10 s (used with a short timeout)
/// - `"timeout_tree"` — spawns a descendant (PID recorded in `<result>.childpid`) and sleeps
/// - `"capfail"` — fails the capabilities probe (but `--version` works)
/// - `"version_hang"` — hangs on `--version` (bounded version-probe test)
pub fn create_fake_blender(dir: &Path, mode: &str) -> PathBuf {
    #[cfg(unix)]
    {
        create_fake_blender_unix(dir, mode)
    }
    #[cfg(windows)]
    {
        create_fake_blender_windows(dir, mode)
    }
}

/// Записывает fake-исполняемый файл с явным `sync`, чтобы избежать ETXTBUSY
/// при параллельном запуске тестов.
fn write_fake_executable(path: &Path, contents: &[u8]) {
    {
        let mut file = std::fs::File::create(path).expect("create fake blender script");
        file.write_all(contents).expect("write fake blender script");
        file.sync_all().expect("sync fake blender script");
    }

    // Also sync the parent directory so the new file is durable.
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

/// Unix fake: Bash script with shebang (`fake_blender.sh`); the mode is baked in.
#[cfg(unix)]
fn create_fake_blender_unix(dir: &Path, mode: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let script = dir.join("fake_blender.sh");
    let contents = UNIX_FAKE_SCRIPT.replace("__MODE__", mode);
    write_fake_executable(&script, contents.as_bytes());
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("make fake blender executable");
    script
}

/// Windows fake: ASCII `.cmd` wrapper (Unicode-safe: paths travel as `%~N`
/// arguments and environment variables, never as script text) plus a
/// mode-baked PowerShell implementation (`fake_blender.ps1`).
#[cfg(windows)]
fn create_fake_blender_windows(dir: &Path, mode: &str) -> PathBuf {
    let wrapper = dir.join("fake_blender.cmd");
    let implementation = dir.join("fake_blender.ps1");
    write_fake_executable(&wrapper, WINDOWS_FAKE_WRAPPER.as_bytes());
    write_fake_executable(
        &implementation,
        WINDOWS_FAKE_SCRIPT.replace("__MODE__", mode).as_bytes(),
    );
    wrapper
}

/// Unix fake Blender implementation. `MODE` is substituted at generation time.
#[cfg(unix)]
const UNIX_FAKE_SCRIPT: &str = r#"#!/bin/bash
# Fake Blender executable used by michelangelo_blender adapter tests.
# Never launches Blender; emulates the worker result contract deterministically.
MODE='__MODE__'
job_json="${@: -2:1}"
result_json="${@: -1}"

if [ "$result_json" = "--version" ]; then
    if [ "$MODE" = "version_hang" ]; then
        echo 'Fake Blender: version hang' >&2
        sleep 30
        exit 0
    fi
    echo 'Blender fake 0.0.0'
    exit 0
fi

if [ "$result_json" = "--capabilities" ]; then
    if [ "$MODE" = "capfail" ]; then
        echo 'fake capabilities probe failure' >&2
        exit 1
    fi
    echo 'MICHELANGELO_CAPABILITIES:{"operations":["create_box","export_glb","get_scene_summary","render_views","save_blend","set_camera_set","set_material"],"blender_version":"Blender fake 0.0.0","python_version":"3.13.0","gltf_export_available":true,"render_engines":["BLENDER_WORKBENCH"]}'
    exit 0
fi

if [ -z "$job_json" ] || [ -z "$result_json" ]; then
    echo 'fake_blender: missing job/result arguments' >&2
    exit 2
fi

job_id=$(sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p' "$job_json" | head -n 1)
workspace_root=$(sed -n 's/.*"workspace_root":"\([^"]*\)".*/\1/p' "$job_json" | head -n 1)

write_artifact() {
    local relative_path="$1"
    local content="$2"
    local absolute_path="$workspace_root/$relative_path"
    mkdir -p "$(dirname "$absolute_path")"
    printf '%s' "$content" > "$absolute_path"
    wc -c < "$absolute_path" | tr -d '[:space:]'
}

write_success_result() {
    local reported_id="$1"
    local size
    size=$(write_artifact 'blender/scenes/smoke.blend' 'FAKE_BLEND_ARTIFACT_DATA') || exit 3
    printf '{"job_id":"%s","status":"completed","message":"Fake Blender success","artifacts":[{"path":"blender/scenes/smoke.blend","type":"blend","size_bytes":%s}],"scene_summary":{"objects":1,"meshes":1,"triangles":12,"vertices":24}}' "$reported_id" "$size" > "$result_json"
}

case "$MODE" in
    success)
        write_success_result "$job_id"
        exit 0
        ;;
    huge_output)
        yes x | head -c 2097152
        yes y | head -c 2097152 >&2
        write_success_result "$job_id"
        exit 0
        ;;
    completed_nonzero)
        write_success_result "$job_id"
        exit 1
        ;;
    wrong_job_id)
        write_success_result 'wrong-job-id'
        exit 0
        ;;
    missing_artifact)
        printf '{"job_id":"%s","status":"completed","message":"Fake missing artifact","artifacts":[{"path":"blender/scenes/missing.blend","type":"blend","size_bytes":1024}]}' "$job_id" > "$result_json"
        exit 0
        ;;
    empty_artifact)
        size=$(write_artifact 'blender/scenes/empty.blend' '') || exit 3
        printf '{"job_id":"%s","status":"completed","message":"Fake empty artifact","artifacts":[{"path":"blender/scenes/empty.blend","type":"blend","size_bytes":%s}]}' "$job_id" "$size" > "$result_json"
        exit 0
        ;;
    wrong_size)
        write_artifact 'blender/scenes/smoke.blend' 'FAKE_BLEND_ARTIFACT_DATA' > /dev/null
        printf '{"job_id":"%s","status":"completed","message":"Fake wrong size","artifacts":[{"path":"blender/scenes/smoke.blend","type":"blend","size_bytes":999999}]}' "$job_id" > "$result_json"
        exit 0
        ;;
    bad_type)
        size=$(write_artifact 'blender/scenes/smoke.blend' 'FAKE_BLEND_ARTIFACT_DATA') || exit 3
        printf '{"job_id":"%s","status":"completed","message":"Fake bad type","artifacts":[{"path":"blender/scenes/smoke.blend","type":"exe","size_bytes":%s}]}' "$job_id" "$size" > "$result_json"
        exit 0
        ;;
    artifact_outside)
        printf '{"job_id":"%s","status":"completed","message":"Fake outside artifact","artifacts":[{"path":"../evil.blend","type":"blend","size_bytes":1}]}' "$job_id" > "$result_json"
        exit 0
        ;;
    failed_result)
        printf '{"job_id":"%s","status":"failed","message":"Fake failure","error":"Fake worker failure","artifacts":[]}' "$job_id" > "$result_json"
        exit 1
        ;;
    cancelled_result)
        printf '{"job_id":"%s","status":"cancelled","message":"Fake cancellation","artifacts":[]}' "$job_id" > "$result_json"
        exit 1
        ;;
    failed_with_artifact)
        size=$(write_artifact 'blender/scenes/partial.blend' 'PARTIAL_DATA') || exit 3
        printf '{"job_id":"%s","status":"failed","error":"Fake worker failure","artifacts":[{"path":"blender/scenes/partial.blend","type":"blend","size_bytes":%s}]}' "$job_id" "$size" > "$result_json"
        exit 1
        ;;
    failed_unsafe_artifact)
        printf '{"job_id":"%s","status":"failed","error":"Fake worker failure","artifacts":[{"path":"../evil.blend","type":"blend","size_bytes":1}]}' "$job_id" > "$result_json"
        exit 1
        ;;
    malformed)
        printf '{this is not json' > "$result_json"
        exit 0
        ;;
    malformed_nonzero)
        printf '{this is not json' > "$result_json"
        exit 1
        ;;
    noresult)
        echo 'Fake Blender: no result' >&2
        exit 0
        ;;
    fail)
        echo 'Fake Blender: fail' >&2
        exit 1
        ;;
    fail_long_stderr)
        head -c 8000 /dev/zero | tr '\0' 'E' >&2
        echo '' >&2
        exit 1
        ;;
    timeout)
        echo 'Fake Blender: timeout' >&2
        sleep 10
        exit 0
        ;;
    timeout_tree)
        echo 'Fake Blender: timeout tree' >&2
        ( sleep 30 ) &
        echo "$!" > "${result_json}.childpid"
        sleep 30
        exit 0
        ;;
    *)
        echo "fake_blender: unknown mode '$MODE'" >&2
        exit 3
        ;;
esac
"#;

/// Windows fake wrapper: parses the last two argv entries into env vars
/// (Unicode-safe argument passing) and delegates to `fake_blender.ps1`.
///
/// `%~dp0` must be captured before `shift` runs: SHIFT also shifts `%0`, so a
/// late `%~dp0` would point at the working directory (or an argument) instead
/// of the script directory.
#[cfg(windows)]
const WINDOWS_FAKE_WRAPPER: &str = r#"@echo off
setlocal
set "SCRIPT_DIR=%~dp0"
set "JOB_JSON="
set "RESULT_JSON="
:parse_args
if "%~1"=="" goto args_done
set "JOB_JSON=%RESULT_JSON%"
set "RESULT_JSON=%~1"
shift
goto parse_args
:args_done
powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%SCRIPT_DIR%fake_blender.ps1"
exit /b %ERRORLEVEL%
"#;

/// Windows fake implementation (ASCII-only script; `MODE` substituted).
#[cfg(windows)]
const WINDOWS_FAKE_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
$Mode = '__MODE__'
$CapabilitiesJson = '{"operations":["create_box","export_glb","get_scene_summary","render_views","save_blend","set_camera_set","set_material"],"blender_version":"Blender fake 0.0.0","python_version":"3.13.0","gltf_export_available":true,"render_engines":["BLENDER_WORKBENCH"]}'
$lastArgument = [string]$env:RESULT_JSON
if ($lastArgument -eq '--version') {
    if ($Mode -eq 'version_hang') {
        [Console]::Error.WriteLine('Fake Blender: version hang')
        Start-Sleep -Milliseconds 30000
        exit 0
    }
    [Console]::Out.WriteLine('Blender fake 0.0.0')
    exit 0
}
if ($lastArgument -eq '--capabilities') {
    if ($Mode -eq 'capfail') {
        [Console]::Error.WriteLine('fake capabilities probe failure')
        exit 1
    }
    [Console]::Out.WriteLine('MICHELANGELO_CAPABILITIES:' + $CapabilitiesJson)
    exit 0
}
$jobJsonPath = [string]$env:JOB_JSON
$resultPath = [string]$env:RESULT_JSON
if (-not $jobJsonPath -or -not $resultPath) {
    [Console]::Error.WriteLine('fake_blender: missing job/result arguments')
    exit 2
}
$job = Get-Content -Raw -Encoding UTF8 -LiteralPath $jobJsonPath | ConvertFrom-Json
$jobId = [string]$job.job_id
$workspaceRoot = [string]$job.workspace_root
# Canonicalized workspace roots may use the Windows verbatim prefix (\\?\).
# Strip it: PowerShell path cmdlets (Join-Path / New-Item) reject verbatim paths.
if ($workspaceRoot.StartsWith('\\?\UNC\', [System.StringComparison]::Ordinal)) {
    $workspaceRoot = '\\' + $workspaceRoot.Substring(8)
}
elseif ($workspaceRoot.StartsWith('\\?\', [System.StringComparison]::Ordinal)) {
    $workspaceRoot = $workspaceRoot.Substring(4)
}

function Write-Artifact([string]$relativePath, [string]$content) {
    $path = Join-Path $workspaceRoot $relativePath
    $parent = Split-Path -Parent $path
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    [IO.File]::WriteAllText($path, $content)
    return (Get-Item -LiteralPath $path).Length
}

function Write-Raw([string]$text) {
    [IO.File]::WriteAllText($resultPath, $text)
}

function Write-SuccessResult([string]$reportedId) {
    $size = Write-Artifact 'blender/scenes/smoke.blend' 'FAKE_BLEND_ARTIFACT_DATA'
    $json = '{"job_id":"' + $reportedId + '","status":"completed","message":"Fake Blender success","artifacts":[{"path":"blender/scenes/smoke.blend","type":"blend","size_bytes":' + $size + '}],"scene_summary":{"objects":1,"meshes":1,"triangles":12,"vertices":24}}'
    Write-Raw $json
}

switch ($Mode) {
    'success' { Write-SuccessResult $jobId; exit 0 }
    'huge_output' {
        [Console]::Out.Write(('x' * 2097152))
        [Console]::Error.Write(('y' * 2097152))
        Write-SuccessResult $jobId
        exit 0
    }
    'completed_nonzero' { Write-SuccessResult $jobId; exit 1 }
    'wrong_job_id' { Write-SuccessResult 'wrong-job-id'; exit 0 }
    'missing_artifact' {
        Write-Raw ('{"job_id":"' + $jobId + '","status":"completed","message":"Fake missing artifact","artifacts":[{"path":"blender/scenes/missing.blend","type":"blend","size_bytes":1024}]}')
        exit 0
    }
    'empty_artifact' {
        $size = Write-Artifact 'blender/scenes/empty.blend' ''
        Write-Raw ('{"job_id":"' + $jobId + '","status":"completed","message":"Fake empty artifact","artifacts":[{"path":"blender/scenes/empty.blend","type":"blend","size_bytes":' + $size + '}]}')
        exit 0
    }
    'wrong_size' {
        $null = Write-Artifact 'blender/scenes/smoke.blend' 'FAKE_BLEND_ARTIFACT_DATA'
        Write-Raw ('{"job_id":"' + $jobId + '","status":"completed","message":"Fake wrong size","artifacts":[{"path":"blender/scenes/smoke.blend","type":"blend","size_bytes":999999}]}')
        exit 0
    }
    'bad_type' {
        $size = Write-Artifact 'blender/scenes/smoke.blend' 'FAKE_BLEND_ARTIFACT_DATA'
        Write-Raw ('{"job_id":"' + $jobId + '","status":"completed","message":"Fake bad type","artifacts":[{"path":"blender/scenes/smoke.blend","type":"exe","size_bytes":' + $size + '}]}')
        exit 0
    }
    'artifact_outside' {
        Write-Raw ('{"job_id":"' + $jobId + '","status":"completed","message":"Fake outside artifact","artifacts":[{"path":"../evil.blend","type":"blend","size_bytes":1}]}')
        exit 0
    }
    'failed_result' {
        Write-Raw ('{"job_id":"' + $jobId + '","status":"failed","message":"Fake failure","error":"Fake worker failure","artifacts":[]}')
        exit 1
    }
    'cancelled_result' {
        Write-Raw ('{"job_id":"' + $jobId + '","status":"cancelled","message":"Fake cancellation","artifacts":[]}')
        exit 1
    }
    'failed_with_artifact' {
        $size = Write-Artifact 'blender/scenes/partial.blend' 'PARTIAL_DATA'
        Write-Raw ('{"job_id":"' + $jobId + '","status":"failed","error":"Fake worker failure","artifacts":[{"path":"blender/scenes/partial.blend","type":"blend","size_bytes":' + $size + '}]}')
        exit 1
    }
    'failed_unsafe_artifact' {
        Write-Raw ('{"job_id":"' + $jobId + '","status":"failed","error":"Fake worker failure","artifacts":[{"path":"../evil.blend","type":"blend","size_bytes":1}]}')
        exit 1
    }
    'malformed' { Write-Raw '{this is not json'; exit 0 }
    'malformed_nonzero' { Write-Raw '{this is not json'; exit 1 }
    'noresult' { [Console]::Error.WriteLine('Fake Blender: no result'); exit 0 }
    'fail' { [Console]::Error.WriteLine('Fake Blender: fail'); exit 1 }
    'fail_long_stderr' {
        [Console]::Error.Write(('E' * 8000))
        exit 1
    }
    'timeout' {
        [Console]::Error.WriteLine('Fake Blender: timeout')
        Start-Sleep -Milliseconds 10000
        exit 0
    }
    'timeout_tree' {
        [Console]::Error.WriteLine('Fake Blender: timeout tree')
        try {
            $child = Start-Process -FilePath 'ping.exe' -ArgumentList '-n','20','127.0.0.1' -WindowStyle Hidden -PassThru
            [IO.File]::WriteAllText($env:RESULT_JSON + '.childpid', [string]$child.Id)
        } catch { }
        Start-Sleep -Milliseconds 10000
        exit 0
    }
    default {
        [Console]::Error.WriteLine('fake_blender: unknown mode ' + $Mode)
        exit 3
    }
}
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use michelangelo_protocol::{BlenderJobSpec, BlenderOperation, BlenderSpecBody};
    use std::sync::{Mutex, MutexGuard};
    use tempfile::TempDir;

    /// Serializes tests that mutate process-wide environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Возвращает абсолютный путь в синтаксисе текущей платформы.
    ///
    /// POSIX-путь используется как есть на Unix и получает префикс диска
    /// (например, `C:/etc/passwd`) на Windows, чтобы `Path::is_absolute()`
    /// оставался `true` на обеих платформах.
    fn absolute_path(posix_path: &str) -> String {
        #[cfg(windows)]
        {
            format!("C:{posix_path}")
        }
        #[cfg(unix)]
        {
            posix_path.to_string()
        }
    }

    fn test_adapter() -> HeadlessBlenderAdapter {
        HeadlessBlenderAdapter::with_config("blender", resolve_worker_script())
    }

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

    struct FakeRun {
        result: Result<BlenderJobResult, BlenderError>,
        workspace: TempDir,
        output: TempDir,
    }

    fn run_fake_job(
        mode: &str,
        job_id: &str,
        spec: &BlenderJobSpec,
        timeout_ms: Option<u64>,
    ) -> FakeRun {
        let fake_dir = TempDir::new().unwrap();
        let fake = create_fake_blender(fake_dir.path(), mode);
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let result = test_adapter().run_job_with_fake(
            &fake,
            job_id,
            spec,
            workspace.path(),
            output.path(),
            timeout_ms,
        );
        FakeRun {
            result,
            workspace,
            output,
        }
    }

    fn read_fake_child_pid(path: &Path, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    return Some(pid);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_until_process_gone(pid: u32, timeout: Duration) -> Option<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            match process_is_alive(pid) {
                Some(false) => return Some(true),
                None => return None,
                Some(true) if Instant::now() >= deadline => return Some(false),
                Some(true) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    #[cfg(unix)]
    fn process_is_alive(pid: u32) -> Option<bool> {
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if result == 0 {
            return Some(true);
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ESRCH) => Some(false),
            Some(libc::EPERM) => Some(true),
            _ => None,
        }
    }

    #[cfg(windows)]
    fn process_is_alive(pid: u32) -> Option<bool> {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Some(text.contains(&pid.to_string()))
    }

    fn completed_result_with_artifact(path: &str, artifact_type: &str) -> BlenderJobResult {
        BlenderJobResult {
            job_id: "artifact_job".into(),
            status: "completed".into(),
            message: None,
            artifacts: Some(vec![BlenderArtifactDto {
                path: path.to_string(),
                artifact_type: artifact_type.to_string(),
                size_bytes: None,
            }]),
            scene_summary: None,
            error: None,
        }
    }

    #[test]
    fn test_adapter_defaults_to_blender() {
        let _guard = env_guard();
        let previous = std::env::var("BLENDER_BIN").ok();
        std::env::remove_var("BLENDER_BIN");
        let adapter = HeadlessBlenderAdapter::new();
        assert_eq!(adapter.blender_bin(), "blender");
        if let Some(value) = previous {
            std::env::set_var("BLENDER_BIN", value);
        }
    }

    #[test]
    fn test_blender_bin_env_var() {
        let _guard = env_guard();
        let previous = std::env::var("BLENDER_BIN").ok();
        std::env::set_var("BLENDER_BIN", "/custom/blender");
        let adapter = HeadlessBlenderAdapter::new();
        assert_eq!(adapter.blender_bin(), "/custom/blender");
        match previous {
            Some(value) => std::env::set_var("BLENDER_BIN", value),
            None => std::env::remove_var("BLENDER_BIN"),
        }
    }

    #[test]
    fn test_adapter_with_config() {
        let adapter =
            HeadlessBlenderAdapter::with_config("my_blender", PathBuf::from("blender/worker.py"));
        assert_eq!(adapter.blender_bin(), "my_blender");
        assert_eq!(adapter.worker_script(), Path::new("blender/worker.py"));
    }

    #[test]
    fn test_resolve_timeout_precedence_is_pure() {
        // explicit > env > default, without touching process-wide env.
        assert_eq!(resolve_timeout_ms(Some(77), Some("4321"), 1_234), 77);
        assert_eq!(resolve_timeout_ms(None, Some("4321"), 1_234), 4_321);
        assert_eq!(resolve_timeout_ms(None, None, 1_234), 1_234);
        assert_eq!(resolve_timeout_ms(None, Some("not-a-number"), 1_234), 1_234);
        assert_eq!(resolve_timeout_ms(None, Some("0"), 1_234), 1_234);
        assert_eq!(resolve_timeout_ms(Some(0), Some("4321"), 1_234), 4_321);
        assert_eq!(resolve_timeout_ms(Some(0), Some("-1"), 0), 1);
        assert_eq!(resolve_timeout_ms(None, None, 0), 1);

        // The adapter delegates to the same helper (explicit path is env-free).
        let adapter = test_adapter().with_default_timeout(1_234);
        assert_eq!(adapter.effective_timeout_ms(Some(77)), 77);
    }

    #[test]
    fn test_classify_program_wrapper_selection() {
        assert_eq!(
            classify_program(Path::new("blender.exe")),
            ProgramLaunch::Direct
        );
        assert_eq!(
            classify_program(Path::new("blender")),
            ProgramLaunch::Direct
        );

        #[cfg(windows)]
        {
            assert_eq!(
                classify_program(Path::new("fake_blender.cmd")),
                ProgramLaunch::Batch
            );
            assert_eq!(
                classify_program(Path::new("fake_blender.BAT")),
                ProgramLaunch::Batch
            );
        }
        #[cfg(not(windows))]
        {
            // The batch wrapper is Windows-only; elsewhere the path is direct.
            assert_eq!(
                classify_program(Path::new("fake_blender.cmd")),
                ProgramLaunch::Direct
            );
        }
    }

    #[test]
    fn test_program_command_batch_wrapper_is_windows_only() {
        let program = Path::new("C:\\temp\\fake_blender.cmd");
        let cmd = program_command(program);

        #[cfg(windows)]
        {
            assert_eq!(cmd.get_program().to_string_lossy(), "cmd");
            let args: Vec<String> = cmd
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            assert_eq!(args, vec!["/C", "call", "C:\\temp\\fake_blender.cmd"]);
        }
        #[cfg(not(windows))]
        {
            assert_eq!(
                cmd.get_program().to_string_lossy(),
                "C:\\temp\\fake_blender.cmd"
            );
            assert_eq!(cmd.get_args().count(), 0);
        }
    }

    #[test]
    fn test_fake_success_creates_and_validates_artifact() {
        let run = run_fake_job("success", "test_job_001", &sample_job_spec(), Some(30_000));

        let result = run.result.expect("fake success should pass");
        assert_eq!(result.status, "completed");
        assert!(result
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("success"));

        let job_dir = run.output.path().join("test_job_001");
        assert!(job_dir.join("job.json").exists());
        assert!(job_dir.join("result.json").exists());
        assert!(job_dir.join("stdout.log").exists());
        assert!(job_dir.join("stderr.log").exists());

        let artifact = run.workspace.path().join("blender/scenes/smoke.blend");
        assert!(artifact.is_file(), "fake must create its artifact");
        let metadata = std::fs::metadata(&artifact).unwrap();
        assert!(metadata.len() > 0);
        let reported = result.artifacts.unwrap()[0].clone();
        assert_eq!(reported.size_bytes, Some(metadata.len()));
    }

    #[test]
    fn test_fake_supports_unicode_workspace_paths() {
        // The Windows fake is a `.cmd` + PowerShell implementation, so Unicode
        // temp paths (e.g. a Cyrillic user profile) must survive argv and JSON
        // encoding. Also exercises Unicode paths on Unix.
        let unicode_root = std::env::temp_dir().join("michelangelo-it1-юникод");
        std::fs::create_dir_all(&unicode_root).unwrap();
        {
            let fake_dir = TempDir::new_in(&unicode_root).unwrap();
            let fake = create_fake_blender(fake_dir.path(), "success");
            let workspace = TempDir::new_in(&unicode_root).unwrap();
            let output = TempDir::new_in(&unicode_root).unwrap();

            let result = test_adapter().run_job_with_fake(
                &fake,
                "unicode_job_001",
                &sample_job_spec(),
                workspace.path(),
                output.path(),
                Some(30_000),
            );
            let result = result.expect("unicode fake run should succeed");
            assert_eq!(result.status, "completed");
            assert!(workspace
                .path()
                .join("blender/scenes/smoke.blend")
                .is_file());
        }
        let _ = std::fs::remove_dir_all(&unicode_root);
    }

    #[test]
    fn test_fake_success_removes_stale_results_and_logs() {
        let fake_dir = TempDir::new().unwrap();
        let fake = create_fake_blender(fake_dir.path(), "noresult");
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let job_dir = output.path().join("stale_job");
        std::fs::create_dir_all(&job_dir).unwrap();
        std::fs::write(job_dir.join("result.json"), b"{stale garbage").unwrap();
        std::fs::write(job_dir.join("stdout.log"), b"stale stdout").unwrap();
        std::fs::write(job_dir.join("stderr.log"), b"stale stderr").unwrap();

        let result = test_adapter().run_job_with_fake(
            &fake,
            "stale_job",
            &sample_job_spec(),
            workspace.path(),
            output.path(),
            Some(30_000),
        );

        // If stale result.json survived, this would be MalformedResult; stale
        // logs would leak into the captured diagnostics.
        match result.unwrap_err() {
            BlenderError::MissingResultFile(_) => {}
            other => panic!("expected MissingResultFile, got: {other:?}"),
        }
        let stderr_log = std::fs::read_to_string(job_dir.join("stderr.log")).unwrap_or_default();
        assert!(!stderr_log.contains("stale stderr"));
        let stdout_log = std::fs::read_to_string(job_dir.join("stdout.log")).unwrap_or_default();
        assert!(!stdout_log.contains("stale stdout"));
    }

    #[test]
    fn test_fake_non_zero_exit() {
        let run = run_fake_job("fail", "test_job_fail", &sample_job_spec(), Some(30_000));
        match run.result.unwrap_err() {
            BlenderError::NonZeroExit { exit_code, .. } => {
                assert_eq!(exit_code, Some(1));
            }
            other => panic!("Expected NonZeroExit, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_long_stderr_is_capped() {
        let run = run_fake_job(
            "fail_long_stderr",
            "test_job_long_stderr",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::NonZeroExit { stderr, .. } => {
                assert!(
                    stderr.len() <= STDERR_TAIL_BYTES,
                    "stderr tail must be capped at {STDERR_TAIL_BYTES} bytes, got {}",
                    stderr.len()
                );
                assert!(stderr.starts_with('E'));
            }
            other => panic!("Expected NonZeroExit, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_timeout() {
        let run = run_fake_job("timeout", "test_job_timeout", &sample_job_spec(), Some(500));
        match run.result.unwrap_err() {
            BlenderError::Timeout { timeout_ms, stderr } => {
                assert_eq!(timeout_ms, 500);
                assert!(
                    stderr.contains("timeout"),
                    "timeout error must include a stderr tail, got: {stderr:?}"
                );
            }
            other => panic!("Expected Timeout, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_default_timeout_is_bounded() {
        // A user-set env timeout would override the builder default; skip
        // rather than mutate process-wide env in a parallel test run.
        if std::env::var(TIMEOUT_ENV_VAR).is_ok() {
            eprintln!("[skip] test_fake_default_timeout_is_bounded: {TIMEOUT_ENV_VAR} is set");
            return;
        }

        let fake_dir = TempDir::new().unwrap();
        let fake = create_fake_blender(fake_dir.path(), "timeout");
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let adapter = test_adapter().with_default_timeout(300);
        let result = adapter.run_job_with_fake(
            &fake,
            "default_timeout_job",
            &sample_job_spec(),
            workspace.path(),
            output.path(),
            None,
        );

        match result.unwrap_err() {
            BlenderError::Timeout { timeout_ms, .. } => assert_eq!(timeout_ms, 300),
            other => panic!("Expected Timeout, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_timeout_kills_process_tree_descendant() {
        let start = Instant::now();
        let run = run_fake_job(
            "timeout_tree",
            "test_job_timeout_tree",
            &sample_job_spec(),
            Some(1_500),
        );
        let elapsed = start.elapsed();
        match run.result.unwrap_err() {
            BlenderError::Timeout { .. } => {}
            other => panic!("Expected Timeout, got {other:?}"),
        }
        assert!(
            elapsed < Duration::from_secs(15),
            "timeout must return promptly even with a descendant, took {elapsed:?}"
        );

        // The fake records its descendant PID before sleeping. If the fake was
        // killed before it could write the file, skip the liveness check.
        let pid_file = run
            .output
            .path()
            .join("test_job_timeout_tree")
            .join("result.json.childpid");
        let Some(pid) = read_fake_child_pid(&pid_file, Duration::from_secs(5)) else {
            eprintln!("[skip] descendant PID was not recorded by the fake");
            return;
        };

        match wait_until_process_gone(pid, Duration::from_secs(10)) {
            Some(true) => {}
            Some(false) => panic!("descendant pid {pid} is still alive after timeout tree-kill"),
            None => eprintln!("[skip] process liveness check is unavailable on this host"),
        }
    }

    #[test]
    fn test_fake_version_hang_is_bounded() {
        let fake_dir = TempDir::new().unwrap();
        let fake = create_fake_blender(fake_dir.path(), "version_hang");
        let adapter = HeadlessBlenderAdapter::with_config(
            fake.to_string_lossy().to_string(),
            resolve_worker_script(),
        )
        .with_version_timeout(200);

        let start = Instant::now();
        let result = adapter.check_binary();
        let elapsed = start.elapsed();

        match result.unwrap_err() {
            BlenderError::Timeout { timeout_ms, stderr } => {
                assert_eq!(timeout_ms, 200);
                assert!(
                    stderr.contains("version hang"),
                    "timeout must carry a stderr tail, got {stderr:?}"
                );
            }
            other => panic!("Expected Timeout, got {other:?}"),
        }
        assert!(
            elapsed < Duration::from_secs(10),
            "bounded version probe took {elapsed:?}"
        );
    }

    #[test]
    fn test_fake_capabilities_version_hang_is_bounded() {
        let fake_dir = TempDir::new().unwrap();
        let fake = create_fake_blender(fake_dir.path(), "version_hang");
        let adapter = HeadlessBlenderAdapter::with_config(
            fake.to_string_lossy().to_string(),
            resolve_worker_script(),
        )
        .with_version_timeout(200);

        let start = Instant::now();
        let caps = adapter.probe_capabilities();
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(10),
            "capabilities probe must be bounded, took {elapsed:?}"
        );
        assert!(
            caps.binary_found,
            "binary exists even though the version probe timed out"
        );
        assert!(!caps.bpy_available);
        assert!(caps.error.is_some());
        assert_eq!(caps.supported_operations.len(), SAFE_WORKER_OPS.len());
    }

    #[test]
    fn test_fake_malformed_result() {
        let run = run_fake_job(
            "malformed",
            "test_job_malformed",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::MalformedResult(_) => {}
            other => panic!("Expected MalformedResult, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_malformed_result_with_nonzero_exit_is_non_zero_exit() {
        let run = run_fake_job(
            "malformed_nonzero",
            "test_job_malformed_nonzero",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::NonZeroExit { exit_code, .. } => {
                assert_eq!(exit_code, Some(1));
            }
            other => panic!("Expected NonZeroExit, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_no_result_file() {
        let run = run_fake_job(
            "noresult",
            "test_job_noresult",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::MissingResultFile(_) => {}
            other => panic!("Expected MissingResultFile, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_failed_status_is_job_level_result() {
        let run = run_fake_job(
            "failed_result",
            "test_job_failed",
            &sample_job_spec(),
            Some(30_000),
        );
        let result = run
            .result
            .expect("failed worker result must not be an adapter error");
        assert_eq!(result.status, "failed");
        assert_eq!(result.error.as_deref(), Some("Fake worker failure"));
    }

    #[test]
    fn test_fake_cancelled_status_is_job_level_result() {
        let run = run_fake_job(
            "cancelled_result",
            "test_job_cancelled",
            &sample_job_spec(),
            Some(30_000),
        );
        let result = run
            .result
            .expect("cancelled worker result must not be an adapter error");
        assert_eq!(result.status, "cancelled");
        assert!(result
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("cancellation"));
    }

    #[test]
    fn test_fake_failed_status_preserves_partial_artifacts() {
        let run = run_fake_job(
            "failed_with_artifact",
            "test_job_failed_artifact",
            &sample_job_spec(),
            Some(30_000),
        );
        let result = run.result.expect("failed result is Ok");
        assert_eq!(result.status, "failed");
        let artifacts = result.artifacts.unwrap_or_default();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].path, "blender/scenes/partial.blend");
        assert!(run
            .workspace
            .path()
            .join("blender/scenes/partial.blend")
            .is_file());
    }

    #[test]
    fn test_fake_failed_status_sanitizes_unsafe_artifact_without_masking_error() {
        let run = run_fake_job(
            "failed_unsafe_artifact",
            "test_job_failed_unsafe",
            &sample_job_spec(),
            Some(30_000),
        );
        let result = run.result.expect("failed result is Ok");
        assert_eq!(result.status, "failed");
        assert_eq!(result.error.as_deref(), Some("Fake worker failure"));
        assert!(result.artifacts.unwrap_or_default().is_empty());
    }

    #[test]
    fn test_fake_wrong_job_id_is_rejected() {
        let run = run_fake_job(
            "wrong_job_id",
            "test_job_identity",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::JobIdMismatch { expected, actual } => {
                assert_eq!(expected, "test_job_identity");
                assert_eq!(actual, "wrong-job-id");
            }
            other => panic!("Expected JobIdMismatch, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_missing_artifact_is_rejected() {
        let run = run_fake_job(
            "missing_artifact",
            "test_job_missing_artifact",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::MissingArtifact(message) => {
                assert!(message.contains("missing.blend"));
            }
            other => panic!("Expected MissingArtifact, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_empty_artifact_is_rejected() {
        let run = run_fake_job(
            "empty_artifact",
            "test_job_empty_artifact",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::InvalidArtifact(message) => assert!(message.contains("empty")),
            other => panic!("Expected InvalidArtifact, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_wrong_artifact_size_is_rejected() {
        let run = run_fake_job(
            "wrong_size",
            "test_job_wrong_size",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::InvalidArtifact(message) => assert!(message.contains("size mismatch")),
            other => panic!("Expected InvalidArtifact, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_bad_artifact_type_is_rejected() {
        let run = run_fake_job(
            "bad_type",
            "test_job_bad_type",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::InvalidArtifact(message) => assert!(message.contains("unknown type")),
            other => panic!("Expected InvalidArtifact, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_artifact_outside_workspace_is_rejected() {
        let run = run_fake_job(
            "artifact_outside",
            "test_job_outside",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::InvalidArtifact(message) => {
                assert!(message.contains("unsafe artifact path"));
            }
            other => panic!("Expected InvalidArtifact, got {other:?}"),
        }
    }

    #[test]
    fn test_completed_artifact_directory_is_rejected() {
        let workspace = TempDir::new().unwrap();
        let dir_path = workspace.path().join("blender/scenes/not_a_file.blend");
        std::fs::create_dir_all(&dir_path).unwrap();

        let result = completed_result_with_artifact("blender/scenes/not_a_file.blend", "blend");
        match validate_completed_artifacts(&result, workspace.path()).unwrap_err() {
            BlenderError::InvalidArtifact(message) => {
                assert!(message.contains("not a regular file"));
            }
            other => panic!("Expected InvalidArtifact, got {other:?}"),
        }
    }

    #[test]
    fn test_completed_artifact_type_extension_mismatch_is_rejected() {
        let workspace = TempDir::new().unwrap();
        let path = workspace.path().join("blender/scenes/mismatch.glb");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"data").unwrap();

        let result = completed_result_with_artifact("blender/scenes/mismatch.glb", "blend");
        match validate_completed_artifacts(&result, workspace.path()).unwrap_err() {
            BlenderError::InvalidArtifact(message) => {
                assert!(message.contains("does not match"));
            }
            other => panic!("Expected InvalidArtifact, got {other:?}"),
        }
    }

    #[test]
    fn test_completed_artifact_symlink_escape_is_rejected() {
        let workspace = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let outside_file = outside.path().join("outside.blend");
        std::fs::write(&outside_file, b"outside-data").unwrap();
        let link = workspace.path().join("link.blend");

        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(&outside_file, &link);
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_file(&outside_file, &link);

        if let Err(error) = link_result {
            eprintln!("[skip] symlink creation denied or unavailable: {error}");
            return;
        }

        let result = completed_result_with_artifact("link.blend", "blend");
        match validate_completed_artifacts(&result, workspace.path()).unwrap_err() {
            BlenderError::InvalidArtifact(message) => {
                assert!(
                    message.contains("outside the workspace"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("Expected InvalidArtifact for symlink escape, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_completed_with_nonzero_exit_is_rejected() {
        let run = run_fake_job(
            "completed_nonzero",
            "test_job_nonzero",
            &sample_job_spec(),
            Some(30_000),
        );
        match run.result.unwrap_err() {
            BlenderError::NonZeroExit { exit_code, .. } => assert_eq!(exit_code, Some(1)),
            other => panic!("Expected NonZeroExit, got {other:?}"),
        }
    }

    #[test]
    fn test_fake_huge_output_does_not_deadlock() {
        let start = Instant::now();
        let run = run_fake_job(
            "huge_output",
            "test_job_huge_output",
            &sample_job_spec(),
            Some(60_000),
        );
        let elapsed = start.elapsed();

        let result = run.result.expect("huge output run should succeed");
        assert_eq!(result.status, "completed");
        assert!(
            elapsed < Duration::from_secs(60),
            "huge output must not deadlock, took {elapsed:?}"
        );

        let job_dir = run.output.path().join("test_job_huge_output");
        let stdout_len = std::fs::metadata(job_dir.join("stdout.log"))
            .expect("stdout.log")
            .len();
        let stderr_len = std::fs::metadata(job_dir.join("stderr.log"))
            .expect("stderr.log")
            .len();
        assert!(
            stdout_len >= 2 * 1024 * 1024,
            "expected >=2 MiB stdout, got {stdout_len}"
        );
        assert!(
            stderr_len >= 2 * 1024 * 1024,
            "expected >=2 MiB stderr, got {stderr_len}"
        );
    }

    #[test]
    fn test_invalid_job_ids_are_rejected_before_launch() {
        let run = run_fake_job("success", "bad/job/id", &sample_job_spec(), Some(30_000));
        match run.result.unwrap_err() {
            BlenderError::InvalidJobId(job_id) => assert_eq!(job_id, "bad/job/id"),
            other => panic!("Expected InvalidJobId, got {other:?}"),
        }

        assert!(validate_job_id("job_001-abc").is_ok());
        assert!(validate_job_id("").is_err());
        assert!(validate_job_id("with space").is_err());
        assert!(validate_job_id("äöü").is_err());
        assert!(validate_job_id(&"a".repeat(128)).is_ok());
        assert!(validate_job_id(&"a".repeat(129)).is_err());
    }

    #[test]
    fn test_path_validation_rejects_absolute() {
        let err = validate_artifact_path(&absolute_path("/absolute/path/file.blend")).unwrap_err();
        match err {
            BlenderError::InvalidArtifact(msg) => {
                assert!(msg.contains("Absolute"));
            }
            other => panic!("Expected InvalidArtifact, got {other:?}"),
        }
    }

    #[test]
    fn test_path_validation_rejects_parent_traversal() {
        let err = validate_artifact_path("../../etc/passwd").unwrap_err();
        match err {
            BlenderError::InvalidArtifact(msg) => {
                assert!(msg.contains("unsafe artifact path"));
            }
            other => panic!("Expected InvalidArtifact, got {other:?}"),
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

        std::fs::write(&job_json_path, serde_json::to_vec(&worker_input).unwrap()).unwrap();

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
            BlenderError::BinaryNotFound(_) => {}
            other => panic!("Expected BinaryNotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_validate_spec_path_absolute_rejected() {
        let err = validate_spec_path(&absolute_path("/etc/passwd")).unwrap_err();
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

    #[test]
    fn test_spec_path_allows_dotdot_inside_file_name() {
        // `..` inside a file-name component is not traversal (P2-3).
        assert!(validate_spec_path("blender/scenes/model.v1..2.blend").is_ok());
        let spec = spec_with_op("save_blend", Some("blender/scenes/model.v1..2.blend"));
        assert!(validate_job_spec_paths(&spec).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_drive_relative_and_unc_paths_rejected() {
        for path in ["C:foo.blend", r"\\server\share\evil.blend"] {
            let artifact_error = validate_artifact_path(path).unwrap_err();
            assert!(
                matches!(artifact_error, BlenderError::InvalidArtifact(_)),
                "artifact path {path:?} must be rejected"
            );
            let spec_error = validate_spec_path(path).unwrap_err();
            assert!(
                format!("{spec_error}").contains("Absolute")
                    || format!("{spec_error}").contains("Root/prefix"),
                "spec path {path:?} must be rejected, got: {spec_error}"
            );
        }
    }

    #[test]
    fn test_validate_job_spec_rejects_absolute_path() {
        let absolute = absolute_path("/absolute/path.blend");
        let spec = spec_with_op("save_blend", Some(&absolute));
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
        let absolute = absolute_path("/etc/out.glb");
        let spec = spec_with_op("export_glb", Some(&absolute));
        let err = validate_job_spec_paths(&spec).unwrap_err();
        assert!(format!("{err}").contains("Absolute"));
    }

    #[test]
    fn test_validate_job_spec_fake_rejects_absolute_before_run() {
        // Verify that run_job_with_fake rejects before launching fake binary
        let dir = TempDir::new().unwrap();
        let fake = create_fake_blender(dir.path(), "success");

        let spec = spec_with_op("save_blend", Some(&absolute_path("/etc/malicious.blend")));
        let workspace = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();

        let result = test_adapter().run_job_with_fake(
            &fake,
            "job_spec_abs",
            &spec,
            workspace.path(),
            output.path(),
            Some(30_000),
        );

        assert!(result.is_err(), "expected spec validation error");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Absolute"),
            "expected Absolute rejection, got: {err_msg}"
        );
    }

    #[test]
    fn test_fake_capabilities_probe() {
        let fake_dir = TempDir::new().unwrap();
        let fake = create_fake_blender(fake_dir.path(), "success");
        let adapter = HeadlessBlenderAdapter::with_config(
            fake.to_string_lossy().to_string(),
            resolve_worker_script(),
        );

        let caps = adapter.probe_capabilities();
        assert!(caps.binary_found);
        assert!(caps.bpy_available, "probe error: {:?}", caps.error);
        assert!(caps.error.is_none());
        assert!(caps
            .supported_operations
            .contains(&"create_box".to_string()));
        assert!(caps
            .blender_version
            .as_deref()
            .unwrap_or_default()
            .contains("Blender"));
        assert_eq!(caps.python_version.as_deref(), Some("3.13.0"));
        assert_eq!(caps.gltf_export_available, Some(true));
        assert!(caps
            .render_engines
            .as_deref()
            .unwrap_or_default()
            .contains(&"BLENDER_WORKBENCH".to_string()));
    }

    #[test]
    fn test_missing_binary_capabilities_is_normal_response() {
        let adapter = HeadlessBlenderAdapter::with_config(
            "nonexistent_blender_xyzzy_it1",
            resolve_worker_script(),
        );
        let caps = adapter.probe_capabilities();
        assert!(!caps.binary_found);
        assert!(!caps.bpy_available);
        assert!(caps.blender_version.is_none());
        assert!(caps.error.is_some());
        assert_eq!(caps.supported_operations.len(), SAFE_WORKER_OPS.len());
    }

    #[test]
    fn test_failed_probe_capabilities_falls_back_to_static_ops() {
        let fake_dir = TempDir::new().unwrap();
        let fake = create_fake_blender(fake_dir.path(), "capfail");
        let adapter = HeadlessBlenderAdapter::with_config(
            fake.to_string_lossy().to_string(),
            resolve_worker_script(),
        );

        let caps = adapter.probe_capabilities();
        assert!(caps.binary_found);
        assert!(!caps.bpy_available);
        assert!(caps.error.is_some());
        assert_eq!(caps.supported_operations.len(), SAFE_WORKER_OPS.len());
        assert!(caps
            .supported_operations
            .contains(&"create_box".to_string()));
    }

    /// Real Blender smoke test — runs only if `blender` CLI is available.
    /// Verifies worker.py can be executed and produces a valid result.json.
    #[test]
    fn test_real_blender_worker_smoke() {
        let adapter = HeadlessBlenderAdapter::with_config("blender", resolve_worker_script());
        if adapter.check_binary().is_err() {
            eprintln!("[skip] real_blender_worker_smoke: blender not available");
            return;
        }

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

        let result = adapter
            .run_job(
                "real_blender_smoke_001",
                &spec,
                workspace.path(),
                output.path(),
                Some(60_000),
            )
            .expect("Real Blender smoke should succeed");

        assert_eq!(
            result.status, "completed",
            "Blender job failed: {:?}",
            result.error
        );
        let artifacts = result.artifacts.clone().unwrap_or_default();
        assert!(!artifacts.is_empty(), "Expected at least one artifact");
        for artifact in &artifacts {
            let path = workspace.path().join(&artifact.path);
            assert!(
                path.is_file(),
                "artifact {} should exist at {path:?}",
                artifact.path
            );
            assert!(std::fs::metadata(&path).unwrap().len() > 0);
        }

        let job_dir = output.path().join("real_blender_smoke_001");
        assert!(job_dir.join("result.json").exists());
        assert!(job_dir.join("stdout.log").exists());
        assert!(job_dir.join("stderr.log").exists());

        let content =
            std::fs::read_to_string(job_dir.join("result.json")).expect("Should read result.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("result.json should be valid JSON");
        assert_eq!(parsed["status"], "completed");
        eprintln!("[info] Real Blender smoke artifacts: {artifacts:?}");
    }

    /// Real capabilities probe — runs only if `blender` CLI is available.
    #[test]
    fn test_real_blender_capabilities_probe() {
        let adapter = HeadlessBlenderAdapter::with_config("blender", resolve_worker_script());
        if adapter.check_binary().is_err() {
            eprintln!("[skip] real_blender_capabilities_probe: blender not available");
            return;
        }

        let caps = adapter.probe_capabilities();
        assert!(caps.binary_found);
        assert!(
            caps.bpy_available,
            "real capabilities probe failed: {:?}",
            caps.error
        );
        assert!(caps.error.is_none());
        assert!(caps
            .supported_operations
            .contains(&"create_box".to_string()));
        let version = caps.blender_version.as_deref().unwrap_or_default();
        assert!(
            version.starts_with("Blender "),
            "unexpected blender version: {version}"
        );
        assert!(caps.python_version.is_some());
        assert!(caps
            .render_engines
            .as_deref()
            .unwrap_or_default()
            .contains(&"BLENDER_WORKBENCH".to_string()));
    }
}
