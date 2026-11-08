//! The MCP adapter: a newline-delimited JSON-RPC 2.0 server over stdio.
//!
//! The server is the transport of the assistant surface: it exposes the four
//! allow-listed tools to a local LLM client and nothing else. It performs no
//! network access, it embeds no model and it never applies a candidate.
//!
//! The transport is one JSON object per line on stdin and stdout; the
//! diagnostics go to stderr. A malformed line never kills the server and the
//! end of the input terminates the server with the exit code 0.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::budget::{
    AuditEntry, BudgetStore, DECISION_ERROR, DECISION_OK, DECISION_PROPOSED, DECISION_REJECTED,
};
use crate::error::AssistantError;
use crate::policy::{
    AssistantPolicy, TOOL_GET_REPORT, TOOL_GET_TASK, TOOL_SUBMIT_CANDIDATE, TOOL_VALIDATE_PACKAGE,
};
use crate::task::{load_task, task_path};
use crate::tools::{self, ToolContext, ToolOutcome};
use crate::{sanitize_paths, Clock};

/// The MCP protocol version of the server.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// The name of the server.
pub const SERVER_NAME: &str = "michelangelo";
/// The version of the server.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The JSON-RPC code of a malformed line.
pub const ERROR_PARSE: i64 = -32700;
/// The JSON-RPC code of an invalid request.
pub const ERROR_INVALID_REQUEST: i64 = -32600;
/// The JSON-RPC code of an unknown method.
pub const ERROR_METHOD_NOT_FOUND: i64 = -32601;
/// The JSON-RPC code of invalid tool arguments.
pub const ERROR_INVALID_PARAMS: i64 = -32602;
/// The JSON-RPC code of an unexpected internal failure.
pub const ERROR_INTERNAL: i64 = -32603;
/// The JSON-RPC code of a policy denial.
pub const ERROR_POLICY_DENIED: i64 = -32001;
/// The JSON-RPC code of a path outside the workspace.
pub const ERROR_PATH_NOT_ALLOWED: i64 = -32002;
/// The JSON-RPC code of an exhausted budget.
pub const ERROR_BUDGET_EXHAUSTED: i64 = -32003;
/// The JSON-RPC code of an invalid task or candidate.
pub const ERROR_TASK_INVALID: i64 = -32004;
/// The JSON-RPC code of a missing object.
pub const ERROR_NOT_FOUND: i64 = -32005;

/// A JSON-RPC error of the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// The JSON-RPC code of the failure.
    pub code: i64,
    /// The human readable reason.
    pub message: String,
}

impl RpcError {
    /// Build an error of the server.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError {
            code,
            message: message.into(),
        }
    }
}

/// The MCP server of a workspace.
pub struct McpServer {
    workspace: PathBuf,
    policy: AssistantPolicy,
    clock: Box<dyn Clock>,
}

impl McpServer {
    /// Build the server of a workspace.
    pub fn new(
        workspace: PathBuf,
        policy: AssistantPolicy,
        clock: Box<dyn Clock>,
    ) -> Result<Self, AssistantError> {
        let canonical = std::fs::canonicalize(&workspace).map_err(|error| {
            AssistantError::io(
                &workspace,
                format!("the workspace cannot be resolved: {error}"),
            )
        })?;
        Ok(McpServer {
            workspace: canonical,
            policy,
            clock,
        })
    }

    /// The root of the workspace of the server.
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// The policy of the server.
    pub fn policy(&self) -> &AssistantPolicy {
        &self.policy
    }

    /// Serve the requests of a client until the end of the input.
    pub fn serve(
        &mut self,
        input: &mut dyn BufRead,
        output: &mut dyn Write,
    ) -> Result<(), AssistantError> {
        let mut buffer: Vec<u8> = Vec::new();
        loop {
            buffer.clear();
            let read = input.read_until(b'\n', &mut buffer).map_err(|error| {
                AssistantError::io(
                    Path::new("<stdin>"),
                    format!("the input cannot be read: {error}"),
                )
            })?;
            if read == 0 {
                break;
            }
            let line = String::from_utf8_lossy(&buffer).to_string();
            if let Some(response) = self.handle_line(&line) {
                writeln!(output, "{response}").map_err(|error| {
                    AssistantError::io(
                        Path::new("<stdout>"),
                        format!("the output cannot be written: {error}"),
                    )
                })?;
                output.flush().map_err(|error| {
                    AssistantError::io(
                        Path::new("<stdout>"),
                        format!("the output cannot be flushed: {error}"),
                    )
                })?;
            }
        }
        Ok(())
    }

    /// Handle one line of the transport.
    ///
    /// None is returned for a line that carries no response (an accepted
    /// notification and an empty line).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => {
                return Some(error_response(
                    Value::Null,
                    ERROR_PARSE,
                    "the request is not valid JSON",
                ))
            }
        };
        let object = match value.as_object() {
            Some(object) => object,
            None => {
                return Some(error_response(
                    Value::Null,
                    ERROR_INVALID_REQUEST,
                    "the request is not a JSON object",
                ))
            }
        };
        let id = object.get("id").cloned().unwrap_or(Value::Null);
        if object.get("jsonrpc").and_then(|value| value.as_str()) != Some("2.0") {
            return Some(error_response(
                id,
                ERROR_INVALID_REQUEST,
                "the request does not declare the protocol version 2.0",
            ));
        }
        let method = match object.get("method").and_then(|value| value.as_str()) {
            Some(method) => method.to_string(),
            None => {
                return Some(error_response(
                    id,
                    ERROR_INVALID_REQUEST,
                    "the request carries no method",
                ))
            }
        };

        match method.as_str() {
            "notifications/initialized" => None,
            "initialize" => Some(result_response(id, self.initialize_result())),
            "ping" => Some(result_response(id, json!({}))),
            "tools/list" => Some(result_response(id, json!({"tools": self.tool_list()}))),
            "tools/call" => Some(self.handle_call(id, object.get("params"))),
            other => Some(error_response(
                id,
                ERROR_METHOD_NOT_FOUND,
                &format!("the method '{other}' is not supported"),
            )),
        }
    }

    /// The result of the `initialize` method.
    fn initialize_result(&self) -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
        })
    }

    /// The descriptors of the tools the policy allows.
    fn tool_list(&self) -> Vec<Value> {
        let mut tools: Vec<Value> = Vec::new();
        for (name, description, schema) in tool_descriptors() {
            if !self.policy.allows_tool(name) {
                continue;
            }
            tools.push(json!({
                "name": format!("michelangelo_{name}"),
                "description": description,
                "inputSchema": schema,
            }));
        }
        tools
    }

    /// Handle a `tools/call` request.
    fn handle_call(&mut self, id: Value, params: Option<&Value>) -> String {
        let params = match params {
            Some(Value::Object(map)) => map,
            Some(Value::Null) | None => {
                return error_response(
                    id,
                    ERROR_INVALID_PARAMS,
                    "the request carries no parameters",
                )
            }
            Some(_) => {
                return error_response(
                    id,
                    ERROR_INVALID_PARAMS,
                    "the parameters of the request are not an object",
                )
            }
        };
        let name = match params.get("name").and_then(|value| value.as_str()) {
            Some(name) => name.to_string(),
            None => {
                return error_response(id, ERROR_INVALID_PARAMS, "the request carries no tool name")
            }
        };
        let arguments = match params.get("arguments") {
            Some(Value::Object(map)) => Value::Object(map.clone()),
            Some(Value::Null) | None => json!({}),
            Some(_) => {
                return error_response(
                    id,
                    ERROR_INVALID_PARAMS,
                    "the arguments of the tool are not an object",
                )
            }
        };

        match self.call_tool(&name, &arguments) {
            Ok(value) => result_response(id, tool_result(value)),
            Err(error) => error_response(id, error.code, &error.message),
        }
    }

    /// Call one tool of the surface.
    fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, RpcError> {
        let short = match name {
            "michelangelo_get_task" => TOOL_GET_TASK,
            "michelangelo_validate_package" => TOOL_VALIDATE_PACKAGE,
            "michelangelo_submit_candidate" => TOOL_SUBMIT_CANDIDATE,
            "michelangelo_get_report" => TOOL_GET_REPORT,
            other => {
                return Err(RpcError::new(
                    ERROR_INVALID_PARAMS,
                    format!("the tool '{other}' is not one of the four tools of the surface"),
                ))
            }
        };
        if !self.policy.allows_tool(short) {
            return Err(RpcError::new(
                ERROR_POLICY_DENIED,
                format!("the policy of the workspace does not allow the tool '{short}'"),
            ));
        }

        let audit_run = self.audit_run(short, arguments);
        let outcome = self.dispatch(short, arguments);
        let decision = match &outcome {
            Ok(value) => decision_of(value),
            Err(_) => DECISION_ERROR,
        };
        if let Some(run_dir) = audit_run {
            let _ = self.write_audit(&run_dir, short, decision, arguments, &outcome);
        }
        outcome
    }

    /// The run of a call (the audit log is per-run).
    fn audit_run(&self, tool: &str, arguments: &Value) -> Option<PathBuf> {
        let run_id = arguments.get("run_id").and_then(|value| value.as_str())?;
        let _ = tool;
        self.resolve_run(run_id).ok()
    }

    /// Dispatch a tool call.
    fn dispatch(&mut self, tool: &str, arguments: &Value) -> Result<Value, RpcError> {
        let context = ToolContext::new(self.workspace.clone(), &self.policy, self.clock.as_ref());
        match tool {
            TOOL_GET_TASK => {
                let run_id = required_string(arguments, "run_id")?;
                let unknown = optional_string(arguments, "unknown")?;
                let run_dir = self.resolve_run(&run_id)?;
                let outcome = tools::get_task(&context, &run_dir, unknown.as_deref())
                    .map_err(|error| map_error(error, &self.workspace))?;
                outcome
                    .to_value()
                    .map_err(|error| map_error(error, &self.workspace))
            }
            TOOL_VALIDATE_PACKAGE => {
                let package = required_string(arguments, "package")?;
                let package_dir = self.resolve_inside(&self.workspace, &package)?;
                let outcome = tools::validate_package(&context, &package_dir)
                    .map_err(|error| map_error(error, &self.workspace))?;
                outcome
                    .to_value()
                    .map_err(|error| map_error(error, &self.workspace))
            }
            TOOL_GET_REPORT => {
                let run_id = required_string(arguments, "run_id")?;
                let run_dir = self.resolve_run(&run_id)?;
                let outcome = tools::get_report(&context, &run_dir)
                    .map_err(|error| map_error(error, &self.workspace))?;
                outcome
                    .to_value()
                    .map_err(|error| map_error(error, &self.workspace))
            }
            TOOL_SUBMIT_CANDIDATE => {
                let run_id = required_string(arguments, "run_id")?;
                let task_id = required_string(arguments, "task_id")?;
                let candidate = required_string(arguments, "candidate_dir")?;
                let run_dir = self.resolve_run(&run_id)?;
                let task = self.load_task_of_run(&run_dir, &task_id)?;
                let candidate_dir = self.resolve_candidate(&run_dir, &candidate)?;
                let outcome = tools::submit_candidate(&context, &run_dir, &task, &candidate_dir)
                    .map_err(|error| map_error(error, &self.workspace))?;
                outcome
                    .to_value()
                    .map_err(|error| map_error(error, &self.workspace))
            }
            _ => Err(RpcError::new(
                ERROR_INVALID_PARAMS,
                "the tool is not one of the four tools of the surface",
            )),
        }
    }

    /// Read the task of a run.
    fn load_task_of_run(
        &self,
        run_dir: &Path,
        task_id: &str,
    ) -> Result<crate::AssistantTask, RpcError> {
        if !crate::is_safe_relative_path(task_id) || task_id.contains('/') || task_id.contains('\\')
        {
            return Err(RpcError::new(
                ERROR_PATH_NOT_ALLOWED,
                "the task id is not a usable name",
            ));
        }
        let path = task_path(run_dir, task_id);
        if !path.is_file() {
            return Err(RpcError::new(
                ERROR_NOT_FOUND,
                format!("the task '{task_id}' does not exist"),
            ));
        }
        load_task(&path).map_err(|error| map_error(error, &self.workspace))
    }

    /// Resolve the directory of a candidate (workspace-relative or
    /// run-relative).
    fn resolve_candidate(&self, run_dir: &Path, relative: &str) -> Result<PathBuf, RpcError> {
        match self.resolve_inside(&self.workspace, relative) {
            Ok(path) => Ok(path),
            Err(first) => self.resolve_inside(run_dir, relative).map_err(|_| first),
        }
    }

    /// Resolve a run of the workspace.
    fn resolve_run(&self, run_id: &str) -> Result<PathBuf, RpcError> {
        let trimmed = run_id.trim();
        if trimmed.is_empty() {
            return Err(RpcError::new(ERROR_INVALID_PARAMS, "the run id is empty"));
        }
        if trimmed.contains('/') || trimmed.contains('\\') {
            let path = self.resolve_inside(&self.workspace, trimmed)?;
            if !path.join("report.json").is_file() {
                return Err(RpcError::new(
                    ERROR_NOT_FOUND,
                    format!("the run '{trimmed}' does not exist"),
                ));
            }
            return Ok(path);
        }
        // The runs live in <workspace>/recon/<revision>/<run>; a workspace
        // that carries the runs directly is accepted as well.
        let mut parents: Vec<PathBuf> = Vec::new();
        let recon = self.workspace.join("recon");
        if recon.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&recon) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        parents.push(path);
                    }
                }
            }
        }
        parents.push(self.workspace.clone());
        for parent in parents {
            let candidate = parent.join(trimmed);
            if !candidate.is_dir() {
                continue;
            }
            let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
                RpcError::new(
                    ERROR_NOT_FOUND,
                    format!("the run '{trimmed}' cannot be resolved: {error}"),
                )
            })?;
            if !canonical.starts_with(&self.workspace) {
                return Err(RpcError::new(
                    ERROR_PATH_NOT_ALLOWED,
                    "the run resolves outside the workspace",
                ));
            }
            return Ok(canonical);
        }
        Err(RpcError::new(
            ERROR_NOT_FOUND,
            format!("the run '{trimmed}' does not exist"),
        ))
    }

    /// Resolve a path inside a directory of the workspace.
    fn resolve_inside(&self, base: &Path, relative: &str) -> Result<PathBuf, RpcError> {
        let trimmed = relative.trim();
        if trimmed.is_empty() {
            return Err(RpcError::new(ERROR_INVALID_PARAMS, "the path is empty"));
        }
        let candidate = Path::new(trimmed);
        if candidate.is_absolute() || crate::is_escaping_path(trimmed) {
            return Err(RpcError::new(
                ERROR_PATH_NOT_ALLOWED,
                "an absolute or escaping path is not allowed",
            ));
        }
        let joined = base.join(candidate);
        let canonical = std::fs::canonicalize(&joined).map_err(|_| {
            RpcError::new(
                ERROR_NOT_FOUND,
                format!("the path '{trimmed}' does not exist"),
            )
        })?;
        if !canonical.starts_with(base) {
            return Err(RpcError::new(
                ERROR_PATH_NOT_ALLOWED,
                "the path resolves outside the workspace",
            ));
        }
        Ok(canonical)
    }

    /// Append one line to the audit log of a run.
    fn write_audit(
        &self,
        run_dir: &Path,
        tool: &str,
        decision: &str,
        arguments: &Value,
        outcome: &Result<Value, RpcError>,
    ) -> Result<(), AssistantError> {
        let store = BudgetStore::new(run_dir);
        let candidate_id = match outcome {
            Ok(value) => value
                .get("candidate_id")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            Err(_) => None,
        };
        let task_id = arguments
            .get("task_id")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let detail = match outcome {
            Ok(value) => match value.get("task") {
                Some(task) => format!(
                    "the task '{}' was written",
                    task.get("task_id")
                        .and_then(|value| value.as_str())
                        .unwrap_or("-")
                ),
                None => match value.get("decision") {
                    Some(decision) => format!(
                        "the decision of the candidate is '{}'",
                        decision.as_str().unwrap_or("-")
                    ),
                    None => "the call succeeded".to_string(),
                },
            },
            Err(error) => sanitize_paths(&error.message, &[self.workspace.as_path()]),
        };
        let entry = AuditEntry {
            ts: self.clock.now_rfc3339_utc(),
            tool: tool.to_string(),
            decision: decision.to_string(),
            candidate_id,
            task_id,
            detail,
        };
        store.append_audit(&entry)
    }
}

/// The descriptors of the four tools of the surface.
fn tool_descriptors() -> [(&'static str, &'static str, Value); 4] {
    [
        (
            TOOL_GET_TASK,
            "Return the bounded assistant task of a run. The task can be \
             narrowed to one unknown of the run.",
            json!({
                "type": "object",
                "properties": {
                    "run_id": {"type": "string", "description": "The run identifier."},
                    "unknown": {
                        "type": "string",
                        "description": "The unknown the task is narrowed to."
                    }
                },
                "required": ["run_id"],
                "additionalProperties": false
            }),
        ),
        (
            TOOL_VALIDATE_PACKAGE,
            "Validate an input package and return the summary of the report.",
            json!({
                "type": "object",
                "properties": {
                    "package": {"type": "string", "description": "The workspace-relative package path."},
                    "run_id": {"type": "string", "description": "The optional run that carries the audit line."}
                },
                "required": ["package"],
                "additionalProperties": false
            }),
        ),
        (
            TOOL_SUBMIT_CANDIDATE,
            "Submit a candidate into the store of a run; the decision is always 'proposed'.",
            json!({
                "type": "object",
                "properties": {
                    "run_id": {"type": "string", "description": "The run identifier."},
                    "task_id": {"type": "string", "description": "The identifier of the bounded task."},
                    "candidate_dir": {"type": "string", "description": "The workspace-relative candidate path."}
                },
                "required": ["run_id", "task_id", "candidate_dir"],
                "additionalProperties": false
            }),
        ),
        (
            TOOL_GET_REPORT,
            "Return the summary of the reconstruction report of a run.",
            json!({
                "type": "object",
                "properties": {"run_id": {"type": "string", "description": "The run identifier."}},
                "required": ["run_id"],
                "additionalProperties": false
            }),
        ),
    ]
}

/// Read a required string argument.
fn required_string(arguments: &Value, key: &str) -> Result<String, RpcError> {
    match arguments.get(key).and_then(|value| value.as_str()) {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        Some(_) => Err(RpcError::new(
            ERROR_INVALID_PARAMS,
            format!("the argument '{key}' is empty"),
        )),
        None => Err(RpcError::new(
            ERROR_INVALID_PARAMS,
            format!("the argument '{key}' is missing"),
        )),
    }
}

/// An optional string argument (None when the key is absent).
fn optional_string(arguments: &Value, key: &str) -> Result<Option<String>, RpcError> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_str() {
            Some(text) if !text.trim().is_empty() => Ok(Some(text.to_string())),
            Some(_) => Err(RpcError::new(
                ERROR_INVALID_PARAMS,
                format!("the argument '{key}' is empty"),
            )),
            None => Err(RpcError::new(
                ERROR_INVALID_PARAMS,
                format!("the argument '{key}' is not a string"),
            )),
        },
    }
}

/// The decision of a successful call.
fn decision_of(value: &Value) -> &'static str {
    match value.get("decision").and_then(|value| value.as_str()) {
        Some("proposed") => DECISION_PROPOSED,
        Some("rejected") => DECISION_REJECTED,
        _ => DECISION_OK,
    }
}

/// Map an error of the surface to a JSON-RPC error.
fn map_error(error: AssistantError, workspace: &Path) -> RpcError {
    let code = match &error {
        AssistantError::PolicyDenied { .. } => ERROR_POLICY_DENIED,
        AssistantError::PathNotAllowed { .. } => ERROR_PATH_NOT_ALLOWED,
        AssistantError::BudgetExhausted { .. } => ERROR_BUDGET_EXHAUSTED,
        AssistantError::TaskInvalid { .. } => ERROR_TASK_INVALID,
        AssistantError::NotFound { .. } => ERROR_NOT_FOUND,
        AssistantError::Io { .. } | AssistantError::Invalid { .. } => ERROR_INTERNAL,
    };
    // An error message may carry an absolute host path (for example when a
    // candidate geometry file is missing); the transport never exposes one.
    RpcError::new(code, sanitize_paths(&error.to_string(), &[workspace]))
}

/// The result of a tool call.
fn tool_result(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string());
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": value,
        "isError": false,
    })
}

/// Build a JSON-RPC result response.
fn result_response(id: Value, result: Value) -> String {
    let response = json!({"jsonrpc": "2.0", "id": id, "result": result});
    serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string())
}

/// Build a JSON-RPC error response.
fn error_response(id: Value, code: i64, message: &str) -> String {
    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message},
    });
    serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string())
}

/// The outcome of a tool call, used by the tests.
pub fn outcome_decision(outcome: &ToolOutcome) -> &'static str {
    match outcome {
        ToolOutcome::Decision(report) => match report.decision {
            michelangelo_edit::validate::Decision::Proposed => DECISION_PROPOSED,
            michelangelo_edit::validate::Decision::Rejected => DECISION_REJECTED,
            _ => DECISION_OK,
        },
        _ => DECISION_OK,
    }
}
