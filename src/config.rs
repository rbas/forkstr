use std::{
    collections::{BTreeMap, HashSet},
    ffi::{OsStr, OsString},
    fs,
    num::NonZeroUsize,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use crate::model::{
    CommandId, CommandSpec, FailurePolicy, PaneLayout, Pipeline, ReportMode, StageSpec,
};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read configuration {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Default, Debug)]
pub struct Overrides {
    pub jobs: Option<NonZeroUsize>,
    pub failure: Option<FailurePolicy>,
    pub layout: Option<PaneLayout>,
    pub report: Option<ReportMode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    shell: Option<String>,
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    jobs: Option<NonZeroUsize>,
    #[serde(default)]
    failure: FailurePolicy,
    #[serde(default)]
    layout: PaneLayout,
    #[serde(default)]
    report: ReportMode,
    stages: Vec<Stage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage {
    name: String,
    jobs: Option<NonZeroUsize>,
    failure: Option<FailurePolicy>,
    commands: Vec<Command>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    name: Option<String>,
    run: String,
    shell: Option<String>,
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

pub fn load(path: &Path, overrides: &Overrides) -> Result<Pipeline, ConfigError> {
    let path = fs::canonicalize(path).map_err(|source| ConfigError::Read {
        path: path.into(),
        source,
    })?;
    let text = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        path: path.clone(),
        source,
    })?;
    let base = path
        .parent()
        .ok_or_else(|| ConfigError::Invalid("configuration has no parent directory".into()))?;
    parse(&text, base, overrides)
}

pub fn parse(text: &str, base: &Path, overrides: &Overrides) -> Result<Pipeline, ConfigError> {
    let config: Config = toml::from_str(text)?;
    if config.version != 1 {
        return invalid("only configuration version 1 is supported");
    }
    if config.stages.is_empty() {
        return invalid("pipeline must contain at least one stage");
    }
    let cwd = directory(base, config.cwd.as_deref())?;
    let shell = config.shell.as_deref().unwrap_or("/bin/sh");
    let mut inherited: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    merge_env(&mut inherited, &config.env)?;
    let mut names = HashSet::new();
    let mut stages = Vec::new();
    for (stage_index, stage) in config.stages.into_iter().enumerate() {
        validate_name(&stage.name)?;
        if !names.insert(stage.name.clone()) {
            return invalid(format!("duplicate stage name: {}", stage.name));
        }
        let count = NonZeroUsize::new(stage.commands.len())
            .ok_or_else(|| ConfigError::Invalid(format!("stage {} has no commands", stage.name)))?;
        let mut command_names = HashSet::new();
        let mut commands = Vec::new();
        for (command_index, command) in stage.commands.into_iter().enumerate() {
            let name = command
                .name
                .unwrap_or_else(|| format!("command-{}", command_index + 1));
            validate_name(&name)?;
            if !command_names.insert(name.clone()) {
                return invalid(format!("duplicate command name in {}: {name}", stage.name));
            }
            if command.run.trim().is_empty() || command.run.contains('\0') {
                return invalid(format!(
                    "{}/{name}: command must be nonempty and contain no NUL",
                    stage.name
                ));
            }
            let command_cwd = directory(&cwd, command.cwd.as_deref())?;
            let mut env = inherited.clone();
            merge_env(&mut env, &command.env)?;
            let resolved_shell = resolve_shell(
                command.shell.as_deref().unwrap_or(shell),
                &command_cwd,
                &env,
            )?;
            commands.push(CommandSpec {
                id: CommandId {
                    stage: stage_index,
                    command: command_index,
                },
                name,
                script: command.run,
                shell: resolved_shell,
                cwd: command_cwd,
                env,
            });
        }
        stages.push(StageSpec {
            name: stage.name,
            commands,
            jobs: overrides
                .jobs
                .or(stage.jobs)
                .or(config.jobs)
                .unwrap_or(count),
            failure: overrides
                .failure
                .or(stage.failure)
                .unwrap_or(config.failure),
        });
    }
    Ok(Pipeline {
        stages,
        layout: overrides.layout.unwrap_or(config.layout),
        report: overrides.report.unwrap_or(config.report),
    })
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::Invalid(message.into()))
}

fn validate_name(name: &str) -> Result<(), ConfigError> {
    if name.trim().is_empty() || name.chars().any(char::is_control) {
        return invalid("names must be nonempty and contain no control characters");
    }
    Ok(())
}

fn directory(base: &Path, relative: Option<&Path>) -> Result<PathBuf, ConfigError> {
    let path = relative.map_or_else(|| base.to_path_buf(), |p| base.join(p));
    let path = fs::canonicalize(&path)
        .map_err(|e| ConfigError::Invalid(format!("working directory {}: {e}", path.display())))?;
    if !path.is_dir() {
        return invalid(format!("not a directory: {}", path.display()));
    }
    Ok(path)
}

fn merge_env(
    target: &mut BTreeMap<OsString, OsString>,
    values: &BTreeMap<String, String>,
) -> Result<(), ConfigError> {
    for (key, value) in values {
        if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
            return invalid(format!("invalid environment entry {key:?}"));
        }
        target.insert(key.into(), value.into());
    }
    Ok(())
}

fn resolve_shell(
    shell: &str,
    cwd: &Path,
    env: &BTreeMap<OsString, OsString>,
) -> Result<PathBuf, ConfigError> {
    if shell.is_empty() || shell.contains('\0') {
        return invalid("shell must be a nonempty executable name or path");
    }
    let candidates: Vec<PathBuf> = if shell.contains('/') {
        vec![cwd.join(shell)]
    } else {
        std::env::split_paths(
            env.get(OsStr::new("PATH"))
                .map(OsString::as_os_str)
                .unwrap_or(OsStr::new("/usr/bin:/bin")),
        )
        .map(|p| cwd.join(p).join(shell))
        .collect()
    };
    for path in candidates {
        if fs::metadata(&path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0) {
            return fs::canonicalize(path)
                .map_err(|e| ConfigError::Invalid(format!("cannot resolve shell {shell}: {e}")));
        }
    }
    invalid(format!("shell {shell:?} is unavailable or not executable"))
}
