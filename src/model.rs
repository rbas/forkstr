use std::{collections::BTreeMap, ffi::OsString, num::NonZeroUsize, path::PathBuf};

use clap::ValueEnum;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CommandId {
    pub stage: usize,
    pub command: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, ValueEnum, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum FailurePolicy {
    #[default]
    FailFast,
    FinishStage,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, ValueEnum, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PaneLayout {
    #[default]
    Auto,
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, ValueEnum, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ReportMode {
    #[default]
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum, Eq, PartialEq)]
pub enum UiMode {
    #[default]
    Auto,
    Panes,
    Plain,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum, Eq, PartialEq)]
pub enum Transport {
    #[default]
    Auto,
    Pty,
    Pipe,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum, Eq, PartialEq)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug)]
pub struct CommandSpec {
    pub id: CommandId,
    pub name: String,
    pub script: String,
    pub shell: PathBuf,
    pub cwd: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
    pub resources: Vec<String>,
}

#[derive(Debug)]
pub struct StageSpec {
    pub name: String,
    pub commands: Vec<CommandSpec>,
    pub jobs: NonZeroUsize,
    pub failure: FailurePolicy,
}

#[derive(Debug)]
pub struct Pipeline {
    pub stages: Vec<StageSpec>,
    pub layout: PaneLayout,
    pub report: ReportMode,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Outcome {
    Succeeded,
    Failed(String),
    Cancelled(String),
    NotStarted(String),
}

impl Outcome {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed(_) => "failed",
            Self::Cancelled(_) => "cancelled",
            Self::NotStarted(_) => "not started",
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Succeeded => None,
            Self::Failed(reason) | Self::Cancelled(reason) | Self::NotStarted(reason) => {
                Some(reason)
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CommandState {
    Queued,
    Running,
    Settling,
    Finished(Outcome),
}

impl CommandState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Settling => "settling",
            Self::Finished(outcome) => outcome.label(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StageState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Skipped(String),
}

impl StageState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped(_) => "skipped",
        }
    }
}

/// The coordinator is the only writer. Presentation receives shared snapshots.
#[derive(Debug, Clone)]
pub struct ExecutionState {
    commands: Vec<Vec<CommandState>>,
    stages: Vec<StageState>,
}

impl ExecutionState {
    pub fn new(plan: &Pipeline) -> Self {
        Self {
            commands: plan
                .stages
                .iter()
                .map(|s| vec![CommandState::Queued; s.commands.len()])
                .collect(),
            stages: vec![StageState::Pending; plan.stages.len()],
        }
    }

    pub fn command(&self, id: CommandId) -> &CommandState {
        &self.commands[id.stage][id.command]
    }

    pub fn stage(&self, index: usize) -> &StageState {
        &self.stages[index]
    }

    pub(crate) fn start_stage(&mut self, index: usize) {
        debug_assert_eq!(self.stages[index], StageState::Pending);
        self.stages[index] = StageState::Running;
    }

    pub(crate) fn start(&mut self, id: CommandId) {
        debug_assert_eq!(self.command(id), &CommandState::Queued);
        self.commands[id.stage][id.command] = CommandState::Running;
    }

    pub(crate) fn settling(&mut self, id: CommandId) {
        if matches!(self.command(id), CommandState::Running) {
            self.commands[id.stage][id.command] = CommandState::Settling;
        }
    }

    pub(crate) fn finish(&mut self, id: CommandId, outcome: Outcome) {
        debug_assert!(!matches!(self.command(id), CommandState::Finished(_)));
        self.commands[id.stage][id.command] = CommandState::Finished(outcome);
    }

    pub(crate) fn finish_stage(&mut self, index: usize, cancelled: bool) {
        self.stages[index] = if self.commands[index]
            .iter()
            .any(|s| matches!(s, CommandState::Finished(Outcome::Failed(_))))
        {
            StageState::Failed
        } else if cancelled {
            StageState::Cancelled
        } else {
            StageState::Succeeded
        };
    }

    pub(crate) fn skip_from(&mut self, index: usize, reason: &str) {
        for stage in index..self.stages.len() {
            self.stages[stage] = StageState::Skipped(reason.to_owned());
            for command in &mut self.commands[stage] {
                *command = CommandState::Finished(Outcome::NotStarted(reason.to_owned()));
            }
        }
    }
}
