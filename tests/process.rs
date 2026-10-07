#![allow(clippy::unwrap_used)]
use forkstr::{
    config::{self, Overrides},
    model::Transport,
    process::ChildProcess,
};
use std::{
    thread,
    time::{Duration, Instant},
};

fn child(script: &str, transport: Transport) -> ChildProcess {
    let text = format!(
        "version = 1\n[[stages]]\nname = 'test'\n[[stages.commands]]\nrun = {}\n",
        toml::Value::String(script.to_owned())
    );
    let plan = config::parse(
        &text,
        &std::env::current_dir().unwrap(),
        &Overrides::default(),
    )
    .unwrap();
    ChildProcess::spawn(&plan.stages[0].commands[0], transport).unwrap()
}

fn until(process: &mut ChildProcess, output: &mut Vec<u8>, predicate: impl Fn(&[u8]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate(output) {
        for (_, bytes) in process.read_available().unwrap() {
            output.extend(bytes);
        }
        process.flush_input().unwrap();
        assert!(
            Instant::now() < deadline,
            "timeout: {}",
            String::from_utf8_lossy(output)
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn finish(process: &mut ChildProcess) -> (bool, String) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut output = Vec::new();
    loop {
        for (_, bytes) in process.read_available().unwrap() {
            output.extend(bytes);
        }
        let exit = process.observe_exit().unwrap();
        if process.settle().unwrap() {
            return (
                exit.unwrap().success(),
                String::from_utf8_lossy(&output).into_owned(),
            );
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn pipe_preserves_stdout_stderr_and_status() {
    let mut process = child("printf output; printf error >&2; exit 7", Transport::Pipe);
    let (success, output) = finish(&mut process);
    assert!(!success);
    assert!(output.contains("output"));
    assert!(output.contains("error"));
}

#[test]
fn pty_supports_prompts_without_echoing_passwords() {
    let mut process = child(
        "stty -echo; printf 'secret: '; read secret; stty echo; printf '\nreceived:%s\n' \"${#secret}\"",
        Transport::Pty,
    );
    let mut output = Vec::new();
    until(&mut process, &mut output, |b| {
        String::from_utf8_lossy(b).contains("secret:")
    });
    process.queue_input(b"swordfish\n").unwrap();
    process.flush_input().unwrap();
    let (success, tail) = finish(&mut process);
    assert!(success);
    assert!(tail.contains("received:9"));
    assert!(!tail.contains("swordfish"));
}

#[test]
fn pty_resize_reaches_child() {
    let mut process = child("printf ready; read line; stty size", Transport::Pty);
    let mut output = Vec::new();
    until(&mut process, &mut output, |b| {
        String::from_utf8_lossy(b).contains("ready")
    });
    process.resize(17, 63).unwrap();
    process.queue_input(b"\n").unwrap();
    process.flush_input().unwrap();
    let (success, output) = finish(&mut process);
    assert!(success);
    assert!(output.contains("17 63"), "{output}");
}

#[test]
fn interrupt_character_reaches_foreground_command() {
    let mut process = child("printf ready; read answer", Transport::Pty);
    let mut output = Vec::new();
    until(&mut process, &mut output, |b| {
        String::from_utf8_lossy(b).contains("ready")
    });
    process.queue_input(&[3]).unwrap();
    process.flush_input().unwrap();
    assert!(!finish(&mut process).0);
}

#[test]
fn cancellation_kills_descendants_that_hold_output_open() {
    let mut process = child(
        "trap '' TERM; sleep 30 & printf ready; wait",
        Transport::Pipe,
    );
    let mut output = Vec::new();
    until(&mut process, &mut output, |b| {
        String::from_utf8_lossy(b).contains("ready")
    });
    process.stop(false).unwrap();
    assert!(!finish(&mut process).0);
}
