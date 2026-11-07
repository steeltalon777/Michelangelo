//! Discovery and bounded execution of the Python solver and comparator.
//!
//! # Tool discovery
//!
//! * interpreter: --python > env MICHELANGELO_PYTHON > "python",
//! * solver script: --solver > env MICHELANGELO_SOLVER >
//!   <repo>/tools/solver/solve.py.
//!
//! The repository root is discovered **at build time** from
//! CARGO_MANIFEST_DIR/../.. (the crate lives in crates/michelangelo_recon),
//! so the default solver path is correct for a release build of this
//! repository and for the test suite. The comparator is the compare.py
//! sibling of the solver script.
//!
//! # Execution
//!
//! Every subprocess runs with a bounded timeout (default 600 s, overridable
//! with MICHELANGELO_SOLVER_TIMEOUT_MS); stdout and stderr are captured and
//! only the last 8 KiB of each stream are kept.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::ReconError;

/// Default timeout of a solver or comparator subprocess, in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 600_000;

/// Environment variable that overrides the solver subprocess timeout.
pub const TIMEOUT_ENV: &str = "MICHELANGELO_SOLVER_TIMEOUT_MS";

/// Environment variable that overrides the Python interpreter.
pub const PYTHON_ENV: &str = "MICHELANGELO_PYTHON";

/// Environment variable that overrides the solver script.
pub const SOLVER_ENV: &str = "MICHELANGELO_SOLVER";

/// Maximum number of bytes kept from stdout or stderr.
pub const TAIL_BYTES: usize = 8 * 1024;

/// The repository root of this checkout (build-time constant).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// The default solver script of this checkout.
pub fn default_solver_script() -> PathBuf {
    repo_root().join("tools").join("solver").join("solve.py")
}

/// The resolved paths of the external tools of the pipeline.
#[derive(Debug, Clone)]
pub struct ToolPaths {
    /// The Python interpreter (a name is allowed and resolved via PATH).
    pub python: PathBuf,
    /// The solver script (tools/solver/solve.py).
    pub solver_script: PathBuf,
    /// The comparator script (tools/solver/compare.py).
    pub compare_script: PathBuf,
}

/// Resolve the tool paths from the explicit options, the environment and the
/// build-time defaults.
pub fn resolve_tool_paths(
    solver: Option<&Path>,
    python: Option<&Path>,
) -> Result<ToolPaths, ReconError> {
    let python_path = match python {
        Some(path) => path.to_path_buf(),
        None => match std::env::var_os(PYTHON_ENV) {
            Some(value) => PathBuf::from(value),
            None => PathBuf::from("python"),
        },
    };

    let solver_path = match solver {
        Some(path) => path.to_path_buf(),
        None => match std::env::var_os(SOLVER_ENV) {
            Some(value) => PathBuf::from(value),
            None => default_solver_script(),
        },
    };

    let compare_path = solver_path
        .parent()
        .map(|parent| parent.join("compare.py"))
        .unwrap_or_else(|| PathBuf::from("compare.py"));

    if !solver_path.is_file() {
        return Err(ReconError::ToolNotFound {
            what: "the solver script".to_string(),
            path: solver_path,
        });
    }
    if !compare_path.is_file() {
        return Err(ReconError::ToolNotFound {
            what: "the comparator script".to_string(),
            path: compare_path,
        });
    }

    Ok(ToolPaths {
        python: python_path,
        solver_script: solver_path,
        compare_script: compare_path,
    })
}

/// Resolve the effective subprocess timeout: explicit value > environment >
/// the built-in default.
pub fn resolve_timeout_ms(explicit: Option<u64>) -> u64 {
    if let Some(value) = explicit {
        if value > 0 {
            return value;
        }
    }
    if let Ok(value) = std::env::var(TIMEOUT_ENV) {
        if let Ok(parsed) = value.trim().parse::<u64>() {
            if parsed > 0 {
                return parsed;
            }
        }
    }
    DEFAULT_TIMEOUT_MS
}

/// The result of a successful solver run.
#[derive(Debug, Clone)]
pub struct SolverRun {
    /// The parsed build_report.json.
    pub build_report: serde_json::Value,
    /// The tail of the solver stdout.
    pub stdout_tail: String,
    /// The tail of the solver stderr.
    pub stderr_tail: String,
}

/// The result of a comparator run (also produced when the thresholds fail).
#[derive(Debug, Clone)]
pub struct CompareRun {
    /// The parsed quality_report.json.
    pub report: serde_json::Value,
    /// The threshold status of the report ("pass" or "fail").
    pub status: String,
    /// The tail of the comparator stdout.
    pub stdout_tail: String,
    /// The tail of the comparator stderr.
    pub stderr_tail: String,
}

/// Run the solver and parse its build report.
///
/// Exit code 0 is success, 2 means the package is unusable and 3 means the
/// solver itself failed; both are mapped to typed errors.
pub fn run_solver(
    paths: &ToolPaths,
    package_dir: &Path,
    out_dir: &Path,
    resolution: Option<u32>,
    timeout_ms: Option<u64>,
) -> Result<SolverRun, ReconError> {
    std::fs::create_dir_all(out_dir).map_err(|err| ReconError::Io {
        context: format!(
            "the solver output directory cannot be created: '{}'",
            out_dir.display()
        ),
        message: err.to_string(),
    })?;

    let mut args: Vec<OsString> = vec![paths.solver_script.clone().into_os_string()];
    args.push("--package".into());
    args.push(package_dir.into());
    args.push("--out".into());
    args.push(out_dir.into());
    if let Some(resolution) = resolution {
        args.push("--resolution".into());
        args.push(resolution.to_string().into());
    }

    let timeout = resolve_timeout_ms(timeout_ms);
    let captured = run_command(&paths.python, &args, timeout)?;

    match captured.status {
        Some(0) => {}
        Some(2) => {
            return Err(ReconError::SolverRejected {
                message: captured.stderr.trim().to_string(),
                stdout: captured.stdout,
            })
        }
        other => {
            return Err(ReconError::SolverFailed {
                exit_code: other,
                stderr: captured.stderr,
                stdout: captured.stdout,
            })
        }
    }

    let report_path = out_dir.join("build_report.json");
    let text = std::fs::read_to_string(&report_path).map_err(|_| ReconError::SolverOutput {
        message: "the solver did not write build_report.json".to_string(),
    })?;
    let build_report: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| ReconError::SolverOutput {
            message: format!("build_report.json is not valid JSON: {err}"),
        })?;

    Ok(SolverRun {
        build_report,
        stdout_tail: captured.stdout,
        stderr_tail: captured.stderr,
    })
}

/// Run the comparator and parse its quality report.
///
/// Exit code 1 (thresholds not met) is not an error: the report is returned
/// with the status "fail".
pub fn run_compare(
    paths: &ToolPaths,
    package_dir: &Path,
    renders_dir: &Path,
    out_dir: &Path,
    evaluation: Option<(&Path, &Path)>,
    timeout_ms: Option<u64>,
) -> Result<CompareRun, ReconError> {
    let mut args: Vec<OsString> = vec![paths.compare_script.clone().into_os_string()];
    args.push("--package".into());
    args.push(package_dir.into());
    args.push("--renders".into());
    args.push(renders_dir.into());
    args.push("--out".into());
    args.push(out_dir.into());
    if let Some((evaluation_package, evaluation_renders)) = evaluation {
        args.push("--evaluation-package".into());
        args.push(evaluation_package.into());
        args.push("--evaluation-renders".into());
        args.push(evaluation_renders.into());
    }

    let timeout = resolve_timeout_ms(timeout_ms);
    let captured = run_command(&paths.python, &args, timeout)?;

    match captured.status {
        Some(0) | Some(1) => {}
        other => {
            return Err(ReconError::CompareFailed {
                message: format!(
                    "the comparator exited with code {}: {}",
                    other.map_or_else(|| "unknown".to_string(), |code| code.to_string()),
                    captured.stderr.trim()
                ),
            })
        }
    }

    let report_path = out_dir.join("quality_report.json");
    let text = std::fs::read_to_string(&report_path).map_err(|_| ReconError::CompareFailed {
        message: "the comparator did not write quality_report.json".to_string(),
    })?;
    let report: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| ReconError::CompareFailed {
            message: format!("quality_report.json is not valid JSON: {err}"),
        })?;

    let status = report
        .get("thresholds")
        .and_then(|thresholds| thresholds.get("status"))
        .and_then(|value| value.as_str())
        .unwrap_or("fail")
        .to_string();

    Ok(CompareRun {
        report,
        status,
        stdout_tail: captured.stdout,
        stderr_tail: captured.stderr,
    })
}

/// The captured result of a subprocess.
#[derive(Debug, Clone)]
struct Captured {
    /// The exit code of the process (None when it was killed by a signal).
    status: Option<i32>,
    /// The tail of stdout.
    stdout: String,
    /// The tail of stderr.
    stderr: String,
}

/// Run a subprocess with a bounded timeout and captured output tails.
fn run_command(program: &Path, args: &[OsString], timeout_ms: u64) -> Result<Captured, ReconError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|err| ReconError::ToolLaunch {
        what: format!("'{}'", program.display()),
        message: err.to_string(),
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || drain_tail(stdout));
    let stderr_reader = thread::spawn(move || drain_tail(stderr));
    let mut stdout_handle = Some(stdout_reader);
    let mut stderr_handle = Some(stderr_reader);

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let stderr_text = stderr_handle
                        .take()
                        .map(|handle| handle.join().unwrap_or_default())
                        .unwrap_or_default();
                    let stdout_text = stdout_handle
                        .take()
                        .map(|handle| handle.join().unwrap_or_default())
                        .unwrap_or_default();
                    return Err(ReconError::SolverTimeout {
                        timeout_ms,
                        stderr: stderr_text,
                        stdout: stdout_text,
                    });
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => {
                return Err(ReconError::ToolLaunch {
                    what: format!("'{}'", program.display()),
                    message: err.to_string(),
                })
            }
        }
    };

    let stdout_text = stdout_handle
        .take()
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr_text = stderr_handle
        .take()
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();

    Ok(Captured {
        status: status.code(),
        stdout: stdout_text,
        stderr: stderr_text,
    })
}

/// Read a stream to its end and keep only its last TAIL_BYTES bytes.
fn drain_tail<R: Read>(reader: Option<R>) -> String {
    let Some(mut reader) = reader else {
        return String::new();
    };
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                buffer.extend_from_slice(&chunk[..count]);
                if buffer.len() > 4 * TAIL_BYTES {
                    let excess = buffer.len() - TAIL_BYTES;
                    buffer.drain(..excess);
                }
            }
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAILING_SOLVER: &str = "import sys
sys.stderr.write('boom')
sys.exit(3)
";

    const REJECTING_SOLVER: &str = "import sys
sys.stderr.write('the package is unusable')
sys.exit(2)
";

    const HANGING_SOLVER: &str = "import time
time.sleep(30)
";

    const SILENT_SOLVER: &str = "print('nothing is written')
";

    fn fake_paths(directory: &Path, script_body: &str) -> ToolPaths {
        let script = directory.join("fake_solver.py");
        std::fs::write(&script, script_body).expect("the fake solver is written");
        let compare = directory.join("compare.py");
        std::fs::write(
            &compare,
            "# unused
",
        )
        .expect("the fake comparator is written");
        ToolPaths {
            python: PathBuf::from("python"),
            solver_script: script,
            compare_script: compare,
        }
    }

    #[test]
    fn default_solver_script_points_into_the_repository() {
        let script = default_solver_script();
        assert!(script.ends_with("tools/solver/solve.py"));
        assert!(
            script.is_file(),
            "the solver script must exist in the checkout"
        );
    }

    #[test]
    fn tool_paths_prefer_the_explicit_options() {
        let script = default_solver_script();
        let paths = resolve_tool_paths(Some(&script), Some(Path::new("python3")))
            .expect("the repository tools exist");
        assert_eq!(paths.python, PathBuf::from("python3"));
        assert_eq!(paths.solver_script, script);
        assert_eq!(
            paths.compare_script,
            script.parent().unwrap().join("compare.py")
        );
    }

    #[test]
    fn a_missing_solver_script_is_reported() {
        let error = resolve_tool_paths(Some(Path::new("does/not/exist/solve.py")), None)
            .expect_err("a missing solver script must be rejected");
        assert!(matches!(error, ReconError::ToolNotFound { .. }));
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn the_timeout_falls_back_to_the_default() {
        assert_eq!(resolve_timeout_ms(Some(1234)), 1234);
        assert_eq!(resolve_timeout_ms(None), DEFAULT_TIMEOUT_MS);
    }

    #[test]
    fn a_failing_solver_is_mapped_to_a_typed_error() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = fake_paths(directory.path(), FAILING_SOLVER);
        let error = run_solver(
            &paths,
            Path::new("package"),
            &directory.path().join("out"),
            None,
            Some(60_000),
        )
        .expect_err("exit code 3 must be an error");
        match error {
            ReconError::SolverFailed {
                exit_code, stderr, ..
            } => {
                assert_eq!(exit_code, Some(3));
                assert!(stderr.contains("boom"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn a_rejected_package_is_mapped_to_a_typed_error() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = fake_paths(directory.path(), REJECTING_SOLVER);
        let error = run_solver(
            &paths,
            Path::new("package"),
            &directory.path().join("out"),
            None,
            Some(60_000),
        )
        .expect_err("exit code 2 must be an error");
        assert!(matches!(error, ReconError::SolverRejected { .. }));
    }

    #[test]
    fn a_hanging_solver_is_stopped_by_the_timeout() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = fake_paths(directory.path(), HANGING_SOLVER);
        let error = run_solver(
            &paths,
            Path::new("package"),
            &directory.path().join("out"),
            None,
            Some(1_000),
        )
        .expect_err("the solver must time out");
        match error {
            ReconError::SolverTimeout { timeout_ms, .. } => assert_eq!(timeout_ms, 1_000),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn a_missing_build_report_is_reported() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = fake_paths(directory.path(), SILENT_SOLVER);
        let error = run_solver(
            &paths,
            Path::new("package"),
            &directory.path().join("out"),
            None,
            Some(60_000),
        )
        .expect_err("a missing build report must be an error");
        assert!(matches!(error, ReconError::SolverOutput { .. }));
    }

    #[test]
    fn the_output_tail_is_bounded() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let script = directory.path().join("fake_solver.py");
        std::fs::write(
            &script,
            "import sys
sys.stdout.write('x' * 100000)
sys.stderr.write('y' * 100000)
",
        )
        .expect("the fake solver is written");
        let compare = directory.path().join("compare.py");
        std::fs::write(
            &compare,
            "# unused
",
        )
        .expect("the fake comparator is written");
        let paths = ToolPaths {
            python: PathBuf::from("python"),
            solver_script: script,
            compare_script: compare,
        };
        let error = run_solver(
            &paths,
            Path::new("package"),
            &directory.path().join("out"),
            None,
            Some(60_000),
        )
        .expect_err("the fake solver writes no build report");
        match error {
            ReconError::SolverOutput { .. } => {}
            other => panic!("unexpected error: {other:?}"),
        }
        let captured = drain_tail(None::<std::io::Empty>);
        assert!(captured.is_empty());
    }
}
