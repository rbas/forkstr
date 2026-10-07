#![allow(clippy::unwrap_used)]
use forkstr::{
    config::{self, Overrides},
    model::Transport,
    process::ChildProcess,
};
use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn fixture(script: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let text = format!(
        "version=1\n[[stages]]\nname='example'\n[[stages.commands]]\nname='task'\nrun={}\n",
        toml::Value::String(script.to_owned())
    );
    fs::write(dir.path().join("forkstr.toml"), text).unwrap();
    dir
}
fn cli(dir: &tempfile::TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_forkstr"));
    command.current_dir(dir.path()).env("TMPDIR", dir.path());
    command
}
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[test]
fn plain_cli_reports_output_and_lifecycle() {
    let dir = fixture("printf 'hello\\n'; printf 'warning\\n' >&2");
    let output = cli(&dir).args(["run", "--ui", "plain"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("hello"));
    assert!(stdout.contains("warning"));
    assert!(stderr.contains("[example/task stdout] hello"));
    assert!(stderr.contains("[example/task stderr] warning"));
    assert!(!output.stdout.contains(&27));
    assert!(!output.stderr.contains(&27));
    assert!(stderr.contains("Captured logs:"));
}

#[test]
fn disabled_report_still_leaves_summary() {
    let dir = fixture("true");
    let output = cli(&dir)
        .args(["run", "--report", "never"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("task: succeeded"));
}

#[test]
fn validate_does_not_execute_and_bad_configuration_exits_two() {
    let dir = fixture("touch forbidden");
    assert!(cli(&dir).arg("validate").output().unwrap().status.success());
    assert!(!dir.path().join("forbidden").exists());
    fs::write(dir.path().join("forkstr.toml"), "version=2\nstages=[]").unwrap();
    assert_eq!(
        cli(&dir).arg("run").output().unwrap().status.code(),
        Some(2)
    );
}

#[test]
fn command_failure_has_exit_one() {
    let dir = fixture("exit 42");
    let output = cli(&dir).arg("run").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("exit code 42"));
}

#[test]
fn signal_cancels_pipeline_and_leaves_report() {
    let dir = fixture("touch started; sleep 30");
    let mut child = cli(&dir)
        .arg("run")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir.path().join("started").exists() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("command never started");
        }
        thread::sleep(Duration::from_millis(5));
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(143),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("cancelled"));
}

#[test]
fn real_pane_input_and_resize_restore_terminal() {
    let dir = fixture("printf 'Your name: '; read name; printf 'Hello %s!\\n' \"$name\"");
    let script = format!(
        "stty -g > before; {} run --ui panes --log-dir logs; code=$?; stty -g > after; exit \"$code\"",
        quote(env!("CARGO_BIN_EXE_forkstr"))
    );
    let text = format!(
        "version=1\nenv={{ TERM='xterm-256color' }}\n[[stages]]\nname='outer'\n[[stages.commands]]\nrun={}\n",
        toml::Value::String(script)
    );
    let plan = config::parse(&text, dir.path(), &Overrides::default()).unwrap();
    let mut process = ChildProcess::spawn(&plan.stages[0].commands[0], Transport::Pty).unwrap();
    process.resize(30, 100).unwrap();
    let mut output = Vec::new();
    let mut screen = vt100::Parser::new(35, 110, 100);
    let mut sent = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for (_, bytes) in process.read_available().unwrap() {
            screen.process(&bytes);
            output.extend(bytes);
        }
        if !sent && screen.screen().contents().contains("│Your name:") {
            process.resize(35, 110).unwrap();
            process.queue_input(b"\rAlice\r").unwrap();
            sent = true;
        }
        process.flush_input().unwrap();
        let exit = process.observe_exit().unwrap();
        if process.settle().unwrap() {
            assert!(
                exit.unwrap().success(),
                "{}",
                String::from_utf8_lossy(&output)
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal interaction timed out: {}",
            String::from_utf8_lossy(&output)
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(sent);
    assert!(String::from_utf8_lossy(&output).contains("Hello Alice!"));
    assert_eq!(
        fs::read(dir.path().join("before")).unwrap(),
        fs::read(dir.path().join("after")).unwrap()
    );
}

#[test]
fn observe_mode_ctrl_c_cancels_and_restores_terminal() {
    let dir = fixture("printf 'Working'; sleep 30");
    let script = format!(
        "stty -g > before; {} run --ui panes --log-dir logs; code=$?; stty -g > after; exit \"$code\"",
        quote(env!("CARGO_BIN_EXE_forkstr"))
    );
    let text = format!(
        "version=1\nenv={{ TERM='xterm-256color' }}\n[[stages]]\nname='outer'\n[[stages.commands]]\nrun={}\n",
        toml::Value::String(script)
    );
    let plan = config::parse(&text, dir.path(), &Overrides::default()).unwrap();
    let mut process = ChildProcess::spawn(&plan.stages[0].commands[0], Transport::Pty).unwrap();
    process.resize(30, 100).unwrap();
    let mut output = Vec::new();
    let mut screen = vt100::Parser::new(35, 110, 100);
    let mut sent = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for (_, bytes) in process.read_available().unwrap() {
            screen.process(&bytes);
            output.extend(bytes);
        }
        if !sent && screen.screen().contents().contains("Working") {
            process.queue_input(&[3]).unwrap();
            sent = true;
        }
        process.flush_input().unwrap();
        let exit = process.observe_exit().unwrap();
        if process.settle().unwrap() {
            assert_eq!(exit.unwrap().description(), "exit code 130");
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(sent);
    assert_eq!(
        fs::read(dir.path().join("before")).unwrap(),
        fs::read(dir.path().join("after")).unwrap()
    );
}

#[test]
fn broken_report_sink_returns_infrastructure_error() {
    let dir = fixture("printf output");
    let mut child = cli(&dir)
        .arg("run")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("final report:"));
}

#[test]
fn capture_write_failure_is_reported_and_cancels_the_pipeline() {
    use std::os::unix::process::CommandExt;
    let dir = fixture("yes output | head -n 10000");
    let mut command = cli(&dir);
    command.args(["run", "--log-dir", "logs"]);
    // SAFETY: the post-fork callback uses only async-signal-safe libc calls.
    // Limit this test child only, and turn SIGXFSZ into a write error to exercise
    // Forkstr's capture-failure path rather than killing it in the kernel.
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: 4096,
                rlim_max: 4096,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            Ok(())
        });
    }
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("capture or cleanup"), "{stderr}");
}

#[test]
fn completed_panes_stay_until_the_next_stage() {
    let dir = tempfile::tempdir().unwrap();
    let mut config_text = String::from("version=1\n");
    for stage in 0..2 {
        config_text.push_str(&format!("[[stages]]\nname='stage{stage}'\n"));
        for command in 0..4 {
            let script = if command == 3 {
                format!(
                    "printf 'waiting{stage}'; n=0; while [ ! -f gate{stage} ] && [ $n -lt 500 ]; do sleep 0.02; n=$((n+1)); done; test -f gate{stage}"
                )
            } else {
                format!("printf 'output{stage}_{command}'")
            };
            config_text.push_str(&format!(
                "[[stages.commands]]\nname='job{command}'\nrun={}\n",
                toml::Value::String(script)
            ));
        }
    }
    fs::write(dir.path().join("forkstr.toml"), config_text).unwrap();
    let script = format!(
        "{} run --ui panes --color always --log-dir logs",
        quote(env!("CARGO_BIN_EXE_forkstr"))
    );
    let outer = format!(
        "version=1\nenv={{TERM='xterm-256color'}}\n[[stages]]\nname='outer'\n[[stages.commands]]\nrun={}\n",
        toml::Value::String(script)
    );
    let plan = config::parse(&outer, dir.path(), &Overrides::default()).unwrap();
    let mut process = ChildProcess::spawn(&plan.stages[0].commands[0], Transport::Pty).unwrap();
    process.resize(40, 120).unwrap();
    let mut screen = vt100::Parser::new(40, 120, 0);
    let mut verified_stages = 0;
    let mut zoom_step = 0;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        for (_, bytes) in process.read_available().unwrap() {
            screen.process(&bytes);
        }
        let contents = screen.screen().contents();
        if verified_stages == 0
            && zoom_step == 1
            && contents.contains("output0_0")
            && !contents.contains("output0_1")
        {
            process.queue_input(b"\t").unwrap();
            zoom_step = 2;
        } else if verified_stages == 0
            && zoom_step == 2
            && contents.contains("output0_1")
            && !contents.contains("output0_0")
        {
            process.queue_input(b"\x1b").unwrap();
            zoom_step = 3;
        }
        if verified_stages < 2
            && contents.contains(&format!("waiting{verified_stages}"))
            && (0..3).all(|command| {
                contents.contains(&format!("output{verified_stages}_{command}"))
                    && contents.contains(&format!("job{command} [succeeded]"))
            })
        {
            if verified_stages == 1 {
                assert!(!contents.contains("output0_"), "{contents}");
            }
            if verified_stages == 0 && zoom_step == 0 {
                let colors: Vec<_> = (0..40)
                    .flat_map(|row| {
                        (0..120).filter_map({
                            let screen = screen.screen();
                            move |col| screen.cell(row, col)
                        })
                    })
                    .map(|cell| cell.fgcolor())
                    .collect();
                assert!(
                    colors.contains(&vt100::Color::Idx(5)),
                    "selected border is magenta"
                );
                assert!(
                    colors.contains(&vt100::Color::Idx(2)),
                    "succeeded borders are green"
                );
                assert!(
                    colors.contains(&vt100::Color::Idx(4)),
                    "running border is blue"
                );
                process.queue_input(b"z").unwrap();
                zoom_step = 1;
            } else if verified_stages == 1 || zoom_step == 3 {
                fs::write(dir.path().join(format!("gate{verified_stages}")), "").unwrap();
                verified_stages += 1;
            }
        }
        process.flush_input().unwrap();
        let exit = process.observe_exit().unwrap();
        if process.settle().unwrap() {
            assert!(exit.unwrap().success(), "{contents}");
            break;
        }
        assert!(Instant::now() < deadline, "{contents}");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(verified_stages, 2);
}
