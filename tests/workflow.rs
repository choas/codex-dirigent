#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use codex_dirigent::codex::{self, CodexConfig, CodexEvent};
use codex_dirigent::cue::{Cue, CueTarget};
use codex_dirigent::review::{Session, SessionState};
use codex_dirigent::workspace::Workspace;

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .env("LC_ALL", "C")
        .output()
        .expect("Git should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fake_codex(root: &Path, replacement: u8) -> PathBuf {
    let path = root.join("fake-codex");
    let script = format!(
        "#!/bin/sh\ncat >/dev/null\nprintf 'pub fn value() -> u8 {{ {replacement} }}\\n' > src/lib.rs\nprintf '%s\\n' '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"updated value to {replacement}\"}}}}'\n"
    );
    fs::write(&path, script).unwrap();
    let mut permissions = path.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn await_completion(run: &codex::CodexRun) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "fake Codex run timed out");
        match run.try_recv() {
            Ok(CodexEvent::Completed { summary }) => return summary,
            Ok(CodexEvent::Failed(error)) => panic!("fake Codex failed: {error}"),
            Ok(CodexEvent::Cancelled) => panic!("fake Codex was cancelled"),
            Ok(CodexEvent::Progress(_)) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("fake Codex disconnected")
            }
        }
    }
}

#[test]
fn complete_reviewed_codex_workflow() {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), &["init", "-q"]);
    git(
        repository.path(),
        &["config", "user.name", "Codex Dirigent"],
    );
    git(
        repository.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    fs::create_dir(repository.path().join("src")).unwrap();
    fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn value() -> u8 { 1 }\n",
    )
    .unwrap();
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "-qm", "initial"]);

    let mut workspace = Workspace::open(repository.path()).unwrap();
    assert!(workspace.is_clean().unwrap());
    assert!(
        workspace
            .read_text(Path::new("src/lib.rs"))
            .unwrap()
            .contains("value")
    );

    let cue = Cue::new(
        "update the returned value",
        CueTarget::File(PathBuf::from("src/lib.rs")),
    )
    .unwrap();
    let mut session = Session::new(cue);
    let first_run_id = session.begin_run().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let cli_path = fake_codex(tools.path(), 2);
    let config = CodexConfig {
        cli_path: cli_path.clone(),
        ..CodexConfig::default()
    };
    let first_run =
        codex::start(repository.path(), session.cue().prompt(), config.clone()).unwrap();
    let first_summary = await_completion(&first_run);
    workspace.refresh().unwrap();
    let first_diff = workspace.working_diff().unwrap();
    session
        .finish_run(first_run_id, first_summary, first_diff)
        .unwrap();

    let second_run_id = session
        .follow_up("use 3 and keep the function signature")
        .unwrap();
    let _ = fake_codex(tools.path(), 3);
    let prompt = codex::follow_up_prompt(session.cue(), session.messages());
    let second_run = codex::start(repository.path(), prompt, config).unwrap();
    let second_summary = await_completion(&second_run);
    workspace.refresh().unwrap();
    let reviewed_diff = workspace.working_diff().unwrap();
    session
        .finish_run(second_run_id, second_summary, &reviewed_diff)
        .unwrap();
    session.accept(&reviewed_diff).unwrap();

    let approval = session.approval().unwrap().clone();
    let commit = workspace
        .commit_approved(&approval, "Update reviewed value")
        .unwrap();
    session.mark_committed(&commit).unwrap();
    assert!(matches!(session.state(), SessionState::Committed { .. }));
    assert!(workspace.is_clean().unwrap());
    assert!(
        workspace
            .read_text(Path::new("src/lib.rs"))
            .unwrap()
            .contains("{ 3 }")
    );
}
