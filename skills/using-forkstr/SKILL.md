---
name: using-forkstr
description: Create, review, and run Forkstr pipelines for quality gates, development services, or other command workflows that need staged ordering, parallel execution, visible output, and shared-resource coordination.
---

# Using Forkstr

Use Forkstr to turn commands a human already runs into an observable local
pipeline. Preserve the commands' behavior and required ordering; do not weaken
checks or invent project commands merely to increase concurrency.

## Build or revise a pipeline

1. Inspect the project's scripts, task runner, CI, and contributor documentation
   to find the real commands and their prerequisites.
2. Identify hard dependencies. Put commands that can make progress independently
   in the same stage; create a later stage only when it must wait for every
   command in the earlier stage.
3. Look for hidden contention before claiming work is parallel. Commands may
   share a build cache, generated directory, database, port, device, mutable
   fixture, or rate-limited external service.
4. Give commands that must not overlap the same `resources` name. Use separate
   working directories, caches, or output paths only when isolation is safe and
   the speed benefit justifies the extra setup, storage, or cold-start cost.
5. Set `jobs` when the machine or service needs a concurrency limit. Choose
   `fail-fast` when peers should be cancelled after a failure, or `finish-stage`
   when the rest of the current stage should finish before the pipeline stops.
6. Create or edit `forkstr.toml`, then run `forkstr validate`. Fix configuration
   errors before executing the commands.

Stages are barriers: they run in declaration order. Commands within the active
stage are eligible to run concurrently, subject to `jobs` and `resources`.
Commands waiting for a resource do not prevent an unrelated command from
starting, and a command holds all declared resources until it has fully settled.

Read [references/configuration.md](references/configuration.md) when creating a
configuration, selecting runtime options, or explaining Forkstr's UI to a user.

## Run and help the user

For an interactive human, use `forkstr run`. Explain that each command has a live
pane, complete output is retained, and cancelling Forkstr cleans up command
process groups. Mention pane controls only when they are useful to the task.

For an agent, CI log, or benchmark, prefer deterministic plain output:

```sh
forkstr run --ui plain --transport pipe --report never --color never
```

After changing a pipeline, verify that intended commands start, dependencies
remain ordered, resource conflicts do not overlap, failures follow the chosen
policy, and the complete gate succeeds. When optimizing runtime, compare the
same workload under equivalent warm or cold conditions and use multiple runs;
do not infer speedup merely because processes were launched concurrently.

Report missing tools as prerequisites instead of silently dropping commands.
