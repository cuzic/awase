# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**awase**（合わせ）is a Windows keyboard remapper that emulates NICOLA thumb-shift (親指シフト)
input by hooking the low-level keyboard, detecting simultaneous keystrokes, and sending romaji to
the OS IME. The hard part isn't the key-remapping logic — it's tracking and correcting IME
ON/OFF/composition state across dozens of app types (Win32, TSF-native like Chrome/VS
Code/WezTerm, UWP) whose IME behavior is inconsistent and frequently lies about its own state.
Most of the architecture exists to make that tracking resilient and auditable. See
[ARCHITECTURE.md](ARCHITECTURE.md) and [README.md](README.md) for user-facing docs.

## Commands

Build/check must target Windows explicitly — the default host (Linux) target silently skips all
`#[cfg(windows)]` code (`crates/awase-windows`'s Win32/TSF layer), which is most of the platform
logic.

```sh
# Compile check (what you'll use most while iterating)
cargo check --target x86_64-pc-windows-msvc -p awase -p awase-windows
# or: mise run check

# Clippy (same target requirement)
cargo clippy --target x86_64-pc-windows-msvc -p awase -- -A clippy::cargo
# or: mise run clippy

# Release build (produces target/x86_64-pc-windows-msvc/release/awase.exe)
cargo build --release --target x86_64-pc-windows-msvc

# Format
cargo fmt -- --check          # or: mise run fmt

# All pre-push checks at once
mise run pre-push              # check + clippy + test + fmt
```

Tests run fine on Linux for everything except real Win32/TSF interaction — most of the platform
layer is exercised via source-scanning guard tests (see Testing below) rather than real API calls.

```sh
cargo test --lib                                    # core engine unit tests (host target, no --target needed)
cargo test --test scenarios                          # simultaneous-keystroke scenario tests
cargo test -p timed-fsm                               # timed-fsm framework tests
cargo nextest run --workspace --lib                   # what CI runs (nextest, all lib tests)
cargo nextest run -p awase-windows --test architecture_guard --test golden_scenarios --test layer_boundary_guard
cargo test <test_name>                                 # run a single test by name (any of the above targets)
```

Windows-only tests (`ime_key_sequence_golden`, `thumb_context_guard`, `e2e_windows`) only run
under `#![cfg(windows)]` and only assert for real in the `windows-build` CI job on a
`windows-latest` runner — don't expect them to run (or fail) locally on Linux.

The same applies one level deeper and less visibly: `crates/awase-windows/src/runtime/mod.rs`
carries `#[cfg(windows)]` on the whole module tree, so any `#[cfg(test)]` unit test living inside
`runtime/` (e.g. `runtime/open_chain.rs`, `runtime/transport.rs::plan_tests`) or inside any other
`#[cfg(windows)]`-gated file (`ime_controller.rs`, `ime.rs`, `imm.rs`, `hook.rs`, ...) silently
does not exist in the native-Linux test binary at all — it won't show up in `cargo test --list`,
`cargo nextest list`, or even `strings` on the compiled binary, and there is no error or skip
message. This is not a bug to chase; verify such tests instead with
`cargo check --target x86_64-pc-windows-msvc -p awase-windows --tests --lib` (compiles cleanly
without needing a linker) and trust the real run to `windows-build` CI (this sandbox has no
`link.exe`, so `cargo test --target x86_64-pc-windows-msvc ... --no-run` fails on the link step
even when the code is correct — that failure is expected here, not a signal).

```sh
# Custom dylint lints (layer-boundary enforcement — see Architecture below)
cargo install cargo-dylint dylint-link --version 6.0.0 --locked   # once
DYLINT_RUSTFLAGS="-D warnings" cargo dylint --all -p awase-windows -- --target x86_64-pc-windows-msvc

# Mutation testing (slow — validity of tests, not coverage)
mise run mutants                                       # awase core only, full run
git diff main | cargo mutants --in-diff -               # diff-scoped, for pre-review self-check
cargo mutants -p awase-windows --config .cargo/mutants-awase-windows.toml   # platform-independent subset of awase-windows
```

Dead-dependency check: `cargo machete`. Security advisories: `cargo audit` (triaged in
`.cargo/audit.toml` when unfixable).

## Architecture

### Workspace layout

```
awase (root crate, src/)      Platform-independent core: engine/, config.rs, ngram.rs, kana_table.rs, yab/
crates/awase-windows/         Windows platform implementation (the bulk of the complexity)
crates/awase-linux/           Linux platform stub
crates/awase-macos/           macOS platform stub
crates/win32-async/           Async executor + blocking-API timeout isolation (run_with_timeout)
crates/win32-worker/          Worker-thread primitives used by win32-async
crates/awase-settings/        Settings GUI (eframe/egui) — separate binary, awase-settings.exe
crates/awase-gji-config/      GJI (Google 日本語入力) config file handling
crates/awase-vkmap/           VK code / scan code mapping tables
crates/awase-build-support/   Shared build.rs logic (manifest embedding etc.)
crates/timed-fsm/             Standalone timer-aware FSM framework (published to crates.io independently)
```

The core `awase` crate must stay OS-independent (ADR-019) — no `windows-rs`, no
`#[cfg(target_os)]`, no raw VK-code magic numbers outside `crates/awase-vkmap`. Platform crates
classify raw input first and hand the core engine only pre-classified events
(`KeyClassification`/`ImeRelevance`/`PhysicalPos`/`ModifierKey`); the engine never branches on raw
VK/scan codes. `docs/layer-boundaries.md` has the full rule set (categories A–E) with grep-based
detection commands for each — treat it as the PR review checklist for any cross-layer change.

### Concurrency model

Single-threaded, message-loop-driven (`winmsg-executor`; no tokio). Keyboard hook, timers, and
focus detection are all `spawn_local` async tasks on the same thread — no locks needed for
in-process state. Blocking Win32/COM APIs (IMM32, MSAA, UI Automation) that have no async
equivalent and can hang indefinitely on an unresponsive window are isolated via
`run_with_timeout` (`crates/win32-async/src/thread_timeout.rs`): spawned on a worker thread, given
300ms, and if they don't return in time the result is discarded and the thread parked in an
orphan list (`LEAKED_THREADS`, capped at 8, GC'd on next call). The one deliberate exception:
`focus/classifier.rs::INPUT_RELAY_APPS` (`OnceLock<RwLock<Vec<String>>>`) exists because
`ime.rs::read_ime_state_fast` is a `self`-less `pub unsafe fn` reachable from both the
`offload_unsafe` worker thread and the main-thread `spawn_local` path — see that static's doc
comment before reaching for a lock anywhere else; any caller with `self` access should use
`FocusTracker::input_relay_apps()` instead.

### awase-windows internal layers

```
observer/    Raw Win32 API / TSF probes → snapshots (Observe)
state/       Pure classify_* functions + ImeModel/belief reducers (Pure decision + Apply)
runtime/     Orchestration: key_pipeline, ime_coordinator, focus_tracking, executor
output/      Actual key injection / IME apply (vk_send, key_injector, tsf_warmup_coord)
focus/       Focus tracking + AppKind classification (Win32/TsfNative/Uwp) with learned cache
tsf/         TSF-specific 4-layer stack: observer/probe/output + warmup/ (cold-start recovery)
app/         Bootstrap / top-level AppState wiring — one of the few places allowed to touch `crate::APP`
```

**IME belief updates are the most bug-prone area of this codebase** — see
`.claude/rules/ime-belief-architecture.md` for the full Observe → pure `classify_*` → `reduce()`
discipline (belief fields are private outside `state/ime_model.rs`; enforced by the compiler, two
dylint lints, and `tests/architecture_guard.rs`). Read it before touching anything in `state/` or
any `ImeEvent` dispatch — the background section documents several real incidents where shortcuts
here caused silent state corruption.

### Testing structure

- `crates/awase-windows/tests/architecture_guard.rs`, `layer_boundary_guard.rs` — source-scanning
  guard tests enforcing the layer rules above; run on Linux (text-based, no real Win32 needed).
- `ime_key_sequence_golden.rs` — golden tests for strategy selection (ImmCross → GjiDirect →
  MsImeDirect → KanjiToggle) and exact key sequences sent; `#![cfg(windows)]`, Windows-only.
- `golden_scenarios.rs` / `tests/golden/` — scenario goldens, cross-platform.
- `e2e_windows.rs` — real end-to-end tests against actual HWNDs; deterministic subset runs in CI,
  interactive/SendInput-dependent subset is `continue-on-error` (non-deterministic under CI focus).
- `journal_replay.rs` / `docs/journal-replay-guide.md` — replay recorded key-event journals through
  pure `classify_*` functions to regression-test belief/conv transitions without real hardware.
- `thumb_context_guard.rs` — regression guard for thumb-key state in `build_input_context`.

### ADRs and known issues

`docs/adr/` holds a large and growing set of Architecture Decision Records (`docs/adr/index.md` is the index).
`docs/known-bugs.md` tracks confirmed bugs with repro steps and fix-commit history — check it
before assuming a symptom is new. `docs/experiments.md` logs reverted approaches for IME-control
tuning so the same rejected idea doesn't get re-tried blind (see
`.claude/rules/experiment-logging.md`).

## Repo-specific workflow rules (auto-loaded, summarized here for orientation)

These live in `.claude/rules/*.md` and are loaded automatically every session — this list is just
a map of what to expect, not a substitute for reading them when the relevant area is touched:

- `main-develop-branch-flow.md` — all work goes through `develop`; `main` only changes via the
  `release-develop-to-main` skill (or a `hotfix/*` branch for urgent main-only fixes).
- `worktree-per-session.md` — parallel Claude Code sessions must use separate `git worktree`
  checkouts, not the same working tree.
- `experiment-logging.md` — revert commits touching IME control/warmup/focus/key-selection must
  document the observed failure (app, IME, repro) in the commit body.
- `tuning-constants.md` — changes to timing constants in `crates/awase-windows/src/tuning.rs` must
  cite a real measurement (ms) in the commit body, not "increase until it works."
- `fix-requires-evidence.md` — fixes in the warmup/focus/belief/conv/key-selection "reincidence
  families" need either a regression test or a `docs/known-bugs.md` entry.
- `ime-belief-architecture.md` — see Architecture section above.
