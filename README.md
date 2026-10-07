# Forkstr

**Run independent commands together, dependent commands in order, and keep every
output stream visible.**

Local project checks tend to become either a slow shell script or a pile of
terminal tabs. A shell script hides concurrent output and stops being pleasant
when a command needs input; terminal tabs make ordering, failure handling, and
cleanup your problem. Forkstr turns those commands into a small staged pipeline:

- stages run from top to bottom;
- commands within a stage run concurrently;
- every command gets a live terminal pane and retained logs;
- one failure cancels or finishes the current stage according to your policy;
- Ctrl-C cleans up the command's process group, not just its top-level process.

Forkstr is a single Rust binary for macOS and Linux. It is useful for local test
suites, linters, development services, code generators, and any workflow that is
too structured for several terminal tabs but does not need a build system or CI
server.

## Install

Download an archive for your platform from
[GitHub Releases](https://github.com/rbas/forkstr/releases), unpack it, and put
`forkstr` somewhere on your `PATH`.

To build from source, install Rust 1.88 or newer and run:

```sh
cargo install --git https://github.com/rbas/forkstr --locked
```

## Quick start

Create `forkstr.toml` in your project:

```toml
version = 1
failure = "fail-fast"

[[stages]]
name = "static-checks"

[[stages.commands]]
name = "format"
run = "cargo fmt --check"

[[stages.commands]]
name = "docs"
run = "markdownlint '**/*.md'"

[[stages]]
name = "tests"

[[stages.commands]]
name = "unit"
run = "cargo nextest run --locked"
```

Then run:

```sh
forkstr validate
forkstr run
```

In this example formatting and documentation checks run together. Tests start
only after both succeed. Put commands in the same stage only when they can make
progress independently.

Cargo commands that share one target directory may wait on Cargo's build lock.
For that reason this repository places Clippy, Nextest, and the release build in
separate stages. Giving each command a different `CARGO_TARGET_DIR` removes the
lock, but recompiles dependencies and consumes more disk, so it is usually slower
for one package.

## Configuration

Forkstr reads `./forkstr.toml` by default. Relative working directories resolve
from the configuration file. Pipeline and command settings support `shell`,
`cwd`, and `env`; stages can override `jobs` and `failure`.

```toml
version = 1
jobs = 4
failure = "finish-stage" # or "fail-fast"
layout = "auto"          # auto, horizontal, or vertical
report = "always"        # always, failure, or never

[env]
APP_MODE = "development"

[[stages]]
name = "checks"
jobs = 2

[[stages.commands]]
name = "api"
run = "./scripts/check-api"

[[stages.commands]]
name = "web"
run = "npm test"
cwd = "web"
env = { CI = "true" }
```

Useful commands:

```sh
forkstr run --jobs 2 --failure finish-stage
forkstr run --layout vertical
forkstr run --ui plain --transport pipe --report never
forkstr run --log-dir ./logs --color never
forkstr run --help
```

Pane controls are **Tab** / **Shift-Tab** to select, **z** to enlarge, **Esc** to
restore the layout, and **Enter** to send input to a running PTY command. Press
**Ctrl-G** to stop sending input. In observe mode, **Ctrl-C** cancels the whole
pipeline; in input mode it interrupts the selected command.

Each run stores complete raw output, event metadata, and a readable ANSI
transcript in a private log directory. The final report preserves configuration
order even when commands finish in another order. Exit codes are `0` for
success, `1` for a command failure, `2` for configuration/infrastructure errors,
`130` for SIGINT, and `143` for SIGTERM.

## Development and releases

Install [just](https://github.com/casey/just),
[cargo-nextest](https://nexte.st/), and [convco](https://convco.github.io/), then:

```sh
just check                 # format, Clippy, tests, release build
just next-version          # infer the next version from conventional commits
just bump                  # update manifest, lockfile, and CHANGELOG.md
just bump minor            # force patch, minor, or major
just release               # alternatively: bump, commit, and tag in one step
git push origin main --follow-tags
```

Use [Conventional Commits](https://www.conventionalcommits.org/): `fix:` bumps
the patch version, `feat:` bumps the minor version, and a breaking change bumps
the major version. Before `1.0.0`, Convco's default pre-major rules keep features
on patch releases and breaking changes on minor releases. A pushed `v*` tag
starts the release workflow and attaches platform archives plus SHA-256 checksums
to a GitHub Release.

The project is tested with formatting, Clippy, and Nextest on macOS and Linux.
See [CHANGELOG.md](CHANGELOG.md), [the roadmap](docs/roadmap.md), and the
[technical requirements](docs/technical-requirements.md) for deeper detail.

## Current limits

Full-screen terminal applications, terminal-query protocols, shell job control,
and deliberately detached processes are not guaranteed. Windows is not yet
supported. Forkstr is intentionally not a dependency resolver, distributed task
scheduler, or CI system.
