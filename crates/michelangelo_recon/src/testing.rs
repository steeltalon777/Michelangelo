//! Test support: a deterministic fake Blender executable.
//!
//! The fake mimics the worker result contract of the Blender adapter: it
//! reads the job file, produces every artifact the job asks for and writes a
//! `result.json` with the artifact list. It never launches Blender.
//!
//! The silhouette renders are copies of the package masks (so the quality
//! comparison is exact) or, when no mask is available, a tiny generated PNG.
//!
//! The helper is public so that the integration tests of the CLI crate can
//! reuse it; it is never used by the production code paths.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::ReconError;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// The Python source of the fake Blender.
const FAKE_PYTHON: &str = r#"import json
import os
import shutil
import struct
import sys
import zlib


def png_bytes(width, height):
    raw = b""
    for _ in range(height):
        raw += b"\x00" + b"\x00\x00\x00\x00" * width

    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff))

    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def main():
    argv = sys.argv[1:]
    if len(argv) < 2:
        sys.stderr.write("fake_blender: the job and result paths are missing\n")
        return 2
    job_path = argv[-2]
    result_path = argv[-1]
    with open(job_path, "r", encoding="utf-8") as handle:
        job = json.load(handle)
    root = job["workspace_root"]
    operations = job["spec"]["operations"]
    masks = os.environ.get("FAKE_MASKS", "")
    artifacts = []
    for operation in operations:
        kind = operation.get("op")
        if kind == "render_camera":
            relative = operation["path"]
            target = os.path.join(root, relative.replace("/", os.sep))
            directory = os.path.dirname(target)
            if directory:
                os.makedirs(directory, exist_ok=True)
            name = os.path.basename(relative)
            source = ""
            if masks:
                for candidate in (os.path.join(masks, name),
                                  os.path.join(masks, "masks", name)):
                    if os.path.isfile(candidate):
                        source = candidate
                        break
            if source:
                shutil.copyfile(source, target)
            else:
                with open(target, "wb") as handle:
                    handle.write(png_bytes(8, 8))
            artifacts.append({"path": relative, "type": "png",
                              "size_bytes": os.path.getsize(target)})
        elif kind in ("save_blend", "export_glb"):
            relative = operation["path"]
            target = os.path.join(root, relative.replace("/", os.sep))
            directory = os.path.dirname(target)
            if directory:
                os.makedirs(directory, exist_ok=True)
            with open(target, "wb") as handle:
                handle.write(b"FAKE_BLENDER_ARTIFACT")
            artifacts.append({"path": relative,
                              "type": "blend" if kind == "save_blend" else "glb",
                              "size_bytes": os.path.getsize(target)})
    result = {"job_id": job["job_id"], "status": "completed",
              "message": "fake blender", "artifacts": artifacts}
    with open(result_path, "w", encoding="utf-8") as handle:
        json.dump(result, handle)
    return 0


sys.exit(main())
"#;

/// The names of the environment variables that carry the fake configuration.
#[derive(Debug, Clone)]
pub struct FakeBlenderEnv {
    /// The name of the variable with the Python interpreter.
    pub python_var: String,
    /// The name of the variable with the mask directory.
    pub masks_var: String,
}

/// Write a fake Blender executable into a directory.
///
/// The returned path is the executable that can be handed to
/// `HeadlessBlenderAdapter::run_job_with_fake`. The configuration is
/// passed to the fake through environment variables with unique names, so
/// parallel tests never interfere with each other.
pub fn write_fake_blender(
    directory: &Path,
    python: &Path,
    masks_dir: Option<&Path>,
) -> Result<(PathBuf, FakeBlenderEnv), ReconError> {
    std::fs::create_dir_all(directory).map_err(|err| ReconError::Io {
        context: "the fake Blender directory cannot be created".to_string(),
        message: err.to_string(),
    })?;

    let index = COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    let env = FakeBlenderEnv {
        python_var: format!("MICHELANGELO_FAKE_PYTHON_{index}"),
        masks_var: format!("MICHELANGELO_FAKE_MASKS_{index}"),
    };

    let script = directory.join("fake_blender.py");
    std::fs::write(&script, FAKE_PYTHON).map_err(|err| ReconError::Io {
        context: "the fake Blender script cannot be written".to_string(),
        message: err.to_string(),
    })?;

    let python_value = python.display().to_string();
    std::env::set_var(&env.python_var, &python_value);
    let masks_value = masks_dir
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    std::env::set_var(&env.masks_var, &masks_value);

    #[cfg(windows)]
    let executable = {
        let wrapper = directory.join("fake_blender.cmd");
        let content = format!(
            "@echo off\r\nsetlocal\r\nset \"FAKE_PYTHON=%{python_var}%\"\r\nset \"FAKE_MASKS=%{masks_var}%\"\r\n\"%FAKE_PYTHON%\" \"%~dp0fake_blender.py\" %*\r\nexit /b %ERRORLEVEL%\r\n",
            python_var = env.python_var,
            masks_var = env.masks_var
        );
        std::fs::write(&wrapper, content).map_err(|err| ReconError::Io {
            context: "the fake Blender wrapper cannot be written".to_string(),
            message: err.to_string(),
        })?;
        wrapper
    };

    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        let wrapper = directory.join("fake_blender.sh");
        let content = "#!/bin/sh\nset -e\nDIR=$(dirname \"$0\")\nexec \"$FAKE_PYTHON\" \"$DIR/fake_blender.py\" \"$@\"\n";
        std::fs::write(&wrapper, content).map_err(|err| ReconError::Io {
            context: "the fake Blender wrapper cannot be written".to_string(),
            message: err.to_string(),
        })?;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).map_err(
            |err| ReconError::Io {
                context: "the fake Blender wrapper cannot be made executable".to_string(),
                message: err.to_string(),
            },
        )?;
        wrapper
    };

    Ok((executable, env))
}

/// Remove the environment variables of a fake Blender.
pub fn clear_fake_blender_env(env: &FakeBlenderEnv) {
    std::env::remove_var(&env.python_var);
    std::env::remove_var(&env.masks_var);
}
