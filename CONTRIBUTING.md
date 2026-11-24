# Contributing

Pull requests are welcome. Keep one change per PR, and give a behaviour change a test that fails without it.

## CI and local checks

`.github/workflows/ci.yml` is the command set; run all of it before you push. Its `ui-smoke` job runs a bundled build (embedded assets under the CSP, which `trunk serve` skips) until the UI logs `UI mounted`. Locally: `trunk build --release` in `ui/`, `cargo build -p exilence-ce-ng --features tauri/custom-protocol`, then `tools/ui-smoke.sh target/debug/exilence-ce-ng`. A blank window leaves its cause in `exilence-ce-ng.log` as an `ERROR ui:` line (CSP violations and page errors, from `ui/index.html`).

PR CI only cross-checks the Windows and macOS targets; `os-tests.yml` runs the tests there (`release.yml` calls it too). Anything platform-specific (paths, windows, the tray, the overlay, deep links) needs an `os-tests.yml` run (`workflow_dispatch`) before it merges.

Windows or macOS flakes: `gh workflow run os-tests.yml --ref <branch> -f os=windows-latest -f rounds=5 -f filter=<test>` stress-runs the core tests there through `tools/stress.sh` and names every failing test. A run that fails before any step ran was never started: the reason (billing, spending limit, runner) is in the job's annotations, `gh api repos/exilence-ce/exilence-ce/check-runs/<job id>/annotations`.

A change to a workflow or a `tools/` script runs once before its commit: dispatched (`gh workflow run`), or locally with the workflow's `env`. CI can't check a `workflow_dispatch` input, and a script that only a dispatched workflow runs fails first when it's needed.

## Test harness

The core tests live in `src-tauri/tests/core/`, against fake GGG, poe.ninja and backend servers.

- Fake servers come from `mock_server()`. `MockServer::start()` pools servers across tests.
- Time and randomness go through the injected `Clock` and `Jitter`.
- `Harness::invoke` uses the window's own URL (the origin differs on Windows).
- Helpers (`next_toast`, `WAIT`, `poll`, request counters) are defined once in `src-tauri/tests/core.rs`; reuse them before adding another.
- A signed-in harness comes from `Harness::with_login()`, with named options for the fakes, the token expiry and the Profiles.
- A test waits on the event that would have happened; no sleep followed by asserting nothing. Background work that must have no effect is waited for to its last step (`Harness::wait_for_ready` for the session start's GGG part, which for an account without a local Profile ends only once `StartSession` (and a legacy import offer's answer) made its Profiles known, so a test with the backend down stores a Profile (`WithLogin::profiles`); `Setup::wait_until_ended` for a Snapshot and its upload) or run to its end with `block_on` (`group_upload_queue::drain`, `snapshot_removal::send_pending`). Work queued behind the operation slot shows in `OperationSlot::waiting`; a fake backend call is held with `FakeBackend::hold_next_upload` / `hold_next_profile_call`.
- Every slice change and toast goes out through `state::` (one lock; Tauri parks Rust-side events emitted from two threads at once).
- A flaky test goes red in `tools/stress.sh [filter] [copies] [rounds]` (16 × 20 by default), which names each failing test with its output; a fix holds once the same run stays green.
- Tests that prove a parity criterion carry its `F<n>`/`D<n>` number in their name.

## Merging

- Merge hot spots: `src-tauri/src/commands.rs`, the handler list in `lib.rs`, `ipc/src/command.rs`, `ipc/src/message.rs`, `ui/src/main.rs`, the `mod` lists of `tests/core.rs`.
- `.gitattributes` unions `CHANGELOG.md`, `*.ftl` and `ui/style.css`.
- `Cargo.lock` conflict: `git checkout --ours Cargo.lock && cargo metadata >/dev/null`, then commit.
- `CHANGELOG.md` sections are written when a release is cut; a PR leaves it alone.
