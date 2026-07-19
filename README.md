# Codex Dirigent

Codex Dirigent is a focused, native macOS application for directing Codex CLI
changes through an explicit review gate. Open a local Git repository, browse
its code read-only, and create as many repository-, file-, or line-range cues
as needed. Each cue runs concurrently in its own Git worktree and moves through
Run, Review, Done, and Archive lanes. A cue can be committed and merged into
`main` only after accepting its exact current diff.

The interface and application core are written in Rust with `eframe`/`egui`.
Codex CLI is the sole agent process and Git is the sole version-control
workflow.

## Requirements

- macOS 14 or later on Apple Silicon or Intel (the initial supported platform)
- Git available on `PATH`
- Rust 1.92 or newer and Xcode Command Line Tools when building from source
- An installed and authenticated Codex CLI. See the official
  [Codex CLI documentation](https://developers.openai.com/codex/cli), then
  confirm it is available with `codex --version`.

Codex Dirigent launches `codex exec --json` with the repository as its working
root and the `workspace-write` sandbox. Authentication stays with Codex CLI;
the app does not store account credentials or API keys.

## Build and run

```sh
cargo run
```

Run the complete local verification suite:

```sh
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test
```

## Workflow

1. Choose **Open Repository…** and select any folder inside a local Git
   worktree.
2. Select a file in the left sidebar to inspect its line-numbered, read-only
   contents.
3. Open **New Cue**, choose repository, file, or line-range scope, and add the
   task. Codex Dirigent creates a dedicated branch and linked worktree from the
   current `main` commit. Repeat without a fixed cue limit.
4. In the **Cue Board** Run lane, start any cues you want. Their Codex processes
   can run concurrently because they write to different worktrees.
5. In Review, open a cue, inspect its complete tracked and untracked diff, send
   follow-up instructions, or explicitly accept/reject it.
6. After acceptance, enter a commit message and choose **Commit & Merge to
   Main**. The cue is committed in its worktree, checked with `git merge-tree`,
   and merged into a clean `main`. A predicted conflict leaves `main` untouched;
   an unexpected merge failure is aborted. Successfully merged cues move to
   Done and can then be archived to remove their worktree and branch.

Opening a different repository is disabled while Codex is running or active
cues remain. A stale worker result cannot replace a newer run. Rejection has a
confirmation dialog, removes only that cue's isolated worktree and branch, and
never modifies `main`.

Keyboard shortcuts follow macOS conventions: `⌘O` opens a repository, `⌘R`
refreshes it, `⌘,` opens settings, and `Esc` closes settings or a rejection
confirmation.

## Settings

The compact settings sheet contains only execution inputs used by Codex:

- CLI path and optional model
- extra `codex exec` arguments
- environment variable names, one per line
- one direct pre-run command and one direct post-run command

Environment values are resolved only when a run starts and are never written
to settings. Hook commands are parsed into an executable and argument vector;
they are not evaluated by a shell. Arguments that would replace Dirigent's
managed execution mode, working directory, or JSON stream are rejected.

Settings are atomically saved at
`~/Library/Application Support/Codex Dirigent/settings.json`. Unknown old keys
are ignored, and an invalid file produces a visible warning while the app starts
with safe defaults.

## Create a macOS app bundle

```sh
./scripts/bundle-macos.sh
open "dist/Codex Dirigent.app"
```

The script makes a release build, generates the icon from the original SVG,
assembles `dist/Codex Dirigent.app`, and applies an ad-hoc local signature.
For distribution, replace that signature with a Developer ID signature and
notarize the bundle using your Apple developer credentials. Generated `dist`
and `target` content is not committed.

## Architecture and safety

- `workspace`: contained local file browsing plus Git status, diff, isolated
  worktree creation, review-authorized commits, merge preflight, and safe merge
- `cue`: validated repository, file, and 1-based line-range targets
- `review`: run/follow-up/review state machine and exact-diff approval token
- `codex`: direct CLI process lifecycle, JSON progress, hooks, and cancellation
- `settings`: minimal Serde model with atomic persistence
- `app`: native workflow UI and macOS interactions

The clean-start design audit and retained/excluded scope are recorded in
[`docs/reference-audit.md`](docs/reference-audit.md).

## Current limitations

- Initial release is macOS-only and has no signed/notarized binary release.
- The file tree is intentionally flat and capped at 20,000 files; the viewer
  accepts UTF-8 text up to 2 MiB.
- The opened primary worktree must be clean and on a branch named `main` when
  creating or merging cues.
- Merge conflicts cannot be silently eliminated. They are detected before main
  is changed and shown on the Review card so the cue branch remains recoverable.
- Cue-board state is currently session-local. Linked worktrees and branches
  remain recoverable in Git if the app exits before they are archived.
- Visual smoke testing requires an interactive macOS session with screen
  recording permission; automated checks cover the domain and subprocess paths.

## License

MIT. See [`LICENSE`](LICENSE).
