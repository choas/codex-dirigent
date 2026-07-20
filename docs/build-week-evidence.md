# OpenAI Build Week evidence

Evidence date: 2026-07-20 (Europe/Berlin). Target track: **Developer Tools**.

- Public source repository: <https://github.com/choas/codex-dirigent>
- Default branch: `main`
- License: MIT

## Codex and GPT-5.6 contribution

Codex CLI `0.144.5` was invoked explicitly with the ChatGPT-supported GPT-5.6
model identifier `gpt-5.6-sol` in an isolated linked Git worktree.

- Codex session ID: `019f8086-8707-7fc2-89d8-e68341788eaf`
- Task: implement durable, versioned cue-board persistence and safe restart
  reconciliation
- Human direction: persistence boundaries, required recovery behavior, safety
  invariants, focused tests, documentation, and full Rust quality gates
- Human review: inspected the persistence schema and all transition/recovery
  paths; retained the rule that approval fingerprints, diffs, generated output,
  subprocess state, and secrets are never persisted
- Result commit: `a474f46` (`Persist cue board across restarts`)
- Integration commit: `8e06e26` (`Merge durable cue persistence`)

The implementation adds atomic `cue-board.json` persistence for Inbox cues,
exact repository/file/line targets, user-authored follow-ups, lanes, and linked
branch metadata. Recovery joins saved branches against Git's live linked
worktrees, rejects stale metadata, regenerates diffs, and requires fresh review
where an exact approval cannot be proven.

## Verification evidence

The merged `main` branch passed:

```text
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test
```

Test result: 44 unit tests and 2 integration tests passed. The integration suite
includes concurrent cues running in isolated worktrees and merging cleanly into
`main`.

## Judge build

The release workflow produced a universal `arm64` + `x86_64` macOS application
with hardened runtime and a timestamped Developer ID signature.

- Artifact: `Codex-Dirigent-0.1.0-macos-universal.zip`
- SHA-256: `bef92de67c19d90c0fb0ec17a323402c2740e6d06c7cfe9dbdc7dcc4b2719f00`
- Signing authority: `Developer ID Application: Lars Gregori (U49ZNKS7D7)`
- Bundle identifier: `com.openai.codex-dirigent`
- Signature verification: passed `codesign --verify --deep --strict`
- Archive integrity: passed checksum verification and `unzip -t`
- Apple notarization: pending; no `notarytool` keychain profile is configured on
  the build machine

Generated build artifacts remain ignored by Git. Publish the ZIP and checksum
as release assets only after notarization and staple validation succeed.

## Submission artifacts still requiring owner accounts

- Configure a `notarytool` keychain profile, rerun the release script with
  `NOTARY_PROFILE`, and publish the notarized ZIP plus checksum.
- Record and upload the public, audible, sub-three-minute YouTube demonstration.
- Add the repository URL, release URL, video URL, and this Codex session ID to
  the Devpost submission and test every link while signed out.
