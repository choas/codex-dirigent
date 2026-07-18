//! Safe, local-only Git repository browsing.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::review::ReviewApproval;

const MAX_FILES: usize = 20_000;
const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("the selected folder is not a Git worktree")]
    NotGitRepository,
    #[error("path is outside the opened repository")]
    OutsideRepository,
    #[error("file is larger than the 2 MiB viewer limit")]
    FileTooLarge,
    #[error("binary files cannot be displayed")]
    BinaryFile,
    #[error("Git command failed: {0}")]
    Git(String),
    #[error("the working tree no longer matches the accepted review")]
    ReviewInvalidated,
    #[error("enter a commit message")]
    EmptyCommitMessage,
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, WorkspaceError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub relative_path: PathBuf,
    pub status: Option<char>,
}

#[derive(Debug)]
pub struct Workspace {
    root: PathBuf,
    files: Vec<FileEntry>,
    branch: String,
}

impl Workspace {
    /// Open the Git worktree containing `path` and scan its local files.
    ///
    /// # Errors
    ///
    /// Returns an error when the path cannot be resolved, is not inside a Git
    /// worktree, or its files and status cannot be read.
    pub fn open(path: &Path) -> Result<Self> {
        let selected = path.canonicalize()?;
        let output = git_output(&selected, ["rev-parse", "--show-toplevel"])?;
        if !output.status.success() {
            return Err(WorkspaceError::NotGitRepository);
        }
        let root_text = String::from_utf8_lossy(&output.stdout);
        let root = PathBuf::from(root_text.trim()).canonicalize()?;
        let mut workspace = Self {
            root,
            files: Vec::new(),
            branch: String::new(),
        };
        workspace.refresh()?;
        Ok(workspace)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Refresh the file list, status markers, and branch name.
    ///
    /// # Errors
    ///
    /// Returns an error when the worktree cannot be read or Git status fails.
    pub fn refresh(&mut self) -> Result<()> {
        let statuses = self.statuses()?;
        let mut paths = Vec::new();
        collect_files(&self.root, &self.root, &mut paths)?;
        paths.sort();
        paths.truncate(MAX_FILES);
        self.files = paths
            .into_iter()
            .map(|relative_path| FileEntry {
                status: statuses.get(&relative_path).copied(),
                relative_path,
            })
            .collect();
        let branch = git_text(&self.root, ["branch", "--show-current"])?;
        self.branch = String::from(branch.trim());
        if self.branch.is_empty() {
            self.branch = String::from("detached HEAD");
        }
        Ok(())
    }

    /// Load a contained UTF-8 text file for read-only display.
    ///
    /// # Errors
    ///
    /// Returns an error for paths outside the worktree, binary/non-UTF-8 files,
    /// files above the viewer limit, or filesystem failures.
    pub fn read_text(&self, relative_path: &Path) -> Result<String> {
        let candidate = self.root.join(relative_path).canonicalize()?;
        if !candidate.starts_with(&self.root) || !candidate.is_file() {
            return Err(WorkspaceError::OutsideRepository);
        }
        if candidate.metadata()?.len() > MAX_TEXT_BYTES {
            return Err(WorkspaceError::FileTooLarge);
        }
        let bytes = fs::read(candidate)?;
        if bytes.contains(&0) {
            return Err(WorkspaceError::BinaryFile);
        }
        String::from_utf8(bytes).map_err(|_| WorkspaceError::BinaryFile)
    }

    /// Produce a unified diff for tracked changes and untracked local files.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot inspect the worktree.
    pub fn working_diff(&self) -> Result<String> {
        let tracked = git_output(
            &self.root,
            ["diff", "--no-ext-diff", "--binary", "HEAD", "--"],
        )?;
        let mut diff = if tracked.status.success() {
            String::from_utf8_lossy(&tracked.stdout).into_owned()
        } else {
            let fallback = git_output(&self.root, ["diff", "--no-ext-diff", "--binary", "--"])?;
            if !fallback.status.success() {
                return Err(git_error(&fallback));
            }
            String::from_utf8_lossy(&fallback.stdout).into_owned()
        };

        for path in self.untracked_paths()? {
            let output = Command::new("git")
                .args(["diff", "--no-index", "--", "/dev/null"])
                .arg(&path)
                .current_dir(&self.root)
                .env("LC_ALL", "C")
                .output()?;
            if matches!(output.status.code(), Some(0 | 1)) {
                diff.push_str(&String::from_utf8_lossy(&output.stdout));
            } else {
                return Err(git_error(&output));
            }
        }
        Ok(diff)
    }

    /// Report whether the worktree has no staged, unstaged, or untracked files.
    ///
    /// # Errors
    ///
    /// Returns an error when Git status cannot be read.
    pub fn is_clean(&self) -> Result<bool> {
        Ok(self.statuses()?.is_empty())
    }

    /// Commit the exact diff represented by an explicit review approval.
    ///
    /// # Errors
    ///
    /// Returns an error if the diff changed, the message is empty, or staging
    /// and committing through Git fails.
    pub fn commit_approved(&mut self, approval: &ReviewApproval, message: &str) -> Result<String> {
        if message.trim().is_empty() {
            return Err(WorkspaceError::EmptyCommitMessage);
        }
        if !approval.matches(&self.working_diff()?) {
            return Err(WorkspaceError::ReviewInvalidated);
        }
        let add = git_output(&self.root, ["add", "-A", "--"])?;
        if !add.status.success() {
            return Err(git_error(&add));
        }
        let commit = git_output(&self.root, ["commit", "-m", message.trim(), "--"])?;
        if !commit.status.success() {
            return Err(git_error(&commit));
        }
        let hash = git_text(&self.root, ["rev-parse", "HEAD"])?
            .trim()
            .to_owned();
        self.refresh()?;
        Ok(hash)
    }

    /// Restore all changes in a worktree that was clean before a Codex run.
    ///
    /// The caller is responsible for enforcing that clean-baseline invariant
    /// and obtaining explicit rejection confirmation from the user.
    ///
    /// # Errors
    ///
    /// Returns an error when Git restoration or removal of an untracked file
    /// fails. Paths are sourced from Git and checked beneath the worktree.
    pub fn reject_run_changes(&mut self) -> Result<()> {
        let statuses = self.statuses()?;
        let tracked: Vec<_> = statuses
            .iter()
            .filter(|(_, status)| **status != '?')
            .map(|(path, _)| path)
            .collect();
        if !tracked.is_empty() {
            let output = Command::new("git")
                .args(["restore", "--source=HEAD", "--staged", "--worktree", "--"])
                .args(tracked)
                .current_dir(&self.root)
                .env("LC_ALL", "C")
                .output()?;
            if !output.status.success() {
                return Err(git_error(&output));
            }
        }
        for path in statuses
            .iter()
            .filter(|(_, status)| **status == '?')
            .map(|(path, _)| path)
        {
            let candidate = self.root.join(path);
            if !candidate.starts_with(&self.root) {
                return Err(WorkspaceError::OutsideRepository);
            }
            if candidate.symlink_metadata()?.file_type().is_dir() {
                return Err(WorkspaceError::OutsideRepository);
            }
            fs::remove_file(candidate)?;
        }
        self.refresh()?;
        Ok(())
    }

    fn statuses(&self) -> Result<HashMap<PathBuf, char>> {
        let output = git_output(
            &self.root,
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        if !output.status.success() {
            return Err(git_error(&output));
        }
        Ok(parse_porcelain(&output.stdout))
    }

    fn untracked_paths(&self) -> Result<Vec<PathBuf>> {
        let output = git_output(
            &self.root,
            ["ls-files", "--others", "--exclude-standard", "-z"],
        )?;
        if !output.status.success() {
            return Err(git_error(&output));
        }
        Ok(output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(OsString::from(String::from_utf8_lossy(path).as_ref())))
            .filter(|path| self.root.join(path).is_file())
            .collect())
    }
}

fn collect_files(root: &Path, directory: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if output.len() >= MAX_FILES {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            if entry.file_name() != ".git" {
                collect_files(root, &path, output)?;
            }
        } else if file_type.is_file() {
            output.push(
                path.strip_prefix(root)
                    .map_err(|_| WorkspaceError::OutsideRepository)?
                    .to_path_buf(),
            );
        }
        if output.len() >= MAX_FILES {
            break;
        }
    }
    Ok(())
}

fn git_output<const N: usize>(root: &Path, args: [&str; N]) -> Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .env("LC_ALL", "C")
        .output()
        .map_err(WorkspaceError::Io)
}

fn git_text<const N: usize>(root: &Path, args: [&str; N]) -> Result<String> {
    let output = git_output(root, args)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(git_error(&output))
    }
}

fn git_error(output: &Output) -> WorkspaceError {
    WorkspaceError::Git(String::from_utf8_lossy(&output.stderr).trim().to_owned())
}

fn parse_porcelain(bytes: &[u8]) -> HashMap<PathBuf, char> {
    let mut statuses = HashMap::new();
    let mut records = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty());
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            continue;
        }
        let index = char::from(record[0]);
        let worktree = char::from(record[1]);
        let status = if worktree == ' ' { index } else { worktree };
        let path = PathBuf::from(String::from_utf8_lossy(&record[3..]).as_ref());
        if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            // With `-z`, the path in the status record is the destination and
            // the following NUL-delimited path is the source.
            let _source = records.next();
        }
        statuses.insert(path, status);
    }
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .env("LC_ALL", "C")
            .output()
            .expect("git must run in test");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir");
        run(temp.path(), &["init", "-q"]);
        run(temp.path(), &["config", "user.name", "Codex Dirigent"]);
        run(
            temp.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        fs::create_dir(temp.path().join("src")).expect("src directory");
        fs::write(
            temp.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 1 }\n",
        )
        .expect("fixture");
        run(temp.path(), &["add", "."]);
        run(temp.path(), &["commit", "-qm", "initial"]);
        temp
    }

    #[test]
    fn opens_and_browses_repository_without_git_metadata() {
        let temp = repository();
        let workspace = Workspace::open(temp.path()).expect("workspace");
        assert_eq!(workspace.files().len(), 1);
        assert_eq!(workspace.files()[0].relative_path, Path::new("src/lib.rs"));
        assert!(
            workspace
                .read_text(Path::new("src/lib.rs"))
                .unwrap()
                .contains("value")
        );
    }

    #[test]
    fn reports_modified_and_untracked_files_in_diff() {
        let temp = repository();
        fs::write(
            temp.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 2 }\n",
        )
        .unwrap();
        fs::write(temp.path().join("notes.txt"), "review me\n").unwrap();
        let workspace = Workspace::open(temp.path()).expect("workspace");
        let statuses: HashMap<_, _> = workspace
            .files()
            .iter()
            .map(|entry| (entry.relative_path.clone(), entry.status))
            .collect();
        assert_eq!(statuses[Path::new("src/lib.rs")], Some('M'));
        assert_eq!(statuses[Path::new("notes.txt")], Some('?'));
        let diff = workspace.working_diff().expect("diff");
        assert!(diff.contains("+pub fn value() -> u8 { 2 }"));
        assert!(diff.contains("+review me"));
    }

    #[test]
    fn rejects_non_repository_and_outside_paths() {
        let plain = tempfile::tempdir().unwrap();
        assert!(matches!(
            Workspace::open(plain.path()),
            Err(WorkspaceError::NotGitRepository)
        ));
        let temp = repository();
        let workspace = Workspace::open(temp.path()).unwrap();
        assert!(matches!(
            workspace.read_text(Path::new("../outside")),
            Err(WorkspaceError::Io(_) | WorkspaceError::OutsideRepository)
        ));
    }

    #[test]
    fn parses_rename_destination() {
        let parsed = parse_porcelain(b"R  new.rs\0old.rs\0?? note.txt\0");
        assert_eq!(parsed.get(Path::new("new.rs")), Some(&'R'));
        assert_eq!(parsed.get(Path::new("note.txt")), Some(&'?'));
    }

    #[test]
    fn accepted_diff_can_commit_and_changed_diff_cannot() {
        use crate::cue::{Cue, CueTarget};
        use crate::review::Session;

        let temp = repository();
        fs::write(
            temp.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 3 }\n",
        )
        .unwrap();
        let mut workspace = Workspace::open(temp.path()).unwrap();
        let diff = workspace.working_diff().unwrap();
        let mut session = Session::new(Cue::new("change value", CueTarget::Repository).unwrap());
        let run = session.begin_run().unwrap();
        session.finish_run(run, "done", &diff).unwrap();
        session.accept(&diff).unwrap();
        let commit = workspace
            .commit_approved(session.approval().unwrap(), "Change value")
            .unwrap();
        assert_eq!(commit.len(), 40);
        assert!(workspace.is_clean().unwrap());

        fs::write(
            temp.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 4 }\n",
        )
        .unwrap();
        assert!(matches!(
            workspace.commit_approved(session.approval().unwrap(), "Wrong diff"),
            Err(WorkspaceError::ReviewInvalidated)
        ));
    }

    #[test]
    fn rejection_restores_tracked_and_removes_untracked_changes() {
        let temp = repository();
        fs::write(temp.path().join("src/lib.rs"), "changed\n").unwrap();
        fs::write(temp.path().join("new.txt"), "new\n").unwrap();
        let mut workspace = Workspace::open(temp.path()).unwrap();
        workspace.reject_run_changes().unwrap();
        assert!(workspace.is_clean().unwrap());
        assert!(!temp.path().join("new.txt").exists());
        assert!(
            workspace
                .read_text(Path::new("src/lib.rs"))
                .unwrap()
                .contains("value")
        );
    }
}
