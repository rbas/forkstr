# Forkstr functional requirements

Forkstr runs an ordered pipeline of sequential stages. Commands within a stage run concurrently, their output remains observable, and stage completion determines whether the pipeline continues. Forkstr is generic: it contains no language, build, test, or deployment-specific behavior.

Status: functional baseline, incorporating the product discussion of 6 October 2026. Implementation was subsequently authorized with Rust, SOLID principles, domain-driven design, idiomatic code, and cargo-nextest. The technical requirements record the selected defaults and implementation contracts.

## Core model

A pipeline contains one or more stages in configuration order. Each stage contains one or more commands in configuration order. The order is stable across execution and reporting.

A stage starts only after the preceding stage completes successfully under the MVP failure policies. All running commands and their cleanup must settle before transitioning to the next stage.

A stage with one command is a serial step. A stage with several commands is a parallel group. A limit of one also allows a multi-command stage to execute serially.

For example, four checks can run together, followed by a build stage, a package stage, and a deployment stage. There are no dependencies between individual commands in different stages beyond the stage barrier.

## Platforms and execution environment

Forkstr supports macOS and Linux.

Each command executes in its own process context. Commands inherit the environment from the process that launched Forkstr. Configuration must allow environment overrides for the pipeline and for individual commands; command values take precedence over pipeline values, which take precedence over inherited values. Overrides must not change Forkstr's own environment or another command's environment.

Values are literal environment strings unless a later requirement explicitly introduces interpolation. Environment configuration is not automatically printed in command headers or reports. Environment removal and stage-level overrides may be added later.

Working directories must be configurable for the pipeline and individual commands. The default is the configuration file's directory. Resolve a relative pipeline directory against that directory, and a relative command directory against the effective pipeline directory.

Shell selection is separate from configuration syntax. A configuration file might be TOML or YAML, while a command string inside it is interpreted by sh, bash, zsh, or fish. The selected shell determines quoting, variable expansion, redirection, and other command-language behavior.

Users must be able to select an installed shell, including fish. Use a pipeline shell with optional per-command override; default to `/bin/sh` for reproducibility. An explicitly configured unavailable shell must produce a clear configuration or launch error, never silently fall back to another interpreter.

The shell invocation contract, startup-file behavior, and shell-argument syntax belong to technical design. Shell selection does not imply a persistent shell session: changing directory or exporting variables in one command does not modify later commands.

## Concurrency and ordering

By default, all commands in the active stage are eligible to run concurrently. A stage containing five commands runs five concurrent jobs when no limit is configured. There is no default CPU-based or fixed numerical cap.

Users may configure a pipeline concurrency limit and override it for an individual stage. The effective limit is the stage value when set, otherwise the pipeline value, otherwise the number of commands in the stage. Limits must be positive integers.

When capacity is limited, commands are launched in registration order as slots become available. Forkstr must never exceed the effective limit. Actual process progress, output arrival, and completion order are not deterministic.

Terminal size must not silently change execution concurrency. Concurrent commands share the machine and may share files, ports, or other resources; Forkstr does not isolate these resources.

## Failure policies

All MVP commands are required. Exit code zero means success. A nonzero exit, unexpected termination by signal, or inability to launch means failure. A process terminated by Forkstr for cancellation is reported as cancelled instead.

Failure behavior must be configurable for the pipeline, with an optional stage override. The default is fail fast.

### Fail fast

When Forkstr observes a command failure, it must:

1. Record the failed command and its reason.
2. Stop launching queued commands in the active stage.
3. Terminate other running commands in that stage.
4. Mark queued commands as not started.
5. Skip all subsequent stages.
6. Produce an unsuccessful pipeline result and final report.

A command may have started before a concurrent failure was observed. Commands that already completed retain their actual outcomes.

### Finish current stage

Forkstr continues launching queued commands and allows all commands in the active stage to finish. If any command fails, that stage fails and subsequent stages are skipped. If all commands succeed, the next stage starts.

### Run entire pipeline after the MVP

A future policy may allow later stages to execute despite command failures. This must not turn failures into success: the pipeline remains unsuccessful if any required command fails. User cancellation and Forkstr infrastructure errors still stop execution.

## Live output and interaction

In an interactive terminal, Forkstr reserves a separate pane for every command in the active stage, in registration order. Queued commands have placeholders. Completed commands retain their final output and status until the entire stage finishes, including cleanup. Panes are replaced together when the next stage starts.

Required layouts are horizontal, vertical, and automatic grid. Horizontal means side by side; vertical means stacked. The layout adapts to terminal resizing and transitions when the next stage begins.

If all panes cannot fit readably, users must be able to reach every command pane, including completed commands, using a compact view or navigation. Hidden output is still captured, and commands continue running. The exact small-window presentation is a technical and UX design decision.

Users can enlarge the selected pane to fill the available terminal pane area, switch between panes while enlarged, and restore the configured layout. Other commands continue executing and capturing output. Each new stage restores the configured layout. Running panes use blue borders, succeeded panes green, failed panes red, cancelled panes yellow, and queued/skipped panes gray. Selection overrides the border color with magenta; status text remains visible. Color-disabled modes remain supported.

Focused-pane interaction is included in the MVP. Users can select one running command and send keyboard input to it while all other commands keep running and their output remains observable. Input must go only to the selected command, never to all commands at once.

The MVP interaction target is ordinary prompts and line-oriented input, including password prompts whose terminal echo behavior must be respected. Full-screen editors and terminal applications are not guaranteed in the MVP.

Forkstr must visibly identify input focus. When a command exits, input must not silently transfer to a new command that reuses its pane. Users explicitly select the next recipient. A command waiting for input occupies its normal execution slot; Forkstr need not automatically detect prompts.

Users need distinct, documented actions to switch focus, send an interrupt to the focused command, and cancel the whole pipeline. The technical requirements define observe/input modes and the bindings that distinguish command interruption from pipeline cancellation. Interrupting a focused command does not exempt its result from stage failure policy.

## Output fidelity and capture

Forkstr captures output from every command and displays it live without requiring newline-terminated output. ANSI colors and styling should remain visible where practical. Progress updates and common terminal controls should affect only the originating command's region.

Terminal-aware commands should receive terminal-like behavior when needed for output and interaction. The technical design must evaluate PTY support; this document does not select a terminal library or backend.

Preserve complete captured output under normal operation, including output from failed and cancelled commands. Capturing must continue during shutdown while output remains available. Forkstr must not silently discard output to keep the UI responsive. Resource and storage failures must be reported explicitly.

Memory usage should remain bounded as command output grows. The mechanism, persistent log retention, and any explicit storage limits are technical design decisions.

Full terminal emulation and exact reproduction of every historical screen state are outside the MVP guarantee. A readable sequential transcript may normalize cursor movements and progress updates. The technical requirements must distinguish complete byte capture from the readable final transcript and specify how complete capture can be accessed when normalization changes the presentation.

Do not promise exact ordering between separately captured stdout and stderr streams. Preserve each stream's order and describe any merging behavior.

## Noninteractive execution

When pane rendering is unavailable, use plain live output that identifies the stage and command responsible for each output record. Emit explicit stage and command lifecycle events, including starts, completions, failures, cancellation, and skipped work with reasons.

The plain view must make execution understandable without color, cursor movement, or an interactive UI. It must identify partial or interleaved output clearly. It uses the same scheduling, failure, and final-report semantics as the pane view.

Interactive input forwarding is unavailable in noninteractive execution. Children must not accidentally compete for Forkstr's standard input. A command requiring input may encounter EOF or fail; Forkstr must not claim it can reliably detect every prompt or prevent all prompt-related hangs.

The technical requirements define stdout and stderr routing, color detection, and output-mode override flags.

## Cancellation and process cleanup

Users can cancel the entire pipeline. Cancellation prevents further command launches, terminates running command trees, records unfinished work, restores the terminal, and attempts to produce the final report.

Provide a bounded graceful termination period followed by forced termination. A repeated cancellation action must allow users to accelerate shutdown. Cancellation and failure cleanup must handle ordinary child processes spawned by a command, rather than terminating only the immediate shell.

Commands are expected to be foreground jobs. Intentional daemonization and descendants escaping ordinary process ownership are outside the MVP guarantee. Cleanup limitations must be documented; Forkstr must not promise absolute process-tree containment.

Output handles inherited by descendants must not cause an indefinite wait after command termination. Exact shutdown timing and platform mechanisms belong to technical requirements.

## Status and final report

While running, Forkstr shows the active stage name and position, completed stages, and running, queued, succeeded, failed, or cancelled command counts. This basic progress information is part of the MVP. A graphical progress bar and time estimates are deferred.

Final pipeline statuses are succeeded, failed, and cancelled. Final stage statuses are succeeded, failed, cancelled, and skipped. Final command statuses are succeeded, failed, cancelled, and not started. Reasons are recorded separately from statuses.

A failed command causes its stage to fail even if peers are cancelled by fail fast. Later stages are skipped, and their commands are not started. User cancellation marks the active unfinished stage cancelled unless it already has a recorded failure; completed stages and commands retain their outcomes. The pipeline is reported as cancelled when explicitly interrupted, with recorded failures still visible.

Forkstr always leaves a concise status summary. Users can also request captured output for every command in one combined report; printing this combined output must be configurable. The default is to print it.

The combined report is ordered by stage registration, then command registration, regardless of completion order. Every command appears, including commands with no output and commands that never started. Include failure or skip reasons and command exit information where available.

Preserve ANSI colors and styles in the report where practical, while ensuring command output cannot erase report headers or leak styling into another command's section. Plain output must remain readable without ANSI support.

Forkstr exits zero only when all required commands and stages succeed. Command failures, cancellation, invalid configuration, and internal execution errors produce nonzero exits. Exact nonzero code assignments are deferred to technical requirements.

## Configuration and CLI requirements

The MVP needs an explicit ordered configuration for stages and commands. It must support pipeline and command working directories, inherited environment with pipeline and command overrides, shell selection, optional concurrency limits, layout, failure policy, and combined-report control.

Users need a way to run a configuration and validate it without executing commands. Validate the complete configuration before any command starts where possible. Empty pipelines or stages, invalid limits, empty command strings, and ambiguous or unknown fields must produce actionable errors. Runtime launch failures must identify the affected stage and command.

Use the TOML version 1 schema and CLI defined in the technical requirements. Use one command-list representation for both single-command and multi-command stages.

## MVP acceptance scenarios

1. A stage with five commands and no limit makes all five eligible to run concurrently; the following stage waits for all five to succeed and settle.
2. A stage with five commands and limit two never runs more than two commands at once and launches queued work in registration order.
3. Fail fast records the triggering failure, cancels active peers, does not launch queued work after observing the failure, and reports later stages as skipped.
4. Finish-current-stage executes all commands in that stage despite a failure, then skips subsequent stages.
5. A pipeline whose commands all succeed exits zero; a failed or cancelled pipeline exits nonzero.
6. The final report retains configuration order even when commands finish in reverse order, and includes commands with no output or no start.
7. Users can answer a prompt in a selected command while another command continues producing visible output. Input never leaks to another command when focus or pane ownership changes.
8. Pipeline cancellation cleans up ordinary descendants, handles commands that resist graceful termination, and restores the terminal.
9. Resizing or shrinking the terminal preserves capture and command access without altering concurrency.
10. Plain execution identifies each command's output and lifecycle, including why subsequent work was skipped.
11. A command receives inherited environment values and its own overrides without changing a sibling command's environment.
12. A configured installed shell is used; an unavailable shell gives a clear error. Working-directory overrides apply consistently.
13. Color, partial lines, progress updates, large output, and output emitted during cancellation are handled without cross-pane corruption or silent loss.

## Decisions resolved for the MVP

- Working directory defaults to the configuration directory; relative pipeline and command overrides follow the rules above.
- Shell defaults to `/bin/sh`, with pipeline and per-command overrides.
- Observe and input modes distinguish pipeline cancellation from command interruption; Ctrl-G returns to observe mode.
- Fail fast and combined-report printing are the defaults.
- TOML version 1 uses ordered stage and command-object arrays. CLI concurrency and failure overrides apply to every stage.
- Retained raw files preserve capture; a separate readable transcript normalizes terminal controls. The final summary prints the run-directory location.

See the [technical requirements](technical-requirements.md) for the complete implementation contracts.
