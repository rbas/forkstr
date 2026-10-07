use crate::{
    capture::{transcript_path, without_ansi},
    model::{CommandState, Pipeline, ReportMode, StageState},
    runner::RunResult,
};
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write},
};

pub fn write_report(
    plan: &Pipeline,
    result: &RunResult,
    output: &mut impl Write,
    color: bool,
) -> io::Result<()> {
    if plan.report == ReportMode::Never {
        return Ok(());
    }
    for (i, stage) in plan.stages.iter().enumerate() {
        writeln!(
            output,
            "\n===== Stage: {} [{}] =====",
            stage.name,
            result.state.stage(i).label()
        )?;
        if let StageState::Skipped(reason) = result.state.stage(i) {
            writeln!(output, "SKIPPED: {reason}")?;
        }
        for command in &stage.commands {
            writeln!(output, "\n--- {} ---", command.name)?;
            writeln!(output, "$ {}", without_ansi(command.script.as_bytes()))?;
            if let CommandState::Finished(outcome) = result.state.command(command.id) {
                writeln!(
                    output,
                    "{}{}",
                    outcome.label(),
                    outcome
                        .reason()
                        .map(|r| format!(": {r}"))
                        .unwrap_or_default()
                )?;
            }
            let path = transcript_path(&result.logs, command.id);
            match File::open(path) {
                Ok(file) => {
                    for line in BufReader::new(file).split(b'\n') {
                        let line = line?;
                        if color {
                            output.write_all(&line)?;
                            output.write_all(b"\x1b[0m\n")?;
                        } else {
                            writeln!(output, "{}", without_ansi(&line))?;
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    output.flush()
}

pub fn write_summary(
    plan: &Pipeline,
    result: &RunResult,
    output: &mut impl Write,
) -> io::Result<()> {
    writeln!(
        output,
        "\nForkstr: {} (exit {})",
        match result.exit_code {
            0 => "succeeded",
            130 | 143 => "cancelled",
            _ => "failed",
        },
        result.exit_code
    )?;
    for (i, stage) in plan.stages.iter().enumerate() {
        writeln!(
            output,
            "  {}: {}",
            stage.name,
            result.state.stage(i).label()
        )?;
        for command in &stage.commands {
            if let CommandState::Finished(outcome) = result.state.command(command.id) {
                writeln!(
                    output,
                    "    {}: {}{}",
                    command.name,
                    outcome.label(),
                    outcome
                        .reason()
                        .map(|r| format!(" — {r}"))
                        .unwrap_or_default()
                )?;
            }
        }
    }
    for error in &result.errors {
        writeln!(output, "  error: {error}")?;
    }
    writeln!(output, "Captured logs: {}", result.logs.display())?;
    output.flush()
}
