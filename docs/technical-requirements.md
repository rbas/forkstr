# Forkstr technical requirements

Status: MVP implementation contract, following authorization to implement on 6 October 2026. The [functional requirements](functional-requirements.md) remain the product baseline. Implementation refinements and verified platform behavior are recorded below.

## Design summary

Use one executable with a single execution coordinator on a supervision thread. The coordinator walks stages in order, launches commands up to the active stage's limit, and applies failure policy. Process supervision, output capture, and terminal presentation are separate modules within the same application.

Use a PTY per command for terminal mode and ordinary pipes for plain mode. Keep raw output on disk, bounded terminal state in memory, and one ordered final report. No daemon, database, dependency graph, or separate scheduling service is needed.

Rust is the selected implementation language. Use cargo-nextest as the unit and integration test runner. Tests use the standard Rust test harness; Nextest runs them locally and in CI.

## Architecture and code quality principles

Use SOLID principles, domain-driven design, and established Rust best practices to produce clean, idiomatic, maintainable code. Apply them in proportion to Forkstr's scope; every abstraction should protect an invariant, isolate an effect, or simplify an actual use case.

Apply SOLID through Rust modules, composition, and focused traits:

- Single responsibility: give configuration, execution decisions, process supervision, capture, rendering, and reporting distinct responsibilities.
- Open/closed: isolate genuine variation, such as process transports and presentation, behind small boundaries so new implementations do not require rewriting scheduling rules. Do not build speculative extension frameworks.
- Liskov substitution: implementations of a shared trait must honor the same behavioral contract, including error, cancellation, and cleanup semantics. Express capability differences explicitly rather than hiding unsupported operations behind apparent compatibility.
- Interface segregation: expose only the operations each consumer needs; avoid broad traits that force implementations to provide unrelated behavior.
- Dependency inversion: keep execution policy independent of OS handles and terminal libraries. Define effect boundaries from the coordinator's needs and supply concrete implementations at application startup.

Use domain-driven design to model Forkstr's own vocabulary: pipeline, stage, command, execution limit, failure policy, outcome, and cancellation. Keep invariants and state transitions in the model, separate from configuration serialization and infrastructure. Choose ownership and consistency boundaries around the decisions that must change together. Domain-driven design does not require a repository layer, database, event sourcing, or a separate crate for each concept.

Follow idiomatic Rust ownership and API conventions. Give data a clear owner, borrow when ownership transfer is unnecessary, and clone when duplication has a concrete purpose. Use shared ownership and interior mutability only where the execution model requires them. Prefer composition, enums, exhaustive matching, validated constructors, and `TryFrom` for fallible conversions over inheritance-like structures, related boolean flags, or invalid combinations of optional fields.

Keep functions and public APIs focused, use domain-specific names, and document invariants and non-obvious tradeoffs. Propagate recoverable errors with `Result` and `?`, preserving typed causes and useful context. Do not use panics, `unwrap`, or `expect` for configuration, child-process, I/O, or other recoverable runtime failures. Keep unsafe code confined to necessary platform integration, with documented safety obligations and targeted tests.

Make concurrency ownership and shutdown explicit. Track workers, bound queues, minimize shared mutable state, and never let an abandoned worker outlive its intended lifecycle silently. If async is introduced, keep blocking I/O off executor threads and avoid holding locks across await points.

Review each implementation increment for correctness, domain invariants, ownership, error handling, cleanup, and readability. Run `cargo fmt --check`, `cargo nextest run`, and `cargo clippy --all-targets -- -D warnings` once the Cargo package exists. Resolve findings with the smallest clear change; document narrowly scoped lint exceptions when justified. Add behavioral tests for meaningful contracts rather than tests that merely reproduce implementation details.

## Rust structure and libraries

Use one Cargo package with a thin binary entry point and a library containing `config`, `model`, `runner`, `process`, `capture`, `terminal`, and `report` modules. Keep the coordinator's scheduling decisions testable without spawning OS processes. A library target is for internal testing and organization, not a promise of a public SDK.

Use typed stage and command identifiers, a validated positive concurrency limit, `PathBuf` for resolved paths, `Duration` for deadlines, and enums for failure policy, transport, UI mode, and execution status. Represent outcome-specific data inside enum variants rather than independent optional success, failure, and cancellation fields. Keep lifecycle fields private and expose checked transition methods. Deserialize into configuration DTOs, then validate into execution-plan types.

Store process handles and file descriptors in the process layer, outside the scheduling model. Pass typed lifecycle events to the coordinator. Define a small process-backend boundary for launching, resizing, signalling, and observing completion; avoid repository abstractions or a general event bus. Use typed errors for configuration, launch, capture, cleanup, and reporting failures so policy does not depend on matching error strings.

Use two application threads: a coordinator handling nonblocking child I/O and capture, and the main thread handling terminal presentation. Each command receives a bounded read budget per coordinator pass. This avoids a reader and waiter thread per command while keeping terminal writes independent of process supervision. No async runtime is required. Capture writes may apply storage backpressure.

| Library | Responsibility |
| --- | --- |
| nix and libc | PTY setup, process groups, nonblocking descriptors, signals, and wait status |
| vt100 | Bounded virtual terminal screens for panes |
| vte | Incremental parsing and safe readable transcript projection |
| Ratatui and Crossterm | Pane layout, drawing, outer terminal modes, input, and resizing |
| clap, serde, and toml | CLI, configuration DTOs, and parsing |
| signal-hook | Signal-safe cancellation notification |
| tempfile and serde_json | Private run directories and capture event metadata |
| thiserror | Typed configuration errors with preserved sources |

The Unix backend was selected over portable-pty because Forkstr needs direct control over process-group ownership and delayed reaping. It observes exit with `waitid(WNOWAIT)`, keeps the direct child's PID reserved through group cleanup, and then reaps it. Nonblocking reads and writes avoid worker shutdown problems around stuck output handles. macOS uses libproc and Linux uses `/proc` to distinguish live group members from zombies. Darwin's zombie-only process-group behavior was checked against the [Apple kernel signal implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c).

Keep unavoidable unsafe Unix calls confined to platform integration with documented safety obligations. The post-fork setup uses only async-signal-safe operations. Cargo.lock records the resolved application dependencies. Tests exercise the backend on macOS and Linux.

## Defaults

| Setting | Default |
| --- | --- |
| Configuration | `./forkstr.toml`; no parent-directory search |
| Working directory | Configuration file directory |
| Shell | `/bin/sh` |
| Parallelism | Number of commands in the active stage |
| Failure | `fail-fast` |
| Layout | `auto` |
| UI and transport | Automatic terminal detection |
| Combined report | `always` |
| Graceful shutdown | Two seconds, then forced termination |

## Configuration contract

Use TOML with a schema version and ordered arrays of stages and command objects. One command representation supports names, environment, shell, and working-directory overrides without special cases for serial steps.

```toml
version = 1
shell = "/bin/sh"
failure = "fail-fast"
layout = "auto"
report = "always"
# jobs omitted: run all commands in each stage concurrently

[env]
APP_MODE = "development"

[[stages]]
name = "checks"
# jobs = 2

[[stages.commands]]
name = "format"
run = "./check-format.sh"

[[stages.commands]]
name = "tests"
run = "./test.sh"
env = { TEST_GROUP = "unit" }
resources = ["build-cache"]

[[stages]]
name = "package"

[[stages.commands]]
name = "archive"
run = "./package.fish"
shell = "fish"
```

Pipeline fields: `version`, `shell`, `cwd`, `env`, `jobs`, `failure`, `layout`, `report`, and `stages`. Stage fields: `name`, `jobs`, `failure`, and `commands`. Command fields: optional `name`, required `run`, and optional `shell`, `cwd`, `env`, and `resources`.

Stage names must be nonempty and unique. Explicit command names must be nonempty and unique within their stage. Resource names must be nonempty, contain no control characters, and occur at most once per command; the same name on different commands establishes mutual exclusion. Internal command identity is the pair of stage index and command index, never its display name or command text. Unnamed commands use a generated label such as `command-2`, with their command text available in the pane and report.

Reject unsupported schema versions, unknown fields, empty pipelines or stages, whitespace-only commands, invalid enum values, nonpositive limits, non-string environment values, and invalid environment names. Environment keys cannot be empty or contain `=` or NUL; values and process arguments cannot contain NUL.

Resolve configuration to an immutable execution plan before launching children. Store original registration order, effective working directory, shell, environment, resources, concurrency, and failure policy. Validate directories and shell executability up front; spawning still handles races or changes after validation. Validation does not execute shell scripts or promise command syntax validity.

## CLI contract

```text
forkstr run [--config PATH]
            [--jobs N] [--failure fail-fast|finish-stage]
            [--layout auto|horizontal|vertical]
            [--ui auto|panes|plain] [--transport auto|pty|pipe]
            [--report always|never] [--color auto|always|never]
            [--log-dir PATH]

forkstr validate [--config PATH]
forkstr skill export DIRECTORY
forkstr --help
forkstr --version
```

Explicit CLI `jobs` and `failure` values override every stage. Otherwise stage settings override pipeline settings, then defaults. Presentation flags override their configuration equivalents. No command filtering, ad hoc command grammar, or implicit stage selection is introduced in the MVP.

`skill export` is an offline operation that writes the complete bundled agent
skill beneath a newly created destination directory. It does not inspect project
configuration, authenticate, or configure an agent. Existing destinations are
rejected without modification, and the destination's parent must already exist.

Live output and lifecycle information go to stderr. The combined report goes to stdout. A concise final status summary always goes to stderr, including when the combined report is disabled. Color auto-detection is evaluated separately for each output destination; no color flag can restore styling that a child never emitted.

Automatic pane mode requires terminal input, terminal stderr, and a usable terminal type. Otherwise select plain mode. Explicit pane mode without these prerequisites is a clear error. Transport auto selects PTY for panes and pipes for plain mode. Panes with pipes are observable but do not support child input; plain mode with forced PTY remains noninteractive and must be documented as potentially unsuitable for prompting programs.

## Shell and environment

Resolve a shell path directly or look up a bare shell name using the command's effective PATH and working directory. Do not split the shell field into arguments. Invoke the selected executable with `-c` and the complete command string as one argument. The supported shell contract is compatibility with that invocation; arbitrary interpreters are not implied.

Do not automatically add login or interactive flags, or inject shell-specific startup flags. Shells retain their own normal startup behavior. Fish supports `-c` and has separate options for interactive, login, and configuration behavior, as described in its [invocation documentation](https://fishshell.com/docs/current/cmds/fish.html).

Selecting fish changes interpretation of the `run` string. Executing `./script` still follows the script's own shebang; to interpret a script explicitly with fish, use `run = "fish ./script"`. Forkstr does not rewrite scripts or enable shell options such as `errexit` or `pipefail`. Success is the selected shell's final exit status; users express compound-command failure semantics themselves.

Resolve pipeline cwd against the config directory and command cwd against pipeline cwd. Merge environment in this order: inherited process environment, pipeline overrides, command overrides. Do not perform template interpolation or mutate the parent's environment. PTY setup supplies `TERM=xterm` when absent; an explicitly configured TERM is preserved. Common styling and prompt behavior are tested, but full xterm capability emulation and query protocols are not guaranteed.

## Modules and ownership

| Module | Owns |
| --- | --- |
| Config | Parsing, validation, effective execution plan |
| Coordinator | Stage position, command states, launch permissions, aggregate outcome |
| Process | Child handles, process groups, transport, resize and signal operations |
| Capture | Raw files, received-output order, transcript, bounded display state |
| Terminal | Outer terminal modes, input routing, pane layout and drawing |
| Report | Ordered iteration of results and transcript files |

Use one coordinator event loop with nonblocking child descriptors and one independent presentation thread. Snapshot updates and input requests cross bounded presentation/control boundaries.

Record output before publishing display updates. Keep execution outcomes in coordinator-owned state and raw capture separate from a bounded live-display queue. If the display falls behind, report the number of omitted live records; captured bytes and final outcomes are retained. UI writes never block the coordinator. Slow storage may backpressure a child; storage failure stops the pipeline as an internal error.

## Execution and state model

Command execution states are `queued`, `running`, `settling`, `succeeded`, `failed`, `cancelled`, and `not-started`. `settling` means the direct child has exited or shutdown has started, but cleanup or output draining is still in progress. Store cancellation intent separately so it cannot erase an already-observed result.

Stages move from `pending` to `running` to `succeeded`, `failed`, or `cancelled`; untouched stages may become `skipped`. The pipeline moves from `pending` to `running`, optionally `cancelling`, then `succeeded`, `failed`, or `cancelled`. Record infrastructure errors and failure reasons separately from these statuses.

The coordinator fills available slots with the first resource-eligible queued command. A later independent command may pass a command waiting for a resource. Jobs and resources remain occupied through settling. Observe a nonzero child exit immediately for fail-fast purposes; do not wait for output EOF to stop launches. Only move to the next stage after every started command has settled and every queued command has a final disposition.

On fail fast, the initiating command fails, active peers receive cancellation requests, and queued commands become not started. Finish-stage continues to fill slots until all commands finish, then checks aggregate success. Terminal states never revert. A child whose completed result was observed before cancellation retains it; otherwise mark a command cancelled when Forkstr interrupts unfinished execution and retain the actual wait status as diagnostic data.

User cancellation takes precedence in the pipeline's displayed outcome, while existing command and stage failures remain visible. An infrastructure error stops the pipeline regardless of command failure policy and must never produce a successful result.

## PTY and input contract

Each terminal command receives an independent PTY with stdin, stdout, and stderr attached to its slave side. Output is consequently merged. A PTY supplies terminal behavior and terminal-generated signals; it is not just a colored-output pipe. See the [PTY interface](https://man7.org/linux/man-pages/man7/pty.7.html).

The backend establishes the child session, controlling terminal, and foreground process group before execution. Session creation and foreground ownership are distinct operations; verify both on macOS and Linux using the [session contract](https://man7.org/linux/man-pages/man3/setsid.3p.html) and [terminal foreground-group contract](https://man7.org/linux/man-pages/man3/tcsetpgrp.3p.html).

Pipe mode uses null stdin and independently captures stdout and stderr. Preserve each stream's byte order and record Forkstr's receive order for the combined transcript; this is not an exact reconstruction of child write order across streams.

Keyboard contract:

| Context | Input | Action |
| --- | --- | --- |
| Observe mode | Tab / Shift-Tab | Select next / previous command pane, including while enlarged |
| Observe mode | z / Esc | Toggle selected pane enlargement / restore configured layout |
| Observe mode | Enter | Enter input mode for the selected PTY command |
| Observe mode | Ctrl-C | Cancel the pipeline; repeat to force shutdown |
| Input mode | Ordinary keys and paste | Forward only to the bound command |
| Input mode | Ctrl-C | Forward the interrupt character to the child terminal |
| Input mode | Ctrl-G | Return to observe mode |

Display the mode, selected command, and escape/cancellation hint continuously. Ctrl-G is reserved by Forkstr and is not forwarded in the MVP. Escape remains available to child programs. A real SIGINT delivered to Forkstr itself always requests pipeline cancellation regardless of UI mode.

Bind input to command identity, never a reusable pane slot. Process exit clears input mode. Queued input targeting an old identity is discarded, not redirected. Do not write keyboard input to capture files; child-echoed output is captured normally. Password echo is controlled by the child terminal. Bound pending input and handle partial writes without blocking supervision.

Job suspension and full interactive shell job control are outside the MVP. Do not expose a Forkstr suspend shortcut. Document child suspension as unsupported and keep pipeline cancellation available even if a child stops itself.

## Rendering and capture

Use an existing terminal parser for each command's virtual screen. Never pass raw child terminal controls to the outer terminal. Support styled text, carriage returns, basic cursor positioning, erase operations, and incremental UTF-8 parsing. Unsupported controls must be consumed without affecting outer terminal state. A streaming filter discards unsupported terminal strings before parsing, preventing unterminated OSC or DCS payloads from growing parser memory; raw capture is written first. Terminal-query reply protocols and full-screen application compatibility remain outside the MVP guarantee.

Draw at a capped rate of approximately 30 frames per second, independently of output arrival. Maintain bounded screen and scrollback state per command in the active stage. Automatic layout uses a grid; horizontal and vertical layouts use the documented orientation. Reserve stable panes in registration order for all commands in the active stage. Show queued placeholders and retain completed screens and statuses until the stage ends. Selection includes completed panes; only running commands accept input. Publish the new stage and clear old screens together when transitioning.

On resize, update visible child PTY sizes and virtual screens. Hidden commands retain their last usable size; commands that have never had a pane use 80 by 24 until shown. Avoid zero-size PTYs. If panes do not fit, show a selectable compact list and the selected command's output; capture all other commands normally.

Create a unique run directory under the OS temporary directory by default, or under `--log-dir` when supplied. Use restrictive permissions and numeric command identities for filenames. Retain it after execution and print its location; temporary storage can be removed by the OS, while an explicit log directory is user-managed. No background retention service is introduced.

Store raw output bytes, compact metadata for stream identity and receive order, resize events needed for interpretation, and a readable transcript. PTY bytes are terminal output bytes, not necessarily identical to the child's original writes. Record only observed data and explicit truncation or capture-error markers.

Generate transcripts incrementally with bounded buffers. Preserve every newline-completed line; coalesce carriage-return rewrites of an unfinished line and flush its final content at exit. Normalize basic horizontal cursor edits in bounded line state, retain supported style, and discard terminal side effects. Vertical screen editing is represented by raw capture and the live virtual screen, not a historical transcript of every screen state. This transcript is a readable projection; raw files remain the complete capture when screen rewrites cannot be represented as append-only text.

For plain live output, prefix records with stage, command identity, and stream when known. Emit partial fragments on a short timer, initially 100 ms, or a bounded chunk threshold, initially 8 KiB, so a missing newline cannot hide output or grow memory indefinitely. Mark continuations explicitly. Sanitize controls and serialize output records so prefixes cannot be overwritten by child output.

Invalid text bytes are replaced for display only; raw capture retains them. A disk-write error cancels the pipeline and reports incomplete capture. Backpressure is allowed, silent capture loss is not.

## Process cleanup and terminal restoration

Track the owned child and process group for every command. On cancellation, stop launching, signal owned groups with SIGTERM, allow two seconds while draining output, then signal remaining groups with SIGKILL. Resume stopped owned processes as needed to permit graceful termination. Reap direct children; descendants may be reaped by the OS. Group signaling follows the [POSIX kill contract](https://man7.org/linux/man-pages/man3/kill.3p.html).

After direct-child exit, clean up remaining owned group members rather than allowing background descendants to outlive the command. Group membership does not cover descendants that deliberately create other process groups or sessions. Platform tests verify ordinary shell descendants and foreground terminal behavior without claiming arbitrary process-tree containment.

Do not signal arbitrary PIDs found by process-name matching. Avoid stale process-group identifiers and account for identifier reuse in backend lifecycle ownership. If a backend cannot establish safe ownership, spawning fails rather than running an unsupervised command.

After forced cleanup, drain output for at most one further second. Mark capture incomplete if handles remain open or cleanup cannot finish; report an internal error. An uninterruptible OS task or detached descendant cannot be guaranteed to disappear within a deadline. Never report successful cleanup when it is unresolved.

Restore terminal modes and leave the alternate screen before printing the final report. Use an ownership guard for normal completion and handled errors, plus centralized signal handling. SIGKILL and machine failure cannot guarantee restoration. Treat terminal disconnection as cancellation. Report write failures explicitly and return nonzero where output remains available to do so.

## Final report and exit codes

Iterate the immutable execution plan in registration order. Print each stage, each command's identity and status, reason and exit details, then its transcript when enabled. Commands with no output and skipped work still appear. Reset styles around each section and include the run-directory location in the final summary.

| Exit | Meaning |
| --- | --- |
| 0 | All required commands succeeded and finalization succeeded |
| 1 | Command failure |
| 2 | Configuration or Forkstr infrastructure error |
| 130 | User cancellation or SIGINT |
| 143 | SIGTERM cancellation |

Infrastructure failure has exit-code precedence if it prevents reliable cleanup or reporting; otherwise cancellation takes precedence over prior command failure. Preserve all underlying causes in the summary. Shell child exit codes are diagnostic data, not directly propagated as Forkstr's exit code.

## Validation gates

Nextest tests cover password echo behavior, focused-input isolation, resize, interrupt delivery, cancellation, ordinary descendant cleanup, and terminal restoration on macOS and Linux. Maintain these behavioral gates when changing the backend. Library API availability alone does not prove the contracts. See the [implementation plan](implementation-plan.md) for the development sequence and validation scope.
