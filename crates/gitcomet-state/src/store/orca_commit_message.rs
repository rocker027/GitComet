//! Generates a commit message for the staged changes through an Orca agent.
//!
//! The agent is started as a chat with `orca agent launch`, and its reply is
//! read back with `orca agent read` once the turn is done. The reply is
//! untrusted text: it only ever fills the commit message input.

use super::orca_dispatcher::{OrcaCliDispatcher, parse_agent_launch, parse_installed_agents};
use gitcomet_core::worktree_finish::sanitize_assistant_message;
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

/// The prompt template used until the user saves their own in Settings.
pub const DEFAULT_COMMIT_MESSAGE_PROMPT: &str = "請依據下方已暫存的變更撰寫一則 Git 提交訊息。

格式：
- 第一行是標題，採用 Conventional Commits 格式，例如 feat(ui): 新增複製按鈕，不超過 72 字元。
- 第二行空白。
- 第三行起是內文，用條列說明改了什麼與為什麼。
- 使用繁體中文（台灣用語），程式碼識別字保留英文。";

const OUTPUT_INSTRUCTION: &str =
    "只輸出提交訊息本身，不要加任何說明、前言或 Markdown 程式碼區塊，也不要修改任何檔案。";
/// Linux caps one argument at 128 KiB; Windows caps the whole command line
/// at 32,767 UTF-16 units.
#[cfg(not(windows))]
const MAX_DIFF_BYTES: usize = 100_000;
#[cfg(windows)]
const MAX_DIFF_BYTES: usize = 24_000;
const MAX_MESSAGE_CHARS: usize = 8_000;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const REPLY_TIMEOUT: Duration = Duration::from_secs(300);

/// Reads the staged diff of `workdir`, asks the default Orca agent for a
/// commit message with `template`, and returns the agent's reply.
pub fn generate_commit_message(workdir: &Path, template: &str) -> Result<String, String> {
    let diff = read_staged_diff(workdir)?;
    let prompt = build_prompt(template, &diff);
    OrcaCliDispatcher::new().generate_commit_message(workdir, &prompt, POLL_INTERVAL, REPLY_TIMEOUT)
}

fn read_staged_diff(workdir: &Path) -> Result<String, String> {
    let output = gitcomet_core::process::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["diff", "--cached", "--no-color", "--no-ext-diff"])
        .output()
        .map_err(|err| format!("無法執行 git diff --cached：{err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff --cached 失敗：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let diff = String::from_utf8_lossy(&output.stdout).into_owned();
    if diff.trim().is_empty() {
        return Err("沒有已暫存的變更。".to_string());
    }
    Ok(diff)
}

/// The template, the output instruction, then the staged diff, cut at
/// [`MAX_DIFF_BYTES`] so the prompt stays within the argument size limit.
pub(crate) fn build_prompt(template: &str, diff: &str) -> String {
    let template = match template.trim() {
        "" => DEFAULT_COMMIT_MESSAGE_PROMPT,
        template => template,
    };
    let truncated = diff.len() > MAX_DIFF_BYTES;
    let diff = if truncated {
        let mut end = MAX_DIFF_BYTES;
        while !diff.is_char_boundary(end) {
            end -= 1;
        }
        &diff[..end]
    } else {
        diff
    };
    let mut prompt = format!(
        "{template}\n\n{OUTPUT_INSTRUCTION}\n\n已暫存的變更（git diff --cached）：\n{diff}"
    );
    if truncated {
        prompt.push_str(
            "\n\n（差異過長已截斷。需要完整內容時，請在工作目錄執行 git diff --cached。）",
        );
    }
    prompt
}

/// The reply without surrounding whitespace or a wrapping code fence.
pub(crate) fn clean_reply(reply: &str) -> String {
    let trimmed = reply.trim();
    let unfenced = trimmed
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
        .map(|body| body.split_once('\n').map_or(body, |(_, rest)| rest))
        .unwrap_or(trimmed);
    sanitize_assistant_message(unfenced, MAX_MESSAGE_CHARS)
}

/// The text of the newest assistant message once the latest turn is done,
/// or an error when the agent stopped to wait for the user.
pub(crate) fn finished_reply(json: &Value) -> Result<Option<String>, String> {
    let Some(result) = json.get("result") else {
        return Ok(None);
    };
    match result.get("state").and_then(Value::as_str) {
        Some("done") => {}
        Some(state @ ("blocked" | "waiting")) => {
            return Err(format!(
                "AI agent 正在等待處理（{state}），請到 Orca 查看。"
            ));
        }
        _ => return Ok(None),
    }
    Ok(result
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .find(|entry| {
            entry.get("kind").and_then(Value::as_str) == Some("message")
                && entry.get("role").and_then(Value::as_str) == Some("assistant")
        })
        .and_then(|entry| entry.get("text").and_then(Value::as_str))
        .map(ToOwned::to_owned))
}

impl OrcaCliDispatcher {
    pub(crate) fn generate_commit_message(
        &self,
        workdir: &Path,
        prompt: &str,
        poll_interval: Duration,
        timeout: Duration,
    ) -> Result<String, String> {
        let selector = format!("path:{}", workdir.display());
        let agent = self.default_agent_id(&selector)?;
        let launch_args = [
            "agent",
            "launch",
            "--agent",
            agent.as_str(),
            "--worktree",
            selector.as_str(),
            "--surface",
            "chat",
            "--prompt",
            prompt,
            "--json",
        ]
        .map(str::to_owned);
        let launch = parse_agent_launch(&self.run_orca(&launch_args)?);
        let session_id = launch
            .session_id
            .ok_or_else(|| "Orca 沒有回傳 agent 的 session ID。".to_string())?;
        let read_args = [
            "agent".to_owned(),
            "read".to_owned(),
            format!("orca_session_id:{session_id}"),
            "--limit".to_owned(),
            "5".to_owned(),
            "--json".to_owned(),
        ];
        let deadline = Instant::now() + timeout;
        loop {
            std::thread::sleep(poll_interval);
            if let Some(reply) = finished_reply(&self.run_orca(&read_args)?)? {
                let message = clean_reply(&reply);
                if message.is_empty() {
                    return Err("AI 沒有回傳提交訊息。".to_string());
                }
                return Ok(message);
            }
            if Instant::now() >= deadline {
                return Err(format!("等待 AI 回覆逾時（{} 秒）。", timeout.as_secs()));
            }
        }
    }

    fn default_agent_id(&self, selector: &str) -> Result<String, String> {
        let args = ["agent", "installed", "--worktree", selector, "--json"].map(str::to_owned);
        let agents = parse_installed_agents(&self.run_orca(&args)?);
        agents
            .iter()
            .find(|agent| agent.is_default)
            .or_else(|| agents.first())
            .map(|agent| agent.id.clone())
            .ok_or_else(|| "Orca 找不到已安裝的 agent。".to_string())
    }

    fn run_orca(&self, args: &[String]) -> Result<Value, String> {
        self.run_json(args, None).map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_falls_back_to_default_and_truncates_long_diffs() {
        let prompt = build_prompt("  ", "diff --git a/x b/x");
        assert!(prompt.starts_with(DEFAULT_COMMIT_MESSAGE_PROMPT));
        assert!(prompt.contains(OUTPUT_INSTRUCTION));
        assert!(prompt.ends_with("diff --git a/x b/x"));

        let long_diff = format!("a{}", "ж".repeat(MAX_DIFF_BYTES));
        let prompt = build_prompt("自訂模板", &long_diff);
        assert!(prompt.starts_with("自訂模板"));
        assert!(prompt.contains("差異過長已截斷"));
        assert_eq!(
            prompt.matches('ж').count(),
            (MAX_DIFF_BYTES - 1) / 'ж'.len_utf8()
        );
    }

    #[test]
    fn clean_reply_strips_whitespace_and_code_fences() {
        assert_eq!(clean_reply("  feat: x\n\nbody  "), "feat: x\n\nbody");
        assert_eq!(
            clean_reply("```text\nfeat: x\n\nbody\n```"),
            "feat: x\n\nbody"
        );
        assert_eq!(clean_reply("```\nfix: y\n```"), "fix: y");
    }

    #[test]
    fn finished_reply_waits_for_done_and_takes_the_last_assistant_message() {
        let working = serde_json::json!({
            "ok": true,
            "result": {"state": "working", "entries": [
                {"kind": "message", "role": "assistant", "text": "partial"}
            ]}
        });
        assert_eq!(finished_reply(&working), Ok(None));

        let blocked =
            serde_json::json!({"ok": true, "result": {"state": "blocked", "entries": []}});
        assert!(finished_reply(&blocked).unwrap_err().contains("blocked"));

        let done = serde_json::json!({
            "ok": true,
            "result": {"state": "done", "entries": [
                {"kind": "message", "role": "user", "text": "prompt"},
                {"kind": "message", "role": "assistant", "text": "first"},
                {"kind": "tool", "name": "Bash", "state": "completed"},
                {"kind": "message", "role": "assistant", "text": "feat: final"}
            ]}
        });
        assert_eq!(finished_reply(&done), Ok(Some("feat: final".to_string())));
    }

    #[cfg(unix)]
    #[test]
    fn fake_orca_cli_generates_message_from_launch_and_read() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let fake_orca = dir.path().join("fake-orca");
        let arg_dump = dir.path().join("args.txt");
        let script = format!(
            r#"#!/bin/sh
printf '%s\n' "$@" >> "{dump}"
case "$2" in
  installed) printf '{{"ok":true,"result":{{"agents":[{{"id":"codex","default":false}},{{"id":"claude","default":true}}]}}}}\n' ;;
  launch) printf '{{"ok":true,"result":{{"outcome":{{"kind":"structured","sessionId":"sess_9"}}}}}}\n' ;;
  read) printf '{{"ok":true,"result":{{"state":"done","entries":[{{"kind":"message","role":"assistant","text":"feat: generated\\n\\n- body"}}]}}}}\n' ;;
esac
"#,
            dump = arg_dump.display()
        );
        std::fs::write(&fake_orca, script).expect("write fake orca");
        let mut perms = std::fs::metadata(&fake_orca).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_orca, perms).unwrap();

        let message = OrcaCliDispatcher::with_executable(fake_orca)
            .generate_commit_message(
                dir.path(),
                "prompt with $(touch should_not_exist)",
                Duration::ZERO,
                Duration::from_secs(5),
            )
            .expect("generate via fake orca");
        assert_eq!(message, "feat: generated\n\n- body");
        assert!(!dir.path().join("should_not_exist").exists());

        let dumped = std::fs::read_to_string(&arg_dump).expect("read dumped args");
        assert!(dumped.contains("--agent\nclaude\n"));
        assert!(dumped.contains("--surface\nchat\n"));
        assert!(dumped.contains("orca_session_id:sess_9"));
    }
}
