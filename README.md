# Codex Dirigent

Codex Dirigent is a focused, native macOS application for directing Codex CLI
changes through an explicit review gate. Open a local Git repository, browse
its code read-only, attach an instruction to the repository, a file, or a line
range, run Codex, refine the result, review the diff, then accept or reject it.
A commit is possible only after accepting the exact current diff.

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
3. Open **Cue**, choose repository, file, or line-range scope, enter the task,
   and create the cue.
4. In **Run**, start Codex. The initial run requires a clean worktree so that
   rejection has a precise, safe baseline. Progress is streamed into the app;
   the run can be cancelled.
5. In **Review**, inspect the complete tracked and untracked diff. Enter a
   follow-up instruction to refine it, or explicitly accept/reject it.
6. After acceptance, enter a commit message. Codex Dirigent rechecks that the
   working diff still matches the accepted review before staging and committing.

Opening a different repository is disabled while Codex is running. A stale
worker result cannot replace a newer run. Rejection has a confirmation dialog
and restores only a run that began from the enforced clean baseline.

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

- `workspace`: contained local file browsing plus Git status, diff, restore,
  and review-authorized commit operations
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
- An initial cue cannot run on a dirty worktree. Commit or stash existing work
  first; refinement runs operate on the current result under review.
- Visual smoke testing requires an interactive macOS session with screen
  recording permission; automated checks cover the domain and subprocess paths.

## License

MIT. See [`LICENSE`](LICENSE).

