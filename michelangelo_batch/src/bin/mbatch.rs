//! `mbatch` — provisional PoC CLI for the durable batch executor.
//!
//! This is NOT the future `michelangelo batch ...` CLI routing: it neither
//! touches the existing CLI dispatcher nor reuses its transport. Subcommands:
//!
//!   run      --plan <file> --db <sqlite> --runs <dir> [--slot-wait-ms N]
//!            [--inject-crash-after-reserve] [--inject-crash-before-insert]
//!   resume   --plan-id <id>  --db <sqlite> --runs <dir>
//!   retry    --plan-id <id>  --db <sqlite> --runs <dir> --items a,b
//!   status   --plan-id <id>  --db <sqlite>
//!   cancel   --plan-id <id>  --db <sqlite> [--item <id>]
//!   worker   --spec <file> --attempt-dir <dir>     (internal)
//!
//! Machine output: a JSON batch report on stdout. Human diagnostics go to
//! stderr. Exit codes: see `michelangelo_batch::exit_codes`.

use std::path::PathBuf;
use std::process::ExitCode;

use michelangelo_batch::executor::{
    cancel_batch, execute, load_or_register, load_registered, read_status, ExecError, ExecOptions,
    RunMode,
};
use michelangelo_batch::exit_codes;
use michelangelo_batch::worker::run_worker;

fn fail(err: &ExecError) -> ExitCode {
    eprintln!("mbatch: {err}");
    match err {
        ExecError::SlotBusy { .. } => ExitCode::from(exit_codes::SLOT_BUSY as u8),
        ExecError::BatchCancelled => ExitCode::from(exit_codes::USAGE as u8),
        ExecError::Plan(_) | ExecError::Conflict(_) => ExitCode::from(exit_codes::USAGE as u8),
        ExecError::Store(michelangelo_batch::StoreError::PlanImmutable { .. }) => {
            ExitCode::from(exit_codes::USAGE as u8)
        }
        _ => ExitCode::from(exit_codes::INTERNAL as u8),
    }
}

struct Args {
    flags: std::collections::HashMap<String, String>,
    switches: std::collections::HashSet<String>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut flags = std::collections::HashMap::new();
    let mut switches = std::collections::HashSet::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            if let Some(eq_pos) = name.find('=') {
                flags.insert(name[..eq_pos].to_string(), name[eq_pos + 1..].to_string());
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags.insert(name.to_string(), args[i + 1].clone());
                i += 1;
            } else {
                switches.insert(name.to_string());
            }
        }
        // Positional arguments are not used by this provisional CLI; they are
        // ignored on purpose so unknown tokens never become silent options.
        i += 1;
    }
    Ok(Args { flags, switches })
}

impl Args {
    fn required(&self, name: &str) -> Result<String, String> {
        self.flags
            .get(name)
            .cloned()
            .ok_or_else(|| format!("missing required --{name}"))
    }
    fn optional(&self, name: &str) -> Option<String> {
        self.flags.get(name).cloned()
    }
    fn opt_u64(&self, name: &str) -> Result<Option<u64>, String> {
        match self.flags.get(name) {
            Some(v) => v
                .parse::<u64>()
                .map(Some)
                .map_err(|_| format!("--{name} must be a number, got {v:?}")),
            None => Ok(None),
        }
    }
    fn is_set(&self, name: &str) -> bool {
        self.switches.contains(name) || self.flags.contains_key(name)
    }
}

fn print_report(report: &michelangelo_batch::BatchReport) -> bool {
    match report.to_json() {
        Ok(json) => {
            println!("{json}");
            true
        }
        Err(e) => {
            eprintln!("mbatch: report serialization failed: {e}");
            false
        }
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.is_empty() {
        eprintln!("usage: mbatch <run|resume|retry|status|cancel|worker> [options]");
        return ExitCode::from(exit_codes::USAGE as u8);
    }
    let cmd = raw[0].clone();
    let args = match parse_args(&raw[1..]) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("mbatch: {e}");
            return ExitCode::from(exit_codes::USAGE as u8);
        }
    };

    match cmd.as_str() {
        "run" => cmd_run(&args),
        "resume" => cmd_resume(&args),
        "retry" => cmd_retry(&args),
        "status" => cmd_status(&args),
        "cancel" => cmd_cancel(&args),
        "worker" => cmd_worker(&args),
        other => {
            eprintln!("mbatch: unknown command {other:?}");
            ExitCode::from(exit_codes::USAGE as u8)
        }
    }
}

fn cmd_run(args: &Args) -> ExitCode {
    let plan_file = match args.required("plan") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    let db = match args.required("db") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    let runs = match args.required("runs") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    let slot_wait_ms = args
        .opt_u64("slot-wait-ms")
        .ok()
        .flatten()
        .unwrap_or(10_000);
    let opts = ExecOptions {
        slot_wait_ms,
        inject_crash_after_reserve: args.is_set("inject-crash-after-reserve"),
        inject_crash_before_insert: args.is_set("inject-crash-before-insert"),
    };

    let store = match michelangelo_batch::Store::open(&db) {
        Ok(s) => s,
        Err(e) => return fail(&ExecError::Store(e)),
    };
    let (plan, plan_id, fingerprint) = match load_or_register(&plan_file, &store) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    drop(store);

    match execute(
        &plan,
        &plan_id,
        &fingerprint,
        &db,
        &runs,
        RunMode::FreshRun,
        &opts,
    ) {
        Ok(report) => finish_with_report(&report),
        Err(e) => fail(&e),
    }
}

fn cmd_resume(args: &Args) -> ExitCode {
    let plan_id = match args.required("plan-id") {
        Ok(v) => v,
        Err(e) => return usage_err(&e),
    };
    cmd_with_stored_plan(args, &plan_id, RunMode::Resume)
}

fn cmd_retry(args: &Args) -> ExitCode {
    let plan_id = match args.required("plan-id") {
        Ok(v) => v,
        Err(e) => return usage_err(&e),
    };
    let items = match args.required("items") {
        Ok(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>(),
        Err(e) => return usage_err(&e),
    };
    if items.is_empty() {
        return usage_err("--items must list at least one item id");
    }
    cmd_with_stored_plan(args, &plan_id, RunMode::Retry(items))
}

fn cmd_with_stored_plan(args: &Args, plan_id: &str, mode: RunMode) -> ExitCode {
    let db = match args.required("db") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    let runs = match args.required("runs") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    let slot_wait_ms = args
        .opt_u64("slot-wait-ms")
        .ok()
        .flatten()
        .unwrap_or(10_000);
    let opts = ExecOptions {
        slot_wait_ms,
        inject_crash_after_reserve: false,
        inject_crash_before_insert: false,
    };
    let store = match michelangelo_batch::Store::open(&db) {
        Ok(s) => s,
        Err(e) => return fail(&ExecError::Store(e)),
    };
    let (plan, plan_id, fingerprint) = match load_registered(plan_id, &store) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    drop(store);
    match execute(&plan, &plan_id, &fingerprint, &db, &runs, mode, &opts) {
        Ok(report) => finish_with_report(&report),
        Err(e) => fail(&e),
    }
}

fn cmd_status(args: &Args) -> ExitCode {
    let plan_id = match args.required("plan-id") {
        Ok(v) => v,
        Err(e) => return usage_err(&e),
    };
    let db = match args.required("db") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    match read_status(&plan_id, &db) {
        Ok(report) => {
            if print_report(&report) {
                ExitCode::from(exit_codes::OK as u8)
            } else {
                ExitCode::from(exit_codes::INTERNAL as u8)
            }
        }
        Err(e) => fail(&e),
    }
}

fn cmd_cancel(args: &Args) -> ExitCode {
    let plan_id = match args.required("plan-id") {
        Ok(v) => v,
        Err(e) => return usage_err(&e),
    };
    let db = match args.required("db") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    match cancel_batch(&plan_id, &db, args.optional("item").as_deref()) {
        Ok(report) => {
            if print_report(&report) {
                ExitCode::from(exit_codes::OK as u8)
            } else {
                ExitCode::from(exit_codes::INTERNAL as u8)
            }
        }
        Err(e) => fail(&e),
    }
}

fn cmd_worker(args: &Args) -> ExitCode {
    let spec = match args.required("spec") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    let attempt_dir = match args.required("attempt-dir") {
        Ok(v) => PathBuf::from(v),
        Err(e) => return usage_err(&e),
    };
    ExitCode::from(run_worker(&spec, &attempt_dir) as u8)
}

fn finish_with_report(report: &michelangelo_batch::BatchReport) -> ExitCode {
    if !print_report(report) {
        return ExitCode::from(exit_codes::INTERNAL as u8);
    }
    if report.all_success() {
        ExitCode::from(exit_codes::OK as u8)
    } else {
        ExitCode::from(exit_codes::PARTIAL_FAILURE as u8)
    }
}

fn usage_err(msg: &str) -> ExitCode {
    eprintln!("mbatch: {msg}");
    ExitCode::from(exit_codes::USAGE as u8)
}
