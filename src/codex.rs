//! Direct, Codex CLI-only cue execution.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use crate::cue::Cue;
use crate::review::{Message, Speaker};

const POLL_INTERVAL: Duration = Duration::from_millis(75);
pub const DEFAULT_MODEL: &str = "gpt-5.6";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexConfig {
    pub cli_path: PathBuf,
    pub model: String,
    pub extra_arguments: String,
    /// Names to copy from the app's runtime environment into Codex.
    pub environment_names: Vec<String>,
    /// One executable plus arguments, parsed without a shell.
    pub pre_run_command: String,
    /// One executable plus arguments, parsed without a shell.
    pub post_run_command: String,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            cli_path: PathBuf::from("codex"),
            model: DEFAULT_MODEL.to_owned(),
            extra_arguments: String::new(),
            environment_names: Vec::new(),
            pre_run_command: String::new(),
            post_run_command: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexEvent {
    Progress(String),
    Completed { summary: String },
    Cancelled,
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    #[error("Codex CLI could not be started: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("invalid Codex arguments: {0}")]
    InvalidArguments(String),
    #[error("invalid environment variable name: {0}")]
    InvalidEnvironmentName(String),
}

pub struct CodexRun {
    receiver: Receiver<CodexEvent>,
    cancel: Arc<AtomicBool>,
}

impl CodexRun {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    /// Receive the next available progress event without blocking the UI.
    ///
    /// # Errors
    ///
    /// Returns `Disconnected` after the execution worker exits and all queued
    /// events have been consumed.
    pub fn try_recv(&self) -> Result<CodexEvent, TryRecvError> {
        self.receiver.try_recv()
    }
}

impl Drop for CodexRun {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Start one Codex CLI execution in a background worker.
///
/// # Errors
///
/// Returns an error before spawning when extra arguments, hook commands, or
/// environment-variable names are invalid.
pub fn start(
    repository: &Path,
    prompt: String,
    config: CodexConfig,
) -> Result<CodexRun, CodexError> {
    let repository = repository.to_path_buf();
    let extra_arguments = parse_arguments(&config.extra_arguments)?;
    let pre_run = parse_command(&config.pre_run_command)?;
    let post_run = parse_command(&config.post_run_command)?;
    validate_environment_names(&config.environment_names)?;
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || {
        if let Err(error) = execute(
            &repository,
            &prompt,
            &config,
            &extra_arguments,
            pre_run.as_deref(),
            post_run.as_deref(),
            &worker_cancel,
            &sender,
        ) {
            let _ = sender.send(CodexEvent::Failed(error.to_string()));
        }
    });
    Ok(CodexRun { receiver, cancel })
}

#[must_use]
pub fn follow_up_prompt(cue: &Cue, messages: &[Message]) -> String {
    let mut prompt = cue.prompt();
    prompt.push_str(
        "\n\nThis is a refinement of an earlier run. The current working tree contains the result under review. Preserve good existing changes and address the latest feedback.\n\nConversation:\n",
    );
    for message in messages {
        let speaker = match message.speaker {
            Speaker::User => "User",
            Speaker::Codex => "Codex",
        };
        prompt.push_str(speaker);
        prompt.push_str(": ");
        prompt.push_str(&message.text);
        prompt.push('\n');
    }
    prompt
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn execute(
    repository: &Path,
    prompt: &str,
    config: &CodexConfig,
    extra_arguments: &[String],
    pre_run: Option<&[String]>,
    post_run: Option<&[String]>,
    cancel: &AtomicBool,
    sender: &mpsc::Sender<CodexEvent>,
) -> std::io::Result<()> {
    if let Some(command) = pre_run {
        run_hook("Pre-run", command, repository, sender)?;
    }
    if cancel.load(Ordering::Acquire) {
        let _ = sender.send(CodexEvent::Cancelled);
        return Ok(());
    }

    let mut command = Command::new(&config.cli_path);
    command
        .args([
            "exec",
            "--json",
            "--color",
            "never",
            "--sandbox",
            "workspace-write",
            "-C",
        ])
        .arg(repository);
    if !config.model.trim().is_empty() {
        command.args(["--model", config.model.trim()]);
    }
    command.args(extra_arguments).arg("-");
    command
        .current_dir(repository)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "PATH",
        "HOME",
        "CODEX_HOME",
        "TMPDIR",
        "USER",
        "LOGNAME",
        "SHELL",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for name in &config.environment_names {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("Codex stdout was not piped"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("Codex stderr was not piped"))?;
    let (line_sender, line_receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line_sender.send(line).is_err() {
                break;
            }
        }
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    let mut summary = String::new();
    loop {
        while let Ok(line) = line_receiver.try_recv() {
            match line {
                Ok(line) => parse_json_line(&line, &mut summary, sender),
                Err(error) => {
                    let _ = sender.send(CodexEvent::Progress(format!("stream error: {error}")));
                }
            }
        }
        if cancel.load(Ordering::Acquire) {
            terminate_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            let _ = sender.send(CodexEvent::Cancelled);
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            while let Ok(line) = line_receiver.recv_timeout(Duration::from_millis(10)) {
                if let Ok(line) = line {
                    parse_json_line(&line, &mut summary, sender);
                }
            }
            let stderr = stderr_thread.join().unwrap_or_default();
            if !status.success() {
                let detail = stderr.trim();
                let message = if detail.is_empty() {
                    format!("Codex exited with {status}")
                } else {
                    format!("Codex exited with {status}: {detail}")
                };
                let _ = sender.send(CodexEvent::Failed(message));
                return Ok(());
            }
            if let Some(command) = post_run {
                run_hook("Post-run", command, repository, sender)?;
            }
            if summary.is_empty() {
                "Codex completed the cue.".clone_into(&mut summary);
            }
            let _ = sender.send(CodexEvent::Completed { summary });
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn run_hook(
    label: &str,
    command: &[String],
    repository: &Path,
    sender: &mpsc::Sender<CodexEvent>,
) -> std::io::Result<()> {
    let Some((program, arguments)) = command.split_first() else {
        return Ok(());
    };
    let _ = sender.send(CodexEvent::Progress(format!("{label}: {program}")));
    let output = Command::new(program)
        .args(arguments)
        .current_dir(repository)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{label} command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn parse_json_line(line: &str, summary: &mut String, sender: &mpsc::Sender<CodexEvent>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        if !line.trim().is_empty() {
            let _ = sender.send(CodexEvent::Progress(line.to_owned()));
        }
        return;
    };
    let event_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("event");
    let text = find_text(&value).unwrap_or_else(|| event_type.to_owned());
    let is_agent_message = event_type.contains("message")
        || event_type.contains("agent")
        || value
            .pointer("/item/type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|item_type| item_type == "agent_message");
    if is_agent_message {
        summary.clone_from(&text);
    }
    let _ = sender.send(CodexEvent::Progress(text));
}

fn find_text(value: &serde_json::Value) -> Option<String> {
    for key in ["text", "message", "summary", "content"] {
        if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
            return Some(text.to_owned());
        }
    }
    for key in ["item", "msg", "part", "result"] {
        if let Some(nested) = value.get(key)
            && let Some(text) = find_text(nested)
        {
            return Some(text);
        }
    }
    None
}

fn parse_arguments(arguments: &str) -> Result<Vec<String>, CodexError> {
    let parsed = shlex::split(arguments)
        .ok_or_else(|| CodexError::InvalidArguments("unmatched quote".to_owned()))?;
    let protected = ["exec", "resume", "review", "--json", "-C", "--cd"];
    if let Some(argument) = parsed
        .iter()
        .find(|argument| protected.contains(&argument.as_str()))
    {
        return Err(CodexError::InvalidArguments(format!(
            "`{argument}` is controlled by Codex Dirigent"
        )));
    }
    Ok(parsed)
}

fn parse_command(command: &str) -> Result<Option<Vec<String>>, CodexError> {
    if command.trim().is_empty() {
        return Ok(None);
    }
    let parsed = shlex::split(command)
        .ok_or_else(|| CodexError::InvalidArguments("unmatched quote in command".to_owned()))?;
    if parsed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parsed))
    }
}

fn validate_environment_names(names: &[String]) -> Result<(), CodexError> {
    for name in names {
        let valid = !name.is_empty()
            && name.bytes().enumerate().all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
            });
        if !valid {
            return Err(CodexError::InvalidEnvironmentName(name.clone()));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-TERM", &format!("-{pid}")])
        .status();
}

#[cfg(not(unix))]
fn terminate_process_group(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(temp: &tempfile::TempDir, body: &str) -> PathBuf {
        let path = temp.path().join("fake-codex");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = path.metadata().unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[test]
    fn streams_json_and_completes() {
        let temp = tempfile::tempdir().unwrap();
        let cli = executable(
            &temp,
            "cat >/dev/null\nprintf '%s\\n' '{\"type\":\"agent_message\",\"text\":\"implemented safely\"}'",
        );
        let config = CodexConfig {
            cli_path: cli,
            ..CodexConfig::default()
        };
        let run = start(temp.path(), "prompt".to_owned(), config).unwrap();
        let mut events = Vec::new();
        loop {
            let event = run.receiver.recv_timeout(Duration::from_secs(2)).unwrap();
            let completed = matches!(event, CodexEvent::Completed { .. });
            events.push(event);
            if completed {
                break;
            }
        }
        assert!(events.contains(&CodexEvent::Progress("implemented safely".to_owned())));
        assert!(events.contains(&CodexEvent::Completed {
            summary: "implemented safely".to_owned()
        }));
    }

    #[test]
    fn cancellation_stops_run() {
        let temp = tempfile::tempdir().unwrap();
        let cli = executable(&temp, "cat >/dev/null\nsleep 10");
        let run = start(
            temp.path(),
            "prompt".to_owned(),
            CodexConfig {
                cli_path: cli,
                ..CodexConfig::default()
            },
        )
        .unwrap();
        run.cancel();
        assert_eq!(
            run.receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            CodexEvent::Cancelled
        );
    }

    #[test]
    fn validates_protected_args_and_environment_names() {
        assert!(parse_arguments("--json").is_err());
        assert!(parse_arguments("--profile careful --ephemeral").is_ok());
        assert!(validate_environment_names(&["SAFE_NAME".to_owned()]).is_ok());
        assert!(validate_environment_names(&["BAD-NAME".to_owned()]).is_err());
    }

    #[test]
    fn refinement_prompt_contains_conversation() {
        let cue = Cue::new("first task", crate::cue::CueTarget::Repository).unwrap();
        let messages = vec![
            Message {
                speaker: Speaker::Codex,
                text: "first result".to_owned(),
            },
            Message {
                speaker: Speaker::User,
                text: "please refine".to_owned(),
            },
        ];
        let prompt = follow_up_prompt(&cue, &messages);
        assert!(prompt.contains("Codex: first result"));
        assert!(prompt.contains("User: please refine"));
    }
}
