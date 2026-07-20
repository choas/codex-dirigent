# Verification record

Final verification date: 2026-07-20 on Apple Silicon macOS.

## Automated checks

| Check | Result |
| --- | --- |
| `cargo fmt --check` | passed |
| `cargo check --all-targets` | passed |
| `cargo clippy --all-targets -- -D warnings` | passed without warnings |
| `cargo test` | passed: 44 unit tests and 2 integration tests |
| `cargo test --test workflow -- --nocapture` | passed full and concurrent fake-Codex workflows |
| `cargo build --release --locked` from a fresh `git archive HEAD` | passed; arm64 Mach-O produced |
| `sh -n scripts/bundle-macos.sh` | passed |
| `sh -n scripts/package-release-macos.sh` | passed |
| `plutil -lint packaging/Info.plist` | passed |
| `scripts/bundle-macos.sh` | passed; `.app` and `.icns` produced |
| `scripts/package-release-macos.sh` with Developer ID | passed; universal ZIP and checksum produced |
| `lipo -archs` on release executable | passed: `x86_64 arm64` |
| `codesign --verify --deep --strict` on release app | passed Developer ID signature verification |
| `shasum -a 256 -c` and `unzip -t` on release ZIP | passed |
| `spctl --assess --type execute` | expected rejection: notarization remains pending |
| direct launch of debug and packaged executables | stayed running until smoke-test termination |

The first integration test creates a disposable Git repository, browses its
committed file, creates a file cue, executes a fake JSON-streaming Codex CLI,
collects the diff, sends a contextual follow-up through a second execution,
accepts the exact reviewed diff, commits through the approval gate, and verifies
the clean result. The second starts two fake Codex runs concurrently in separate
linked worktrees, reviews and commits both results, safely merges both branches
into `main`, and verifies their combined result. Focused tests separately cover
cancellation, rejection, stale runs, unsafe cue paths, dirty-tree status,
untracked diffs, expandable file-tree grouping, responsive foldable cue lanes,
newest-first card ordering, dedicated review navigation, linked-worktree restart
recovery, durable Inbox and user-conversation recovery, versioned board round
trips, unknown board fields, corrupt-board fallback, stale-state reconciliation,
lazy Inbox worktree creation, bulk Inbox-to-Run transitions, merge preflight and
conflict isolation, obsolete settings, missing/moved repository diagnostics, and
corrupt settings fallback.

## Removed-integration search

A case-insensitive tracked-file search was run for every required forbidden
term. There were only two matches: “Jujutsu” and “SSH” in the deliberate
exclusions section of `docs/reference-audit.md`. They document what is absent;
neither is a user option, dependency, executable path, or runtime identifier.
All other requested terms returned no matches.

`cargo tree` was separately searched for dependency names associated with the
removed integrations and returned no matches. Runtime process construction was
reviewed: fixed invocations are limited to Git and `/bin/kill`; Codex uses the
configured CLI path; explicitly configured hook commands execute directly
without a shell.

## Repository hygiene

`git ls-files` was searched for environment files, local databases and journals,
macOS metadata, reference application state directories, scanner/cache folders,
Git internals, `target`, and `dist`. No tracked matches were found. The generated
bundle and Rust build outputs are ignored. The target contains no imported
reference assets, bulk source tree, Git history, credentials, runtime state, or
database.

## Manual limitations

Both native executables launched successfully. Screen capture was unavailable
because the verification process did not have macOS screen-recording permission,
so pixel-level visual inspection remains a manual release check in an interactive
signed-in session. The local release artifact was built as a universal `arm64`
and `x86_64` application and passed strict verification with a timestamped
Developer ID signature. It is not notarized because no `notarytool` keychain
profile is configured; Gatekeeper therefore reports `Unnotarized Developer ID`.
Codex-generated summaries and progress are intentionally not persisted; after
restart, conversation recovery contains the original user-authored instruction
and follow-ups, and the review diff is regenerated from the linked worktree.
