use clap::{Args, Parser, Subcommand};
use forkstr::{
    config::{self, Overrides},
    model::{ColorMode, FailurePolicy, PaneLayout, ReportMode, Transport, UiMode},
    report,
    runner::{self, Control, View},
    skill, terminal,
};
use std::{
    io::{self, IsTerminal, Write},
    num::NonZeroUsize,
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
};

#[derive(Parser)]
#[command(
    version,
    about = "Sequential stages, parallel commands, live terminal output"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Run(RunArgs),
    Validate {
        #[arg(short, long, default_value = "forkstr.toml")]
        config: PathBuf,
    },
    Skill(SkillArgs),
}

#[derive(Args)]
struct SkillArgs {
    #[command(subcommand)]
    command: SkillCommand,
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Export the bundled AI skill offline without configuring an agent.
    Export {
        /// New destination directory; its parent must already exist.
        directory: PathBuf,
    },
}

#[derive(Args)]
struct RunArgs {
    #[arg(short, long, default_value = "forkstr.toml")]
    config: PathBuf,
    #[arg(short, long)]
    jobs: Option<NonZeroUsize>,
    #[arg(long, value_enum)]
    failure: Option<FailurePolicy>,
    #[arg(long, value_enum)]
    layout: Option<PaneLayout>,
    #[arg(long, value_enum, default_value = "auto")]
    ui: UiMode,
    #[arg(long, value_enum, default_value = "auto")]
    transport: Transport,
    #[arg(long, value_enum)]
    report: Option<ReportMode>,
    #[arg(long, value_enum, default_value = "auto")]
    color: ColorMode,
    #[arg(long)]
    log_dir: Option<PathBuf>,
}

struct SignalGuard(Vec<signal_hook::SigId>);
impl SignalGuard {
    fn register(control: &Arc<Control>) -> io::Result<Self> {
        let mut guard = Self(Vec::new());
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            let control = Arc::clone(control);
            // SAFETY: the callback only performs lock-free atomic operations.
            guard.0.push(unsafe {
                signal_hook::low_level::register(signal, move || control.cancel(signal))
            }?);
        }
        Ok(guard)
    }
}
impl Drop for SignalGuard {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let _ = writeln!(io::stderr(), "forkstr: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<u8, Box<dyn std::error::Error>> {
    match cli.command {
        Action::Validate { config } => {
            let plan = config::load(&config, &Overrides::default())?;
            writeln!(
                io::stdout(),
                "Valid: {} stages, {} commands",
                plan.stages.len(),
                plan.stages.iter().map(|s| s.commands.len()).sum::<usize>()
            )?;
            Ok(0)
        }
        Action::Skill(SkillArgs {
            command: SkillCommand::Export { directory },
        }) => {
            let exported = skill::export(&directory)?;
            writeln!(
                io::stdout(),
                "Exported Forkstr skill to {}",
                exported.skill_directory.display()
            )?;
            writeln!(
                io::stdout(),
                "Review the complete skill folder, then import it using your agent's native workflow."
            )?;
            writeln!(io::stdout(), "No agent configuration was changed.")?;
            Ok(0)
        }
        Action::Run(args) => {
            let plan = config::load(
                &args.config,
                &Overrides {
                    jobs: args.jobs,
                    failure: args.failure,
                    layout: args.layout,
                    report: args.report,
                },
            )?;
            let (panes, transport) = terminal::resolve_modes(args.ui, args.transport)?;
            let logs = runner::prepare_logs(args.log_dir.as_deref())?;
            let control = Arc::new(Control::default());
            let view = Arc::new(View::new(&plan));
            let _signals = SignalGuard::register(&control)?;
            let (mut result, presentation) = std::thread::scope(|scope| {
                let worker =
                    scope.spawn(|| runner::execute(&plan, transport, &logs, &control, &view));
                let presentation =
                    terminal::present(&plan, panes, args.color, transport, &view, &control, || {
                        worker.is_finished()
                    });
                if presentation.is_err() {
                    control.cancel(libc::SIGINT);
                }
                (worker.join(), presentation)
            });
            let result = result.as_mut().map_err(|_| {
                io::Error::other("execution worker panicked; child cleanup attempted")
            })?;
            if let Err(error) = presentation {
                result.errors.push(format!("terminal output: {error}"));
                result.exit_code = 2;
            }
            if let Err(error) = report::write_report(
                &plan,
                result,
                &mut io::stdout().lock(),
                terminal::use_color(args.color, io::stdout().is_terminal()),
            ) {
                result.errors.push(format!("final report: {error}"));
                result.exit_code = 2;
            }
            report::write_summary(&plan, result, &mut io::stderr().lock())?;
            Ok(result.exit_code)
        }
    }
}
