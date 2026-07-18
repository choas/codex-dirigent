# Codex Dirigent reference audit

Audit date: 2026-07-18. Reference: the tracked source and documentation in
`~/prj/Dirigent`, treated as read-only. The new product is intentionally not a
fork.

## What the reference proves

Dirigent's useful spine is a read-only repository browser followed by a cue,
CLI execution with streamed events, filesystem-derived Git diff, iterative
feedback, explicit review, and a separate commit action. Its strongest
implementation lessons are bounded file/diff rendering, worker-to-UI message
passing, stale-run identifiers, Git as the source of truth for changes, and
never persisting secret environment-variable values.

The reference also shows what not to reproduce: a large mutable application
object, provider and VCS dispatch layers, mixed libgit2/CLI mutations, dozens of
dialogs, synchronous subprocess work in the UI loop, and settings whose
complexity reflects features outside this product.

No source is selected for direct porting. The behavior is small enough to
reimplement with narrower types and tests. Product wording and architecture
below are original to Codex Dirigent.

## Retained workflow map

| Workflow | Core module | Rust UI | Persistence | Focused verification |
| --- | --- | --- | --- | --- |
| Open a local repository | `workspace` | native folder dialog and recent path | last repository only | reject non-Git paths; open temp repo |
| Browse files read-only | `workspace::tree`, `workspace::viewer` | file tree and code pane | none | ignore `.git`; path containment; text-size limit |
| Create repository/file/range cue | `cue` | cue composer bound to selection | current session | prompt and range validation tests |
| Execute with Codex | `codex` | Run/Cancel and restrained progress pulse | Codex settings only | fake CLI JSON stream, cancellation, arguments, env allowlist |
| Refine with follow-up | `session` | conversation and follow-up composer | current session | state transition and prompt-context tests |
| Review changes | `git::diff`, `review` | unified diff pane | none | tracked and untracked diff fixtures; review gate tests |
| Accept or reject | `review`, `git` | explicit actions | none | accept records reviewed snapshot; reject restores only run-owned paths |
| Commit accepted work | `git::commit` | commit sheet and shortcut | none | commit impossible before acceptance or after tree changes |
| Configure execution | `settings` | compact settings sheet | atomic JSON in app support | defaults, obsolete fields, corrupt file recovery, no secret values |

## New architecture boundary

The crate will have a small library for domain and subprocess behavior plus an
`eframe`/`egui` macOS application binary. `App` owns one `Workspace`, one
`Session`, `Settings`, and UI-only state. Background Codex execution reports
typed events over a channel. Git operations use the `git` executable with
explicit arguments and `LC_ALL=C`; there is no generic backend trait because
there is only one backend of each kind.

The review safety invariant is: a commit is enabled only after the user accepts
the exact current diff fingerprint. A changed working tree invalidates that
acceptance. Reject is scoped to paths captured as part of the run and is
confirmed in the UI; unrelated pre-existing changes are never silently reset.

## Deliberate exclusions

- Every non-Codex agent, provider selection, hosted model endpoint, provider
  compatibility type, and provider-specific prompt or setting.
- Jujutsu and VCS dispatch, remote repository and SSH behavior, pull requests,
  forge hosting, LSP, external finding sources, workflow orchestration, MCP
  sidecars, telemetry, and autonomous agent pools.
- Fast-model analysis, smart-interaction tuning, custom themes, animation
  selection, novelty animations, sounds, and game-like views.
- SQLite: the focused session does not need a database. Small settings use
  atomic JSON and cues live in the current application session.
- Reference assets and packaging metadata. New identity assets and metadata
  will be authored from scratch.

## Data and security constraints

- Never read or import the reference `.env`, `.dirigent`, `.cache`, databases,
  `.git`, build output, machine settings, or generated files.
- Repository paths are canonicalized; browsed and reverted paths must remain
  beneath the selected worktree.
- Codex receives explicitly configured environment variable names resolved at
  runtime, never persisted values.
- Pre/post-run scripts are argument-vector commands, not shell strings; shell
  control syntax is rejected. Pre-run failure blocks execution and post-run
  failure is reported.
- Prompts are passed through stdin. User strings are never interpolated into a
  shell command.

## Phase acceptance checklist

- [x] Clean Rust/macOS application shell, formatting, linting, tests, packaging
- [x] Local Git workspace, bounded tree, read-only viewer, status and diff
- [x] Cue types and review/accept/reject/commit state machine
- [x] Codex JSON execution, progress, cancellation, follow-up, scripts
- [x] Minimal atomic settings with obsolete-setting tolerance
- [x] Codex identity, macOS interactions, accessibility, light/dark appearance
- [x] Installation, prerequisite, usage, and release documentation
- [x] Clean build/test/lint plus forbidden-term and artifact audits
