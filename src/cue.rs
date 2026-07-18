//! User instructions anchored to a repository, file, or line range.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CueTarget {
    Repository,
    File(PathBuf),
    Lines {
        path: PathBuf,
        start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    instruction: String,
    target: CueTarget,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CueError {
    #[error("enter an instruction for Codex")]
    EmptyInstruction,
    #[error("cue file paths must be relative paths inside the repository")]
    InvalidPath,
    #[error("line ranges are 1-based and the end must not precede the start")]
    InvalidLineRange,
}

impl Cue {
    /// Create and validate a cue.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty instruction, unsafe file path, or invalid
    /// 1-based line range.
    pub fn new(instruction: impl Into<String>, target: CueTarget) -> Result<Self, CueError> {
        let instruction = instruction.into().trim().to_owned();
        if instruction.is_empty() {
            return Err(CueError::EmptyInstruction);
        }
        match &target {
            CueTarget::Repository => {}
            CueTarget::File(path) => validate_path(path)?,
            CueTarget::Lines { path, start, end } => {
                validate_path(path)?;
                if *start == 0 || end < start {
                    return Err(CueError::InvalidLineRange);
                }
            }
        }
        Ok(Self {
            instruction,
            target,
        })
    }

    #[must_use]
    pub fn instruction(&self) -> &str {
        &self.instruction
    }

    #[must_use]
    pub const fn target(&self) -> &CueTarget {
        &self.target
    }

    #[must_use]
    pub fn prompt(&self) -> String {
        match &self.target {
            CueTarget::Repository => format!(
                "Repository-level task.\n\nInstruction:\n{}",
                self.instruction
            ),
            CueTarget::File(path) => format!(
                "File task for `{}`. Inspect the repository as needed, but keep the requested scope in mind.\n\nInstruction:\n{}",
                path.display(),
                self.instruction
            ),
            CueTarget::Lines { path, start, end } => format!(
                "Line-range task for `{}:{start}-{end}`. Inspect surrounding code and the repository as needed.\n\nInstruction:\n{}",
                path.display(),
                self.instruction
            ),
        }
    }
}

fn validate_path(path: &Path) -> Result<(), CueError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(CueError::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_all_three_cue_scopes() {
        let repository = Cue::new("run tests", CueTarget::Repository).unwrap();
        let file = Cue::new("simplify", CueTarget::File(PathBuf::from("src/lib.rs"))).unwrap();
        let lines = Cue::new(
            "fix boundary",
            CueTarget::Lines {
                path: PathBuf::from("src/lib.rs"),
                start: 10,
                end: 14,
            },
        )
        .unwrap();
        assert!(repository.prompt().contains("Repository-level"));
        assert!(file.prompt().contains("src/lib.rs"));
        assert!(lines.prompt().contains("10-14"));
    }

    #[test]
    fn rejects_empty_unsafe_and_inverted_cues() {
        assert_eq!(
            Cue::new(" ", CueTarget::Repository),
            Err(CueError::EmptyInstruction)
        );
        assert_eq!(
            Cue::new("x", CueTarget::File(PathBuf::from("../secret"))),
            Err(CueError::InvalidPath)
        );
        assert_eq!(
            Cue::new(
                "x",
                CueTarget::Lines {
                    path: PathBuf::from("safe.rs"),
                    start: 9,
                    end: 3,
                }
            ),
            Err(CueError::InvalidLineRange)
        );
    }
}
