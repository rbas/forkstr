# Forkstr configuration and operation

Use this reference when creating or editing `forkstr.toml`, choosing CLI options,
or helping a human operate Forkstr.

## Pipeline model

```toml
version = 1
jobs = 4
failure = "fail-fast" # or "finish-stage"
layout = "auto"       # auto, horizontal, or vertical
report = "always"     # always or never
shell = "/bin/sh"
cwd = "."

[env]
CI = "true"

[[stages]]
name = "quality"
jobs = 3
failure = "finish-stage"

[[stages.commands]]
name = "format"
run = "npm run format:check"

[[stages.commands]]
name = "lint"
run = "npm run lint"

[[stages.commands]]
name = "integration"
run = "npm run test:integration"
cwd = "services/api"
env = { TEST_DATABASE = "forkstr-tests" }
resources = ["test-database"]

[[stages]]
name = "package"

[[stages.commands]]
name = "build"
run = "npm run build"
```

Top-level `shell`, `cwd`, and `env` are inherited by commands. A command may
override `shell` and `cwd`; command environment values extend or replace inherited
values. Relative working directories resolve from the configuration file.

Top-level `jobs` limits concurrent commands in every stage unless a stage
overrides it. With no limit, all commands in the stage are eligible to start.
Top-level `failure` is inherited unless a stage overrides it.

Resource names are arbitrary logical labels. Commands sharing any label never
overlap. Use names that describe the constrained thing, such as `test-database`,
`port-3000`, `gpu`, `generated-client`, or `shared-build-cache`.

## Common shapes

- Put formatters, linters, independent test suites, audits, and documentation
  checks in one quality stage when they do not mutate shared state.
- Put packaging or deployment preparation in a later stage when it requires the
  entire quality stage to pass.
- Put long-running development services in one stage when they should run
  together and remain visible.
- Split tests by package, workspace, browser, or environment only when each shard
  is genuinely independent.
- If a compiler or package manager serializes work through a shared cache, either
  model that cache as a resource or configure isolated caches/output directories.
  Measure the tradeoff; isolated caches often cost disk space and slower cold runs.

## Commands and interface

Validate without running anything:

```sh
forkstr validate
forkstr validate --config path/to/pipeline.toml
```

Run interactively or override selected policy:

```sh
forkstr run
forkstr run --jobs 2 --failure finish-stage --layout vertical
forkstr run --config path/to/pipeline.toml --log-dir ./forkstr-logs
```

Use plain, pipe-based output when there is no interactive terminal:

```sh
forkstr run --ui plain --transport pipe --color never
```

In the pane UI, Tab and Shift-Tab change selection, `z` enlarges the selected
pane, Escape restores the layout, and Enter begins sending input to the selected
command. Ctrl-G stops input mode. Ctrl-C cancels the pipeline in observe mode and
interrupts the selected command in input mode.

Forkstr retains raw command output, event metadata, and a readable transcript in
its run log directory. Exit status is `0` for success, `1` for command failure,
`2` for configuration or infrastructure failure, `130` for SIGINT, and `143` for
SIGTERM.
