#![allow(clippy::unwrap_used)]
use forkstr::{
    config::{self, Overrides},
    model::{CommandId, CommandState, Outcome, StageState, Transport},
    report,
    runner::{self, Control, View},
};
use std::{fs, sync::Arc};

fn config_text(policy: &str, jobs: Option<usize>, first: &[&str], second: &[&str]) -> String {
    let mut text = format!("version=1\nfailure='{policy}'\n");
    if let Some(jobs) = jobs {
        text.push_str(&format!("jobs={jobs}\n"));
    }
    for (index, commands) in [first, second].iter().enumerate() {
        if commands.is_empty() {
            continue;
        }
        text.push_str(&format!("[[stages]]\nname='stage{index}'\n"));
        for (i, script) in commands.iter().enumerate() {
            text.push_str(&format!(
                "[[stages.commands]]\nname='job{i}'\nrun={}\n",
                toml::Value::String(script.to_string())
            ));
        }
    }
    text
}

fn execute(temp: &tempfile::TempDir, text: &str) -> (forkstr::model::Pipeline, runner::RunResult) {
    let plan = config::parse(text, temp.path(), &Overrides::default()).unwrap();
    let logs = runner::prepare_logs(Some(temp.path())).unwrap();
    let result = runner::execute(
        &plan,
        Transport::Pipe,
        &logs,
        &Arc::new(Control::default()),
        &Arc::new(View::new(&plan)),
    );
    (plan, result)
}

#[test]
fn unlimited_stage_starts_every_command_before_any_finishes() {
    let temp = tempfile::tempdir().unwrap();
    let scripts: Vec<_> = (0..5).map(|i| format!("touch started{i}; n=0; while [ $(ls started* | wc -l) -lt 5 ]; do n=$((n+1)); [ $n -lt 300 ] || exit 90; sleep 0.01; done; touch done{i}")).collect();
    let refs: Vec<_> = scripts.iter().map(String::as_str).collect();
    let text = config_text(
        "fail-fast",
        None,
        &refs,
        &["test $(ls done* | wc -l) -eq 5"],
    );
    let (_, result) = execute(&temp, &text);
    assert_eq!(result.exit_code, 0, "{:?}", result.errors);
}

#[test]
fn stage_limit_and_launch_order_hold() {
    let temp = tempfile::tempdir().unwrap();
    let text = config_text(
        "fail-fast",
        Some(2),
        &[
            "touch first; n=0; while [ ! -e second ]; do n=$((n+1)); [ $n -lt 300 ] || exit 90; sleep 0.01; done; test ! -e third; touch firstdone",
            "touch second; n=0; while [ ! -e firstdone ]; do n=$((n+1)); [ $n -lt 300 ] || exit 90; sleep 0.01; done",
            "test -e firstdone; touch third",
        ],
        &["test -e third; test -e firstdone"],
    );
    let (_, result) = execute(&temp, &text);
    assert_eq!(result.exit_code, 0, "{:?}", result.errors);
}

#[test]
fn fail_fast_cancels_peers_and_skips_queued_and_later_commands() {
    let temp = tempfile::tempdir().unwrap();
    let text = config_text(
        "fail-fast",
        Some(2),
        &[
            "n=0; while [ ! -e peer ]; do n=$((n+1)); [ $n -lt 300 ] || exit 90; sleep 0.01; done; exit 7",
            "touch peer; sleep 30",
            "touch forbidden",
        ],
        &["touch later"],
    );
    let (_, result) = execute(&temp, &text);
    assert_eq!(result.exit_code, 1, "{:?}", result.errors);
    assert!(matches!(
        result.state.command(CommandId {
            stage: 0,
            command: 0
        }),
        CommandState::Finished(Outcome::Failed(_))
    ));
    assert!(matches!(
        result.state.command(CommandId {
            stage: 0,
            command: 1
        }),
        CommandState::Finished(Outcome::Cancelled(_))
    ));
    assert!(matches!(
        result.state.command(CommandId {
            stage: 0,
            command: 2
        }),
        CommandState::Finished(Outcome::NotStarted(_))
    ));
    assert!(matches!(result.state.stage(1), StageState::Skipped(_)));
    assert!(!temp.path().join("forbidden").exists());
    assert!(!temp.path().join("later").exists());
}

#[test]
fn finish_stage_runs_queued_work_but_stops_pipeline() {
    let temp = tempfile::tempdir().unwrap();
    let (_, result) = execute(
        &temp,
        &config_text(
            "finish-stage",
            Some(1),
            &["exit 3", "touch completed"],
            &["touch forbidden"],
        ),
    );
    assert_eq!(result.exit_code, 1);
    assert!(temp.path().join("completed").exists());
    assert!(!temp.path().join("forbidden").exists());
}

#[test]
fn report_order_does_not_follow_completion_order() {
    let temp = tempfile::tempdir().unwrap();
    let text = config_text(
        "fail-fast",
        None,
        &[
            "n=0; while [ ! -e fast ]; do n=$((n+1)); [ $n -lt 300 ] || exit 90; sleep 0.01; done; printf SLOW",
            "printf FAST; touch fast",
        ],
        &["true"],
    );
    let (plan, result) = execute(&temp, &text);
    assert_eq!(result.exit_code, 0, "{:?}", result.errors);
    let mut bytes = Vec::new();
    report::write_report(&plan, &result, &mut bytes, false).unwrap();
    let report = String::from_utf8(bytes).unwrap();
    assert!(report.find("--- job0 ---").unwrap() < report.find("--- job1 ---").unwrap());
    assert!(report.find("SLOW\n").unwrap() < report.find("FAST\n").unwrap());
    assert_eq!(fs::read(result.logs.join("1-1.raw")).unwrap(), b"SLOW");
}

#[test]
fn cancellation_before_start_runs_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let plan = config::parse(
        &config_text("fail-fast", None, &["touch forbidden"], &["true"]),
        temp.path(),
        &Overrides::default(),
    )
    .unwrap();
    let control = Arc::new(Control::default());
    control.cancel(libc::SIGINT);
    let result = runner::execute(
        &plan,
        Transport::Pipe,
        temp.path(),
        &control,
        &Arc::new(View::new(&plan)),
    );
    assert_eq!(result.exit_code, 130);
    assert!(!temp.path().join("forbidden").exists());
}

#[test]
fn capture_creation_failure_stops_execution_and_is_not_success() {
    let temp = tempfile::tempdir().unwrap();
    let plan = config::parse(
        &config_text("finish-stage", None, &["touch forbidden"], &["true"]),
        temp.path(),
        &Overrides::default(),
    )
    .unwrap();
    let logs = temp.path().join("missing-parent");
    let result = runner::execute(
        &plan,
        Transport::Pipe,
        &logs,
        &Arc::new(Control::default()),
        &Arc::new(View::new(&plan)),
    );
    assert_eq!(result.exit_code, 2);
    assert!(!result.errors.is_empty());
    assert!(!temp.path().join("forbidden").exists());
    assert!(matches!(result.state.stage(1), StageState::Skipped(_)));
}

#[test]
fn observed_failure_does_not_wait_for_descendant_output_eof() {
    let temp = tempfile::tempdir().unwrap();
    let text = config_text(
        "fail-fast",
        Some(1),
        &[
            "sh -c 'trap \"\" TERM; sleep 10' & exit 9",
            "touch forbidden",
        ],
        &[],
    );
    let (_, result) = execute(&temp, &text);
    assert_eq!(result.exit_code, 1, "{:?}", result.errors);
    assert!(!temp.path().join("forbidden").exists());
}
