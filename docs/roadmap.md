# Forkstr scope and roadmap

Forkstr's initial MVP delivers sequential stages, concurrent commands, observable live output, and predictable failure handling on macOS and Linux. The scope below includes basic interaction with a selected command; the implemented terminal contract is captured in the technical requirements.

## Implemented MVP

| Area | Included behavior |
| --- | --- |
| Execution | Ordered stages, ordered command registration, stage barriers |
| Concurrency | Independent commands in a stage by default; optional job limits and named exclusive resources |
| Failure | Fail fast and finish current stage; explicit reasons for skipped work |
| Environment | Inherit the launching environment; pipeline and per-command overrides |
| Process context | Configurable shell and working directory, including per-command overrides |
| Live terminal | Separate command panes, horizontal/vertical/automatic layout, resizing |
| Interaction | Focus one running command, answer ordinary prompts, switch focus, interrupt a command, cancel the pipeline |
| Fidelity | Practical ANSI styling and progress handling; terminal-aware execution |
| Plain output | Attributed live output and explicit lifecycle events without a TTY |
| Progress | Active stage and position, completed work, command status counts |
| Capture and report | Complete capture under normal operation, configurable ordered transcript, mandatory concise status summary |
| Cleanup | Graceful then forced cancellation, ordinary descendant cleanup, terminal restoration |
| Configuration | One small ordered configuration; execution and validation entry points |

MVP acceptance scenarios and resolved defaults are in [Functional requirements](functional-requirements.md).

## Next release candidates

These are candidates, not commitments or automatic extensions of the MVP.

1. Run the entire pipeline despite command failures while retaining an unsuccessful aggregate result.
2. Add a graphical pipeline progress bar or stage timeline. Completed-stage or command counts must not be presented as an accurate estimate of remaining time.
3. Improve interactive terminal support beyond ordinary prompts, including compatibility testing for full-screen applications and richer input controls.
4. Add log browsing, explicit persistent capture export, and richer scrollback controls.
5. Consider command timeouts, stage-level environment overrides, and explicit removal of inherited environment variables if actual usage needs them.

## Outside the current product scope

Forkstr is not a CI system, build system, dependency resolver, arbitrary DAG executor, distributed task scheduler, or service supervisor. Retries, optional commands, conditional execution, and background-service readiness are not part of the MVP. Windows support is not currently planned.

## Ongoing development

The initial Rust implementation follows the [technical requirements](technical-requirements.md) and [implementation plan](implementation-plan.md). Preserve the functional acceptance scenarios and platform tests as follow-up features are added. cargo-nextest is the required unit and integration test runner.

The terminal and cleanup contracts require continued testing on both supported platforms. Full-screen application compatibility and deliberately detached descendants remain outside the MVP guarantee.
