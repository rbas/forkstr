#![allow(clippy::unwrap_used)]
use forkstr::{
    config::{self, Overrides},
    model::FailurePolicy,
};
use std::{ffi::OsStr, num::NonZeroUsize};

const VALID: &str = "version=1\n[[stages]]\nname='checks'\n[[stages.commands]]\nrun='true'\n[[stages.commands]]\nrun='true'\n";

#[test]
fn grouped_command_arrays_are_equivalent_to_command_tables() {
    let grouped = r#"
version = 1
[[stages]]
name = "checks"
commands = [
  { name = "format", run = "true" },
  { name = "tests", run = "true", env = { MODE = "ci" }, resources = ["database"] },
]
"#;
    let expanded = r#"
version = 1
[[stages]]
name = "checks"
[[stages.commands]]
name = "format"
run = "true"
[[stages.commands]]
name = "tests"
run = "true"
env = { MODE = "ci" }
resources = ["database"]
"#;
    let temp = tempfile::tempdir().unwrap();
    let grouped = config::parse(grouped, temp.path(), &Overrides::default()).unwrap();
    let expanded = config::parse(expanded, temp.path(), &Overrides::default()).unwrap();
    let grouped_stage = &grouped.stages[0];
    let expanded_stage = &expanded.stages[0];

    assert_eq!(grouped_stage.name, expanded_stage.name);
    assert_eq!(grouped_stage.jobs, expanded_stage.jobs);
    assert_eq!(grouped_stage.failure, expanded_stage.failure);
    for (grouped, expanded) in grouped_stage.commands.iter().zip(&expanded_stage.commands) {
        assert_eq!(grouped.id, expanded.id);
        assert_eq!(grouped.name, expanded.name);
        assert_eq!(grouped.script, expanded.script);
        assert_eq!(grouped.shell, expanded.shell);
        assert_eq!(grouped.cwd, expanded.cwd);
        assert_eq!(grouped.env, expanded.env);
        assert_eq!(grouped.resources, expanded.resources);
    }
}

#[test]
fn defaults_to_all_commands_and_config_directory() {
    let temp = tempfile::tempdir().unwrap();
    let plan = config::parse(VALID, temp.path(), &Overrides::default()).unwrap();
    assert_eq!(plan.stages[0].jobs.get(), 2);
    assert_eq!(
        plan.stages[0].commands[0].cwd,
        temp.path().canonicalize().unwrap()
    );
    assert_eq!(
        plan.stages[0].commands[0].shell,
        std::path::Path::new("/bin/sh").canonicalize().unwrap()
    );
    assert!(plan.stages[0].commands[0].resources.is_empty());
}

#[test]
fn command_resources_are_preserved() {
    let text = VALID.replacen(
        "run='true'",
        "run='true'\nresources=['cargo-target', 'database']",
        1,
    );
    let temp = tempfile::tempdir().unwrap();
    let plan = config::parse(&text, temp.path(), &Overrides::default()).unwrap();
    assert_eq!(
        plan.stages[0].commands[0].resources,
        ["cargo-target", "database"]
    );
}

#[test]
fn cli_overrides_stage_overrides_global() {
    let text = VALID.replace("version=1", "version=1\njobs=1").replace(
        "name='checks'",
        "name='checks'\njobs=3\nfailure='finish-stage'",
    );
    let temp = tempfile::tempdir().unwrap();
    let plan = config::parse(&text, temp.path(), &Overrides::default()).unwrap();
    assert_eq!(plan.stages[0].jobs.get(), 3);
    assert_eq!(plan.stages[0].failure, FailurePolicy::FinishStage);
    let overrides = Overrides {
        jobs: NonZeroUsize::new(2),
        failure: Some(FailurePolicy::FailFast),
        ..Overrides::default()
    };
    let plan = config::parse(&text, temp.path(), &overrides).unwrap();
    assert_eq!(plan.stages[0].jobs.get(), 2);
    assert_eq!(plan.stages[0].failure, FailurePolicy::FailFast);
}

#[test]
fn environment_and_relative_cwd_are_isolated() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("project/sub")).unwrap();
    let text = "version=1\ncwd='project'\nenv={ FORKSTR_TEST_VALUE='global' }\n[[stages]]\nname='a'\n[[stages.commands]]\nrun='true'\ncwd='sub'\nenv={ FORKSTR_TEST_VALUE='local' }\n[[stages.commands]]\nrun='true'";
    let plan = config::parse(text, temp.path(), &Overrides::default()).unwrap();
    let commands = &plan.stages[0].commands;
    assert_eq!(commands[0].env[OsStr::new("FORKSTR_TEST_VALUE")], "local");
    assert_eq!(commands[1].env[OsStr::new("FORKSTR_TEST_VALUE")], "global");
    assert_eq!(
        commands[0].cwd,
        temp.path().join("project/sub").canonicalize().unwrap()
    );
    assert_eq!(
        commands[1].cwd,
        temp.path().join("project").canonicalize().unwrap()
    );
    assert_eq!(
        commands[0].env.get(OsStr::new("PATH")),
        std::env::var_os("PATH").as_ref()
    );
}

#[test]
fn rejects_invalid_configuration_before_execution() {
    let cases = [
        VALID.replace("version=1", "version=2"),
        VALID.replace("version=1", "version=1\njobs=0"),
        VALID.replace("version=1", "version=1\nunknown=true"),
        VALID.replace("run='true'", "run=' '"),
        VALID.replace("version=1", "version=1\nshell='forkstr-nonexistent-shell'"),
        VALID.replace("version=1", "version=1\nenv={ 'BAD=KEY'='x' }"),
        VALID.replace("run='true'", "name='duplicate'\nrun='true'"),
        VALID.replacen("run='true'", "run='true'\nresources=['']", 1),
        VALID.replacen("run='true'", "run='true'\nresources=['cargo', 'cargo']", 1),
        "version=1\nstages=[]".into(),
        "version=1\n[[stages]]\nname='empty'\ncommands=[]".into(),
    ];
    let temp = tempfile::tempdir().unwrap();
    for text in cases {
        assert!(
            config::parse(&text, temp.path(), &Overrides::default()).is_err(),
            "accepted {text}"
        );
    }
}
