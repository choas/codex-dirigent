//! Versioned, atomic persistence for user-authored cue-board state.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cue::{Cue, CueError, CueTarget};

pub const FILE_NAME: &str = "cue-board.json";
const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PersistedLane {
    Inbox,
    Run,
    Review,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PersistedTarget {
    Repository,
    File {
        path: PathBuf,
    },
    Lines {
        path: PathBuf,
        start: usize,
        end: usize,
    },
}

impl From<&CueTarget> for PersistedTarget {
    fn from(target: &CueTarget) -> Self {
        match target {
            CueTarget::Repository => Self::Repository,
            CueTarget::File(path) => Self::File { path: path.clone() },
            CueTarget::Lines { path, start, end } => Self::Lines {
                path: path.clone(),
                start: *start,
                end: *end,
            },
        }
    }
}

impl From<PersistedTarget> for CueTarget {
    fn from(target: PersistedTarget) -> Self {
        match target {
            PersistedTarget::Repository => Self::Repository,
            PersistedTarget::File { path } => Self::File(path),
            PersistedTarget::Lines { path, start, end } => Self::Lines { path, start, end },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedCue {
    pub(crate) id: u64,
    pub(crate) lane: PersistedLane,
    instruction: String,
    target: PersistedTarget,
    #[serde(default)]
    pub(crate) follow_ups: Vec<String>,
    #[serde(default)]
    pub(crate) worktree_branch: Option<String>,
    #[serde(default)]
    pub(crate) branch_commit: Option<String>,
    #[serde(default)]
    pub(crate) merged_commit: Option<String>,
    #[serde(default)]
    pub(crate) commit_message: String,
}

impl PersistedCue {
    pub(crate) fn new(id: u64, lane: PersistedLane, cue: &Cue) -> Self {
        Self {
            id,
            lane,
            instruction: cue.instruction().to_owned(),
            target: PersistedTarget::from(cue.target()),
            follow_ups: Vec::new(),
            worktree_branch: None,
            branch_commit: None,
            merged_commit: None,
            commit_message: String::new(),
        }
    }

    pub(crate) fn cue(&self) -> Result<Cue, CueError> {
        Cue::new(self.instruction.clone(), self.target.clone().into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BoardState {
    version: u32,
    pub(crate) repository: Option<PathBuf>,
    pub(crate) next_cue_id: u64,
    pub(crate) cues: Vec<PersistedCue>,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            repository: None,
            next_cue_id: 1,
            cues: Vec::new(),
        }
    }
}

impl BoardState {
    pub(crate) fn for_repository(repository: PathBuf, next_cue_id: u64) -> Self {
        Self {
            repository: Some(repository),
            next_cue_id: next_cue_id.max(1),
            ..Self::default()
        }
    }

    fn validate(&self) -> Result<(), BoardError> {
        if self.version != CURRENT_VERSION {
            return Err(BoardError::UnsupportedVersion(self.version));
        }
        for cue in &self.cues {
            cue.cue()?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    #[error("cue-board state uses unsupported schema version {0}")]
    UnsupportedVersion(u32),
    #[error("cue-board state contains an invalid cue: {0}")]
    InvalidCue(#[from] CueError),
    #[error("cue-board state is invalid: {0}")]
    Invalid(#[from] serde_json::Error),
    #[error("cue-board state could not be read or saved: {0}")]
    Io(#[from] std::io::Error),
    #[error("atomic cue-board replacement failed: {0}")]
    Persist(#[from] tempfile::PersistError),
}

/// Load a board, returning an empty board when no state has been saved yet.
///
/// Unknown object fields are ignored by Serde. Unsupported versions, invalid
/// cue data, and malformed JSON are reported to the caller.
///
/// # Errors
///
/// Returns an error when the file cannot be read or validated.
pub(crate) fn load(path: &Path) -> Result<BoardState, BoardError> {
    match fs::read(path) {
        Ok(bytes) => {
            let state: BoardState = serde_json::from_slice(&bytes)?;
            state.validate()?;
            Ok(state)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BoardState::default()),
        Err(error) => Err(BoardError::Io(error)),
    }
}

/// Load persisted state without allowing corruption to block application startup.
pub(crate) fn load_or_empty(path: &Path) -> (BoardState, Option<String>) {
    match load(path) {
        Ok(state) => (state, None),
        Err(error) => (
            BoardState::default(),
            Some(format!("{error}; continuing with an empty cue board")),
        ),
    }
}

/// Atomically save board state beside its final file.
///
/// # Errors
///
/// Returns an error when directory creation, serialization, syncing, or the
/// final atomic replacement fails.
pub(crate) fn save(path: &Path, state: &BoardState) -> Result<(), BoardError> {
    state.validate()?;
    let parent = path.parent().ok_or_else(|| {
        BoardError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "cue-board path has no parent directory",
        ))
    })?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, state)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_state_round_trips_all_user_authored_cue_data() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested/cue-board.json");
        let cue = Cue::new(
            "Fix the boundary",
            CueTarget::Lines {
                path: PathBuf::from("src/lib.rs"),
                start: 10,
                end: 14,
            },
        )
        .unwrap();
        let mut persisted = PersistedCue::new(7, PersistedLane::Review, &cue);
        persisted.follow_ups = vec!["Add a regression test".to_owned()];
        persisted.worktree_branch = Some("codex-dirigent/cue-7-example".to_owned());
        persisted.commit_message = "Fix boundary".to_owned();
        let mut state = BoardState::for_repository(PathBuf::from("/tmp/project"), 8);
        state.cues.push(persisted);

        save(&path, &state).unwrap();

        assert_eq!(load(&path).unwrap(), state);
    }

    #[test]
    fn obsolete_and_unknown_future_fields_are_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        fs::write(
            &path,
            r#"{
                "version": 1,
                "repository": "/tmp/project",
                "next_cue_id": 3,
                "retired_window_layout": "wide",
                "cues": [{
                    "id": 2,
                    "lane": "inbox",
                    "instruction": "Keep this cue",
                    "target": {"kind": "repository", "future_scope_hint": true},
                    "follow_ups": [],
                    "future_card_color": "green"
                }]
            }"#,
        )
        .unwrap();

        let state = load(&path).unwrap();

        assert_eq!(state.cues.len(), 1);
        assert_eq!(state.cues[0].cue().unwrap().instruction(), "Keep this cue");
    }

    #[test]
    fn corrupt_state_becomes_a_warning_and_an_empty_board() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        fs::write(&path, "{not json").unwrap();

        let (state, warning) = load_or_empty(&path);

        assert!(state.cues.is_empty());
        assert!(warning.unwrap().contains("empty cue board"));
    }
}
