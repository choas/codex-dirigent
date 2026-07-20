//! Safe, local-only Git repository browsing.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::review::ReviewApproval;

const MAX_FILES: usize = 20_000;
const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("the selected folder is not a Git worktree")]
    NotGitRepository,
    #[error(
        "repository is no longer available at `{path}`; reopen its current location",
        path = .0.display()
    )]
    RepositoryUnavailable(PathBuf),
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
    #[error("the primary worktree must be on the main branch before merging cues")]
    NotMainBranch,
    #[error("the cue cannot merge cleanly into main: {0}")]
    MergeConflict(String),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueWorktree {
    path: PathBuf,
    branch: String,
    base_commit: String,
}

impl CueWorktree {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    #[must_use]
    pub fn base_commit(&self) -> &str {
        &self.base_commit
    }

    /// Open this cue's isolated Git worktree.
    ///
    /// # Errors
    ///
    /// Returns an error when the worktree no longer exists or Git cannot read it.
    pub fn open(&self) -> Result<Workspace> {
        Workspace::open(&self.path)
    }
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

    /// Return the commit currently checked out by this worktree.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot resolve `HEAD`.
    pub fn head_commit(&self) -> Result<String> {
        Ok(git_text(&self.root, ["rev-parse", "HEAD"])?
            .trim()
            .to_owned())
    }

    /// Report whether `commit` is contained in the current `HEAD` history.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot perform the ancestry check.
    pub fn contains_commit(&self, commit: &str) -> Result<bool> {
        let output = Command::new("git")
            .args(["merge-base", "--is-ancestor", commit, "HEAD"])
            .current_dir(&self.root)
            .env("LC_ALL", "C")
            .output()?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(git_error(&output)),
        }
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

    /// Create an isolated branch and worktree for one cue at the current HEAD.
    ///
    /// # Errors
    ///
    /// Returns an error when the primary worktree is dirty, has no commit, or
    /// Git cannot create the branch and linked worktree.
    pub fn create_cue_worktree(&self, cue_id: u64) -> Result<CueWorktree> {
        if self.branch != "main" {
            return Err(WorkspaceError::NotMainBranch);
        }
        if !self.is_clean()? {
            return Err(WorkspaceError::Git(
                "commit or stash primary-worktree changes before creating a cue worktree"
                    .to_owned(),
            ));
        }
        let base_commit = git_text(&self.root, ["rev-parse", "HEAD"])?
            .trim()
            .to_owned();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| WorkspaceError::Git(error.to_string()))?
            .as_nanos();
        let repository_key = blake3::hash(self.root.as_os_str().as_encoded_bytes()).to_hex();
        let branch = format!("codex-dirigent/cue-{cue_id}-{}-{nonce}", std::process::id());
        let path = std::env::temp_dir()
            .join("codex-dirigent-worktrees")
            .join(&repository_key[..12])
            .join(format!("cue-{cue_id}-{nonce}"));
        let parent = path.parent().ok_or(WorkspaceError::OutsideRepository)?;
        fs::create_dir_all(parent)?;
        let output = Command::new("git")
            .args(["worktree", "add", "-b", &branch])
            .arg(&path)
            .arg(&base_commit)
            .current_dir(&self.root)
            .env("LC_ALL", "C")
            .output()?;
        if !output.status.success() {
            return Err(git_error(&output));
        }
        Ok(CueWorktree {
            path,
            branch,
            base_commit,
        })
    }

    /// Find linked worktrees previously created for unfinished cues.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot enumerate linked worktrees.
    pub fn linked_cue_worktrees(&self) -> Result<Vec<CueWorktree>> {
        let output = git_output(&self.root, ["worktree", "list", "--porcelain", "-z"])?;
        if !output.status.success() {
            return Err(git_error(&output));
        }
        Ok(parse_linked_cue_worktrees(&output.stdout))
    }

    /// Preflight and merge a committed cue branch into the clean main branch.
    ///
    /// # Errors
    ///
    /// Returns an error without changing main when the primary branch is not
    /// `main`, either worktree is dirty, or Git predicts a conflict. If the
    /// actual merge unexpectedly fails, it is aborted before returning.
    pub fn merge_cue(&mut self, cue: &CueWorktree) -> Result<String> {
        self.refresh()?;
        if self.branch != "main" {
            return Err(WorkspaceError::NotMainBranch);
        }
        if !self.is_clean()? {
            return Err(WorkspaceError::Git(
                "main worktree changed; commit or stash it before merging the cue".to_owned(),
            ));
        }
        if !cue.open()?.is_clean()? {
            return Err(WorkspaceError::Git(
                "cue worktree still has uncommitted changes".to_owned(),
            ));
        }
        let preflight = git_output(
            &self.root,
            ["merge-tree", "--write-tree", "HEAD", cue.branch()],
        )?;
        if !preflight.status.success() {
            let detail = String::from_utf8_lossy(&preflight.stdout);
            let stderr = String::from_utf8_lossy(&preflight.stderr);
            return Err(WorkspaceError::MergeConflict(
                format!("{detail}{stderr}").trim().to_owned(),
            ));
        }
        let merge = Command::new("git")
            .args(["merge", "--no-ff", "--no-edit", cue.branch()])
            .current_dir(&self.root)
            .env("LC_ALL", "C")
            .output()?;
        if !merge.status.success() {
            let _abort = git_output(&self.root, ["merge", "--abort"]);
            return Err(git_error(&merge));
        }
        let hash = git_text(&self.root, ["rev-parse", "HEAD"])?
            .trim()
            .to_owned();
        self.refresh()?;
        Ok(hash)
    }

    /// Remove a cue worktree and its merged branch.
    ///
    /// # Errors
    ///
    /// Returns an error when Git cannot remove the worktree or branch. Call
    /// this only after the user explicitly archives a completed cue.
    pub fn archive_cue_worktree(&mut self, cue: &CueWorktree) -> Result<()> {
        let remove = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(cue.path())
            .current_dir(&self.root)
            .env("LC_ALL", "C")
            .output()?;
        if !remove.status.success() {
            return Err(git_error(&remove));
        }
        let branch = git_output(&self.root, ["branch", "-D", cue.branch()])?;
        if !branch.status.success() {
            return Err(git_error(&branch));
        }
        self.refresh()?;
        Ok(())
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
    if !root.is_dir() {
        return Err(WorkspaceError::RepositoryUnavailable(root.to_path_buf()));
    }
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

fn parse_linked_cue_worktrees(bytes: &[u8]) -> Vec<CueWorktree> {
    let mut worktrees = Vec::new();
    let mut fields = Vec::new();
    for field in bytes.split(|byte| *byte == 0) {
        if field.is_empty() {
            if !fields.is_empty() {
                if let Some(worktree) = cue_worktree_from_fields(&fields) {
                    worktrees.push(worktree);
                }
                fields.clear();
            }
        } else {
            fields.push(field);
        }
    }
    if !fields.is_empty()
        && let Some(worktree) = cue_worktree_from_fields(&fields)
    {
        worktrees.push(worktree);
    }
    worktrees
}

fn cue_worktree_from_fields(fields: &[&[u8]]) -> Option<CueWorktree> {
    let path = fields
        .iter()
        .find_map(|field| field.strip_prefix(b"worktree "))?;
    let head = fields
        .iter()
        .find_map(|field| field.strip_prefix(b"HEAD "))?;
    let branch = fields
        .iter()
        .find_map(|field| field.strip_prefix(b"branch refs/heads/"))?;
    let branch = String::from_utf8_lossy(branch).into_owned();
    if !branch.starts_with("codex-dirigent/cue-") {
        return None;
    }
    let path = PathBuf::from(OsString::from(String::from_utf8_lossy(path).as_ref()));
    if !path.is_dir() {
        return None;
    }
    Some(CueWorktree {
        path,
        branch,
        base_commit: String::from_utf8_lossy(head).into_owned(),
    })
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
        run(temp.path(), &["init", "-q", "-b", "main"]);
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
    fn reports_when_an_open_repository_was_moved_or_removed() {
        let temp = repository();
        let workspace = Workspace::open(temp.path()).unwrap();
        let former_path = workspace.root().to_path_buf();
        temp.close().unwrap();

        let error = workspace.create_cue_worktree(1).unwrap_err();
        assert!(matches!(
            error,
            WorkspaceError::RepositoryUnavailable(ref path) if path == &former_path
        ));
        assert!(error.to_string().contains("reopen its current location"));
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

    fn approve_and_commit(worktree: &mut Workspace, message: &str) {
        use crate::cue::{Cue, CueTarget};
        use crate::review::Session;

        let diff = worktree.working_diff().unwrap();
        let mut session = Session::new(Cue::new(message, CueTarget::Repository).unwrap());
        let run = session.begin_run().unwrap();
        session.finish_run(run, "done", &diff).unwrap();
        session.accept(&diff).unwrap();
        worktree
            .commit_approved(session.approval().unwrap(), message)
            .unwrap();
    }

    #[test]
    fn independent_cue_worktrees_merge_into_main() {
        let temp = repository();
        fs::write(temp.path().join("other.txt"), "one\n").unwrap();
        run(temp.path(), &["add", "."]);
        run(temp.path(), &["commit", "-qm", "add other"]);
        let mut main = Workspace::open(temp.path()).unwrap();
        let first = main.create_cue_worktree(1).unwrap();
        let second = main.create_cue_worktree(2).unwrap();

        let mut first_workspace = first.open().unwrap();
        fs::write(
            first.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 2 }\n",
        )
        .unwrap();
        first_workspace.refresh().unwrap();
        approve_and_commit(&mut first_workspace, "Change value");

        let mut second_workspace = second.open().unwrap();
        fs::write(second.path().join("other.txt"), "two\n").unwrap();
        second_workspace.refresh().unwrap();
        approve_and_commit(&mut second_workspace, "Change other");

        main.merge_cue(&first).unwrap();
        main.merge_cue(&second).unwrap();
        assert!(
            main.read_text(Path::new("src/lib.rs"))
                .unwrap()
                .contains("{ 2 }")
        );
        assert_eq!(main.read_text(Path::new("other.txt")).unwrap(), "two\n");
        main.archive_cue_worktree(&first).unwrap();
        main.archive_cue_worktree(&second).unwrap();
        assert!(!first.path().exists());
        assert!(!second.path().exists());
    }

    #[test]
    fn lists_linked_cue_worktrees_for_recovery() {
        let temp = repository();
        let mut main = Workspace::open(temp.path()).unwrap();
        let cue = main.create_cue_worktree(42).unwrap();

        let linked = main.linked_cue_worktrees().unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].branch(), cue.branch());
        assert_eq!(
            linked[0].path().canonicalize().unwrap(),
            cue.path().canonicalize().unwrap()
        );

        main.archive_cue_worktree(&cue).unwrap();
    }

    #[test]
    fn conflicting_cue_is_rejected_before_main_changes() {
        let temp = repository();
        let mut main = Workspace::open(temp.path()).unwrap();
        let first = main.create_cue_worktree(10).unwrap();
        let second = main.create_cue_worktree(11).unwrap();
        let mut first_workspace = first.open().unwrap();
        fs::write(first.path().join("src/lib.rs"), "first\n").unwrap();
        first_workspace.refresh().unwrap();
        approve_and_commit(&mut first_workspace, "First change");
        let mut second_workspace = second.open().unwrap();
        fs::write(second.path().join("src/lib.rs"), "second\n").unwrap();
        second_workspace.refresh().unwrap();
        approve_and_commit(&mut second_workspace, "Second change");

        main.merge_cue(&first).unwrap();
        let head_before = git_text(temp.path(), ["rev-parse", "HEAD"]).unwrap();
        assert!(matches!(
            main.merge_cue(&second),
            Err(WorkspaceError::MergeConflict(_))
        ));
        assert_eq!(
            git_text(temp.path(), ["rev-parse", "HEAD"]).unwrap(),
            head_before
        );
        assert!(main.is_clean().unwrap());
        main.archive_cue_worktree(&first).unwrap();
        let remove = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(second.path())
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(remove.status.success());
    }
}
