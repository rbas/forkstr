# Forkstr implementation plan

Status: initial MVP implemented. This sequence records the implementation increments and their continuing validation requirements. The [technical requirements](technical-requirements.md) describe the implemented contracts.

Every increment follows the technical requirements' architecture and code quality principles: SOLID, domain-driven design, and clean, idiomatic Rust. Review responsibility boundaries, domain invariants, ownership, error semantics, and worker cleanup as part of the work. Once the Cargo package exists, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo nextest run` with the scope appropriate to the increment. cargo-nextest is the required unit and integration test runner; tests use Rust’s standard test harness. Introduce abstractions only where they serve a concrete contract or effect boundary.

## Step 1 Select the stack and verify terminal feasibility

Use Rust and choose a minimal set of libraries for CLI parsing, TOML, process and PTY control, terminal parsing, and rendering. The selected libraries are recorded in the technical requirements. Avoid a multi-package workspace unless a concrete need appears.

Build a disposable backend prototype with two PTY children. Demonstrate input focus, an ordinary prompt, a password-style no-echo prompt, concurrent output, resize, child interruption, and pipeline cancellation. Verify shell descendants and terminal restoration on macOS and Linux. Probe terminal capability advertising and unsupported control handling.

Acceptance: tests distinguish command interrupt from pipeline cancellation; no input transfers after a command exits; ordinary descendants are cleaned up. Record limitations before integrating the backend. Do not promise Linux validation from a macOS-only run.

## Step 2 Configuration and execution model

Implement parsing, validation, immutable plan resolution, stable command identities, environment merging, working-directory resolution, and state transitions. Add the validation CLI without executing shell commands.

Acceptance: table-driven tests cover invalid configuration, override precedence, default unlimited stage concurrency, duplicate names, directory resolution, and environment isolation.

## Step 3 Serial headless execution

Implement pipe execution, direct-child status observation, raw capture, simple reporting, and sequential stage barriers using one execution slot.

Acceptance: fixture programs verify exit success and failure, launch errors, empty output, stderr, shell invocation, working directories, and environment overrides. No pane UI is needed yet.

## Step 4 Concurrency and failure policy

Add bounded command scheduling, fail fast, finish-stage, skipped work, and aggregate outcomes. Keep launch decisions in the coordinator.

Acceptance: controlled helper processes synchronize through handshakes rather than timing assumptions. Verify maximum concurrency, launch order, stage barriers, immediate failure observation before output EOF, and simultaneous completion/cancellation races.

## Step 5 Complete process cleanup

Integrate the proven backend lifecycle, graceful and forced termination, repeated cancellation, output drain deadlines, and signal handling.

Acceptance: subprocess fixtures spawn grandchildren, retain output descriptors, ignore SIGTERM, stop themselves, and exit during cancellation. Verify outcomes and check that supported descendants are gone. Report cleanup limitations instead of concealing failed assertions.

## Step 6 Capture and plain presentation

Complete bounded output handling, persistent run files, readable transcript generation, attributed plain output, lifecycle messages, and ordered reports.

Acceptance: test split UTF-8 and escape sequences, carriage returns, no-newline output, large output, separate stream ordering, disk-write failure, partial capture, and output sink failure. Assert raw bytes separately from normalized display text.

## Step 7 PTY execution and focused interaction

Integrate terminal transport, observe/input modes, command-bound input, bounded input writes, and interrupt behavior. Keep child output capture independent of renderer speed.

Acceptance: automate prompt replies and no-echo behavior with a test PTY. Verify literal input goes only to its intended command and that exit, pane reuse, and cancellation cannot redirect buffered input.

## Step 8 Panes and terminal lifecycle

Add horizontal, vertical, and automatic layouts, stage transitions, compact small-window behavior, status counts, and resize propagation.

Acceptance: test layout calculations and virtual screens independently, then run end-to-end terminal fixtures. Manually inspect readability, focus hints, colors, progress updates, resizing, and terminal restoration on both platforms.

## Step 9 Release validation

Run all functional acceptance scenarios, CLI help examples, redirected output, report-disabled runs, missing shells, and both failure policies. Verify deterministic report order and all exit-code classes. Check measured memory behavior under sustained output.

Deliver a small documented executable and configuration example. Record tested platform versions and known terminal limitations. Graphical progress bars, full-screen application compatibility, and continue-after-failure remain follow-up work.

## Validation on 6 October 2026

- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings` pass.
- All 32 Nextest tests pass on macOS 26.6.2 on Apple Silicon with Rust 1.97.1.
- The same 32 tests pass in an aarch64 Linux container using the Rust 1.93 Debian Bookworm image. A macOS/Linux CI matrix preserves this coverage for subsequent changes.
- The optimized macOS release binary builds and runs the example pipeline. Fish selection was verified with an installed fish executable.
- PTY integration tests exercise real pane input, resizing, password echo behavior, command interruption, pipeline cancellation, and restoration of the outer terminal settings.
- Failure tests cover both execution policies, skipped work, capture creation and write errors, and a broken final-report pipe. The SIGTERM cancellation test also passed 20 consecutive Nextest stress iterations.
- An unterminated 32 MiB terminal-control string is discarded by a bounded display filter; raw capture remains independent of this normalization.
- The release binary captured 2 MiB and 16 MiB streams exactly, with approximately 4.5 MiB and 4.6 MiB peak resident memory, respectively. This is a bounded-memory smoke check, not a general performance guarantee.
- Documentation links, embedded TOML examples, and the runnable example configurations validate.

The PTY feasibility work is retained as backend and end-to-end regression tests. Full-screen terminal applications, terminal-query protocols, intentionally detached descendants, and job suspension remain outside the tested MVP contract.
