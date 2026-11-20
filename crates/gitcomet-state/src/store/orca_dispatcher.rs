//! Orca CLI implementation of [`gitcomet_core::worktree_finish::AgentDispatcher`].
//!
//! Invokes the resolved `orca` binary strictly via argument arrays (never a shell)
//! and treats all assistant output as untrusted display-only text.

use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use gitcomet_core::worktree_finish::{
    AgentDispatchAvailability, AgentDispatcher, AgentLaunch, AgentSession, AgentSessionState,
    InstalledAgent, sanitize_assistant_message,
};
use serde_json::Value;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_ASSISTANT_MESSAGE_CHARS: usize = 280;

/// Production dispatcher that delegates to the local `orca` CLI.
#[derive(Clone, Debug, Default)]
pub struct OrcaCliDispatcher {
    override_executable: Option<PathBuf>,
}

impl OrcaCliDispatcher {
    pub fn new() -> Self {
        Self {
            override_executable: None,
        }
    }

    /// Creates a dispatcher pinned to an explicit executable path (used in tests
    /// with fake CLI scripts and custom overrides).
    pub fn with_executable(executable: PathBuf) -> Self {
        Self {
            override_executable: Some(executable),
        }
    }

    fn resolve_executable(&self) -> Option<PathBuf> {
        if let Some(path) = &self.override_executable {
            return path.is_file().then(|| path.clone());
        }
        resolve_orca_executable_with_env(
            env::var_os("ORCA_CLI_COMMAND"),
            env::var_os("PATH"),
            env::var_os("PATHEXT"),
            env::var_os("HOME"),
        )
    }

    fn run_json(&self, args: &[String], cancellation: Option<&CancellationToken>) -> Result<Value> {
        if let Some(token) = cancellation {
            token.check_cancelled()?;
        }
        let Some(exe) = self.resolve_executable() else {
            return Err(Error::new(ErrorKind::Backend(
                "Orca CLI executable was not found".to_string(),
            )));
        };

        let output = Command::new(&exe)
            .args(args)
            .output()
            .map_err(|err| Error::new(ErrorKind::Io(err.kind())))?;

        if let Some(token) = cancellation {
            token.check_cancelled()?;
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if let Ok(parsed) = serde_json::from_str::<Value>(&stdout) {
            let err_op_id = extract_operation_id(&parsed);
            if parsed.get("ok").and_then(Value::as_bool) == Some(false) {
                let mut msg = extract_error_message(&parsed)
                    .or_else(|| (!stderr.is_empty()).then_some(stderr))
                    .unwrap_or_else(|| "Orca command returned ok=false".to_string());
                if let Some(op_id) = err_op_id
                    && !msg.contains("--retry-launch")
                {
                    msg.push_str(&format!(" (--retry-launch {op_id})"));
                }
                return Err(Error::new(ErrorKind::Backend(msg)));
            }
            if output.status.success() {
                return Ok(parsed);
            }
            let mut msg = extract_error_message(&parsed)
                .or_else(|| (!stderr.is_empty()).then_some(stderr))
                .unwrap_or_else(|| format!("Orca exited with status {}", output.status));
            if let Some(op_id) = err_op_id
                && !msg.contains("--retry-launch")
            {
                msg.push_str(&format!(" (--retry-launch {op_id})"));
            }
            return Err(Error::new(ErrorKind::Backend(msg)));
        }

        if !output.status.success() {
            let detail = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("Orca exited with status {}", output.status)
            };
            return Err(Error::new(ErrorKind::Backend(detail)));
        }

        Err(Error::new(ErrorKind::Backend(format!(
            "Orca returned non-JSON output: {}",
            sanitize_assistant_message(&stdout, 160)
        ))))
    }
}

impl AgentDispatcher for OrcaCliDispatcher {
    fn availability(&self) -> AgentDispatchAvailability {
        match self.resolve_executable() {
            Some(executable) => AgentDispatchAvailability::Available { executable },
            None => AgentDispatchAvailability::NotFound,
        }
    }

    fn installed(
        &self,
        workspace: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Vec<InstalledAgent>> {
        let selector = format!("path:{}", workspace.display());
        let args = vec![
            "agent".to_string(),
            "installed".to_string(),
            "--worktree".to_string(),
            selector,
            "--json".to_string(),
        ];
        let json = self.run_json(&args, Some(cancellation))?;
        Ok(parse_installed_agents(&json))
    }

    fn launch(
        &self,
        workspace: &Path,
        agent_id: &str,
        prompt: &str,
        retry_operation_id: Option<&str>,
    ) -> Result<AgentLaunch> {
        let selector = format!("path:{}", workspace.display());
        let mut args = vec![
            "agent".to_string(),
            "launch".to_string(),
            "--agent".to_string(),
            agent_id.to_string(),
            "--worktree".to_string(),
            selector,
            "--prompt".to_string(),
            prompt.to_string(),
            "--json".to_string(),
        ];
        if let Some(op_id) = retry_operation_id {
            args.push("--retry-launch".to_string());
            args.push(op_id.to_string());
        }
        let json = self.run_json(&args, None)?;
        Ok(parse_agent_launch(&json))
    }

    fn sessions(
        &self,
        workspace: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Vec<AgentSession>> {
        let selector = format!("path:{}", workspace.display());
        let args = vec![
            "agent".to_string(),
            "list".to_string(),
            "--worktree".to_string(),
            selector,
            "--json".to_string(),
        ];
        let json = self.run_json(&args, Some(cancellation))?;
        Ok(parse_agent_sessions(&json))
    }
}

pub(crate) fn resolve_orca_executable_with_env(
    orca_cli_command: Option<OsString>,
    path_env: Option<OsString>,
    pathext_env: Option<OsString>,
    home_env: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(raw) = orca_cli_command.filter(|value| !value.is_empty()) {
        let raw_str = raw.to_string_lossy();
        let trimmed = raw_str.trim();
        if !trimmed.is_empty() {
            if let Some(found) =
                find_executable_in_path(trimmed, path_env.as_ref(), pathext_env.as_ref())
            {
                return Some(found);
            }
            let direct = PathBuf::from(trimmed);
            if direct.is_file() {
                return Some(direct);
            }
        }
    }

    if let Some(found) = find_executable_in_path("orca", path_env.as_ref(), pathext_env.as_ref()) {
        return Some(found);
    }

    // macOS/Linux GUI apps may inherit a minimal PATH without user bin dirs.
    #[cfg(unix)]
    {
        let mut fallback_dirs = vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ];
        if let Some(home) = home_env.filter(|v| !v.is_empty()) {
            let home = PathBuf::from(home);
            fallback_dirs.insert(0, home.join(".local").join("bin"));
        }
        for dir in fallback_dirs {
            let candidate = dir.join("orca");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = home_env;

    None
}

fn find_executable_in_path(
    name: &str,
    path_env: Option<&OsString>,
    pathext_env: Option<&OsString>,
) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let path_env = path_env?;
    #[cfg(target_os = "windows")]
    let pathext: Vec<String> = pathext_env
        .and_then(|v| v.clone().into_string().ok())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase())
        .collect();
    #[cfg(not(target_os = "windows"))]
    let _ = pathext_env;

    for dir in env::split_paths(path_env) {
        let joined = dir.join(name);
        if joined.is_file() {
            return Some(joined);
        }
        #[cfg(target_os = "windows")]
        if candidate.extension().is_none() {
            for ext in &pathext {
                let with_ext = dir.join(format!("{name}{ext}"));
                if with_ext.is_file() {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

fn extract_operation_id(json: &Value) -> Option<String> {
    json.get("operationId")
        .or_else(|| json.get("error").and_then(|e| e.get("operationId")))
        .or_else(|| json.get("result").and_then(|r| r.get("operationId")))
        .or_else(|| {
            json.get("result")
                .and_then(|r| r.get("receipt"))
                .and_then(|rc| rc.get("operationId"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

/// Extracts `--retry-launch <operationId>` from an Orca CLI error message when a launch reply was lost.
pub(crate) fn extract_retry_operation_id(text: &str) -> Option<String> {
    let idx = text.find("--retry-launch")?;
    let rest = text[idx + "--retry-launch".len()..].trim_start();
    let token = rest
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ')' | ']' | ',' | ';' | '"' | '\''))
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some(token.to_string())
}

fn extract_error_message(json: &Value) -> Option<String> {
    if let Some(err) = json.get("error") {
        if let Some(msg) = err.as_str() {
            return Some(msg.to_string());
        }
        if let Some(msg) = err.get("message").and_then(Value::as_str) {
            return Some(msg.to_string());
        }
    }
    json.get("message")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(crate) fn parse_installed_agents(json: &Value) -> Vec<InstalledAgent> {
    let list = json
        .get("result")
        .and_then(|r| r.get("agents").or(Some(r)))
        .and_then(Value::as_array);
    let Some(items) = list else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id);
            let is_default = item
                .get("default")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some(InstalledAgent {
                id: id.to_string(),
                name: name.to_string(),
                is_default,
            })
        })
        .collect()
}

pub(crate) fn parse_agent_launch(json: &Value) -> AgentLaunch {
    let result = json.get("result").unwrap_or(json);
    let session_id = result
        .get("orcaSessionId")
        .or_else(|| result.get("sessionId"))
        .or_else(|| result.get("receipt").and_then(|r| r.get("orcaSessionId")))
        .or_else(|| result.get("receipt").and_then(|r| r.get("sessionId")))
        .or_else(|| result.get("outcome").and_then(|o| o.get("orcaSessionId")))
        .or_else(|| result.get("outcome").and_then(|o| o.get("sessionId")))
        .or_else(|| result.get("session").and_then(|s| s.get("orcaSessionId")))
        .or_else(|| result.get("agent").and_then(|a| a.get("orcaSessionId")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let operation_id = result
        .get("operationId")
        .or_else(|| result.get("receipt").and_then(|r| r.get("operationId")))
        .or_else(|| json.get("operationId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let outcome_kind = result
        .get("outcome")
        .and_then(|o| o.get("kind"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let detail = result
        .get("receipt")
        .and_then(|r| r.get("detail"))
        .and_then(Value::as_str)
        .map(|s| sanitize_assistant_message(s, MAX_ASSISTANT_MESSAGE_CHARS));

    AgentLaunch {
        session_id,
        operation_id,
        outcome_kind,
        detail,
    }
}

pub(crate) fn parse_agent_sessions(json: &Value) -> Vec<AgentSession> {
    let list = json
        .get("result")
        .and_then(|r| r.get("agents").or(Some(r)))
        .and_then(Value::as_array);
    let Some(items) = list else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let session_id = item
                .get("orcaSessionId")
                .or_else(|| item.get("sessionId"))
                .and_then(Value::as_str)?
                .trim();
            if session_id.is_empty() {
                return None;
            }
            let agent_id = item
                .get("agent")
                .or_else(|| item.get("agentId"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let state = match item
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
            {
                "working" => AgentSessionState::Working,
                "blocked" => AgentSessionState::Blocked,
                "waiting" => AgentSessionState::Waiting,
                "done" => AgentSessionState::Done,
                other => AgentSessionState::Unknown(other.to_string()),
            };
            let worktree_id = item
                .get("worktreeId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let updated_at = item
                .get("updatedAt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let last_assistant_message = item
                .get("lastAssistantMessage")
                .and_then(Value::as_str)
                .map(|raw| sanitize_assistant_message(raw, MAX_ASSISTANT_MESSAGE_CHARS))
                .filter(|s| !s.is_empty());

            Some(AgentSession {
                session_id: session_id.to_string(),
                agent_id,
                state,
                worktree_id,
                updated_at,
                last_assistant_message,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_installed_launch_and_truncates_untrusted_assistant_message() {
        let installed_json: Value = serde_json::json!({
            "ok": true,
            "result": {
                "agents": [
                    {"id": "claude", "name": "Claude", "default": true},
                    {"id": "codex", "name": "Codex", "default": false}
                ]
            }
        });
        let agents = parse_installed_agents(&installed_json);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id, "claude");
        assert!(agents[0].is_default);

        let launch_json: Value = serde_json::json!({
            "ok": true,
            "result": {
                "orcaSessionId": "sess_exact_42",
                "operationId": "op_99",
                "outcome": {"kind": "launched"},
                "receipt": {"detail": "started"}
            }
        });
        let launch = parse_agent_launch(&launch_json);
        assert_eq!(launch.session_id.as_deref(), Some("sess_exact_42"));
        assert_eq!(launch.operation_id.as_deref(), Some("op_99"));

        let long_msg = format!("$(rm -rf /); {}", "x".repeat(400));
        let sessions_json: Value = serde_json::json!({
            "ok": true,
            "result": {
                "agents": [{
                    "orcaSessionId": "sess_exact_42",
                    "agent": "claude",
                    "state": "done",
                    "worktreeId": "wt_1",
                    "lastAssistantMessage": long_msg
                }]
            }
        });
        let sessions = parse_agent_sessions(&sessions_json);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].state, AgentSessionState::Done);
        let msg = sessions[0].last_assistant_message.as_ref().unwrap();
        assert!(msg.chars().count() <= MAX_ASSISTANT_MESSAGE_CHARS + 1);
        assert!(msg.ends_with('…'));
    }

    #[cfg(unix)]
    #[test]
    fn fake_orca_cli_invokes_via_argv_without_shell_expansion() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let fake_orca = dir.path().join("fake-orca");
        let arg_dump = dir.path().join("args.txt");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nprintf '{{\"ok\":true,\"result\":{{\"orcaSessionId\":\"sess_1\",\"operationId\":\"op_1\"}}}}\\n'\n",
            arg_dump.display()
        );
        std::fs::write(&fake_orca, script).expect("write fake orca");
        let mut perms = std::fs::metadata(&fake_orca).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_orca, perms).unwrap();

        let dispatcher = OrcaCliDispatcher::with_executable(fake_orca);
        let marker_file = dir.path().join("should_not_exist");
        let hostile_prompt = format!("$(touch {}) `id`; \"quoted\"", marker_file.display());
        let workspace = dir.path().join("wt space");
        let launch = dispatcher
            .launch(&workspace, "claude", &hostile_prompt, None)
            .expect("launch via fake orca");
        assert_eq!(launch.session_id.as_deref(), Some("sess_1"));
        assert!(
            !marker_file.exists(),
            "shell metacharacters must never execute"
        );

        let dumped = std::fs::read_to_string(&arg_dump).expect("read dumped args");
        assert!(dumped.contains(&hostile_prompt));
        assert!(dumped.contains(&format!("path:{}", workspace.display())));
    }

    #[test]
    fn extracts_retry_launch_operation_id_and_handles_missing_session_receipt() {
        assert_eq!(
            extract_retry_operation_id(
                "launch reply lost; re-issue with --retry-launch op_abc-123 from same dir"
            )
            .as_deref(),
            Some("op_abc-123")
        );
        assert_eq!(
            extract_retry_operation_id("Orca returned ok=false (--retry-launch op_json_9)")
                .as_deref(),
            Some("op_json_9")
        );
        assert_eq!(extract_retry_operation_id("ordinary error"), None);

        let partial_receipt: Value = serde_json::json!({
            "ok": true,
            "result": {
                "outcome": {"kind": "terminal_fallback"},
                "receipt": {
                    "operationId": "op_receipt_77",
                    "detail": "Opened terminal fallback without chat session id"
                }
            }
        });
        let parsed = parse_agent_launch(&partial_receipt);
        assert_eq!(parsed.session_id, None);
        assert_eq!(parsed.operation_id.as_deref(), Some("op_receipt_77"));
        assert_eq!(parsed.outcome_kind.as_deref(), Some("terminal_fallback"));
        assert!(parsed.detail.is_some());

        let nested_receipt: Value = serde_json::json!({
            "ok": true,
            "result": {
                "outcome": {
                    "kind": "chat",
                    "orcaSessionId": "sess_nested_99"
                },
                "receipt": {
                    "operationId": "op_nested_99"
                }
            }
        });
        let nested = parse_agent_launch(&nested_receipt);
        assert_eq!(nested.session_id.as_deref(), Some("sess_nested_99"));
        assert_eq!(nested.operation_id.as_deref(), Some("op_nested_99"));
    }
}
