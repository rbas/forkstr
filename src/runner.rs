use crate::{
    capture::{self, Capture, OutputFilter},
    model::{CommandId, ExecutionState, FailurePolicy, Outcome, Pipeline, StageState, Transport},
    process::{ChildProcess, Exit},
};
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicI32, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

pub enum Request {
    Input(CommandId, Vec<u8>),
    Resize(CommandId, u16, u16),
}

#[derive(Default)]
pub struct Control {
    cancellations: AtomicUsize,
    signal: AtomicI32,
    requests: Mutex<VecDeque<Request>>,
}
impl Control {
    pub fn cancel(&self, signal: i32) {
        let _ = self
            .signal
            .compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
        self.cancellations.fetch_add(1, Ordering::SeqCst);
    }
    pub fn request(&self, request: Request) -> bool {
        let mut requests = lock(&self.requests);
        if requests.len() >= 64 {
            return false;
        }
        requests.push_back(request);
        true
    }
}

#[derive(Clone)]
pub struct Snapshot {
    pub state: ExecutionState,
    pub stage: Option<usize>,
    pub screens: HashMap<CommandId, vt100::Screen>,
    pub events: VecDeque<String>,
    pub omitted: usize,
    pub done: bool,
}

pub struct View(Mutex<Snapshot>);
impl View {
    pub fn new(plan: &Pipeline) -> Self {
        Self(Mutex::new(Snapshot {
            state: ExecutionState::new(plan),
            stage: None,
            screens: HashMap::new(),
            events: VecDeque::new(),
            omitted: 0,
            done: false,
        }))
    }
    pub fn take_snapshot(&self) -> Snapshot {
        let mut view = lock(&self.0);
        let snapshot = view.clone();
        view.events.clear();
        view.omitted = 0;
        snapshot
    }
    fn event(&self, message: String) {
        let mut view = lock(&self.0);
        // The live display may lag, but capture is independent and complete.
        if view.events.len() >= 256 {
            view.events.pop_front();
            view.omitted += 1;
        }
        view.events.push_back(message);
    }
    fn publish(&self, state: &ExecutionState, stage: Option<usize>) {
        let mut view = lock(&self.0);
        view.state = state.clone();
        view.stage = stage;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // These locks contain presentation data / input only, never domain state.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct RunResult {
    pub state: ExecutionState,
    pub exit_code: u8,
    pub logs: PathBuf,
    pub errors: Vec<String>,
}

struct Active {
    id: CommandId,
    process: ChildProcess,
    capture: Capture,
    screen: vt100::Parser,
    screen_filter: OutputFilter,
    outcome: Option<Outcome>,
    cancelled: Option<String>,
}

impl Active {
    fn observe(&mut self) -> io::Result<Option<Exit>> {
        let exit = self.process.observe_exit()?;
        if let Some(exit) = exit
            && self.outcome.is_none()
        {
            self.outcome = Some(if let Some(reason) = &self.cancelled {
                Outcome::Cancelled(format!("{reason}; {}", exit.description()))
            } else if exit.success() {
                Outcome::Succeeded
            } else {
                Outcome::Failed(exit.description())
            });
            return Ok(Some(exit));
        }
        Ok(None)
    }

    fn pump(&mut self, plan: &Pipeline, view: &View) -> io::Result<bool> {
        let spec = &plan.stages[self.id.stage].commands[self.id.command];
        let prefix = format!("{}/{}", plan.stages[self.id.stage].name, spec.name);
        for (stream, bytes) in self.process.read_available()? {
            let text = self.capture.ingest(stream, &bytes)?;
            self.screen.process(&self.screen_filter.filter(&bytes));
            live(view, &prefix, stream.label(), &text);
        }
        for (stream, bytes) in self.capture.partials() {
            live(
                view,
                &prefix,
                &format!("{} partial", stream.label()),
                &bytes,
            );
        }
        self.process.flush_input()?;
        let finished = self.process.settle()?;
        if finished {
            let tail = self.capture.finish()?;
            live(view, &prefix, "tail", &tail);
        }
        lock(&view.0)
            .screens
            .insert(self.id, self.screen.screen().clone());
        Ok(finished)
    }
}

fn live(view: &View, prefix: &str, stream: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        view.event(format!("[{prefix} {stream}] {line}"));
    }
}

/// Supervision runs independently of terminal writes. No child reads or input
/// writes block this loop; each child receives a bounded read budget per pass.
pub fn execute(
    plan: &Pipeline,
    transport: Transport,
    logs: &Path,
    control: &Arc<Control>,
    view: &Arc<View>,
) -> RunResult {
    let mut state = ExecutionState::new(plan);
    let mut errors = Vec::new();
    let mut pipeline_stop: Option<String> = None;
    let mut cancelled = 0;
    for (stage_index, stage) in plan.stages.iter().enumerate() {
        if let Some(reason) = &pipeline_stop {
            state.skip_from(stage_index, reason);
            break;
        }
        state.start_stage(stage_index);
        view.event(format!(
            "Stage {}/{}: {} started",
            stage_index + 1,
            plan.stages.len(),
            stage.name
        ));
        {
            let mut snapshot = lock(&view.0);
            snapshot.screens.clear();
            snapshot.state = state.clone();
            snapshot.stage = Some(stage_index);
        }
        let mut active: Vec<Active> = Vec::new();
        let mut next = 0;
        let mut halt: Option<String> = None;
        loop {
            let cancel_count = control.cancellations.load(Ordering::SeqCst);
            if cancel_count > 0 {
                cancelled = control.signal.load(Ordering::SeqCst);
                halt = Some(format!("pipeline cancelled by signal {cancelled}"));
            }
            let mut requests = std::mem::take(&mut *lock(&control.requests));
            while let Some(request) = requests.pop_front() {
                match request {
                    Request::Input(id, bytes) => {
                        if let Some(job) = active.iter_mut().find(|j| j.id == id) {
                            // Recheck exit before applying input queued by a stale UI.
                            if let Err(error) = job.observe() {
                                errors.push(format!("observe {id:?}: {error}"));
                            }
                            if let Err(error) = job.process.queue_input(&bytes) {
                                view.event(format!("Input rejected: {error}"));
                            }
                        }
                    }
                    Request::Resize(id, rows, cols) => {
                        if let Some(job) = active.iter_mut().find(|j| j.id == id) {
                            match job.process.resize(rows, cols) {
                                Ok(true) => {
                                    job.screen.screen_mut().set_size(rows.max(1), cols.max(1));
                                    if let Err(e) = job.capture.resize(rows, cols) {
                                        errors.push(format!("record resize: {e}"));
                                    }
                                }
                                Ok(false) => {}
                                Err(e) => errors.push(format!("resize: {e}")),
                            }
                        }
                    }
                }
            }
            // Observe every exit before deciding whether any slot may be filled.
            for job in &mut active {
                match job.observe() {
                    Ok(Some(_)) => {
                        state.settling(job.id);
                    }
                    Ok(None) => {}
                    Err(error) => errors.push(format!("observe {:?}: {error}", job.id)),
                }
                if matches!(job.outcome, Some(Outcome::Failed(_)))
                    && stage.failure == FailurePolicy::FailFast
                    && halt.is_none()
                {
                    halt = Some(format!(
                        "command {} failed in stage {}",
                        stage.commands[job.id.command].name, stage.name
                    ));
                }
            }
            if !errors.is_empty() {
                halt = Some("Forkstr infrastructure error".into());
            }
            if let Some(reason) = &halt {
                for job in &mut active {
                    if job.outcome.is_none() {
                        job.cancelled.get_or_insert_with(|| reason.clone());
                    }
                    state.settling(job.id);
                    if let Err(error) = job.process.stop(cancel_count > 1) {
                        errors.push(format!("terminate {:?}: {error}", job.id));
                    }
                }
                for spec in &stage.commands[next..] {
                    state.finish(spec.id, Outcome::NotStarted(reason.clone()));
                    view.event(format!(
                        "[{}/{}] not started: {reason}",
                        stage.name, spec.name
                    ));
                }
                next = stage.commands.len();
            }
            let mut index = 0;
            while index < active.len() {
                let result = active[index].pump(plan, view);
                match result {
                    Ok(false) => {
                        index += 1;
                    }
                    done => {
                        let mut job = active.remove(index);
                        let outcome = match done {
                            Err(error) => {
                                let message = format!("capture or cleanup {:?}: {error}", job.id);
                                errors.push(message.clone());
                                let _ = job.capture.finish();
                                Outcome::Failed(message)
                            }
                            Ok(_) => job.outcome.take().unwrap_or_else(|| {
                                Outcome::Failed("missing process exit status".into())
                            }),
                        };
                        view.event(format!(
                            "[{}/{}] {}{}",
                            stage.name,
                            stage.commands[job.id.command].name,
                            outcome.label(),
                            outcome
                                .reason()
                                .map(|r| format!(": {r}"))
                                .unwrap_or_default()
                        ));
                        state.finish(job.id, outcome);
                    }
                }
            }
            if !errors.is_empty() {
                halt = Some("Forkstr infrastructure error".into());
            }
            // Recheck cancellation immediately before launching each queued job.
            while halt.is_none() && next < stage.commands.len() && active.len() < stage.jobs.get() {
                if control.cancellations.load(Ordering::SeqCst) > 0 {
                    break;
                }
                let spec = &stage.commands[next];
                next += 1;
                let capture = match Capture::new(logs, spec.id) {
                    Ok(capture) => capture,
                    Err(error) => {
                        errors.push(format!("create capture: {error}"));
                        state.finish(spec.id, Outcome::Failed(format!("create capture: {error}")));
                        halt = Some("Forkstr infrastructure error".into());
                        break;
                    }
                };
                match ChildProcess::spawn(spec, transport) {
                    Ok(process) => {
                        state.start(spec.id);
                        view.event(format!("[{}/{}] started", stage.name, spec.name));
                        active.push(Active {
                            id: spec.id,
                            process,
                            capture,
                            screen: vt100::Parser::new(24, 80, 0),
                            screen_filter: OutputFilter::default(),
                            outcome: None,
                            cancelled: None,
                        });
                    }
                    Err(error) => {
                        let reason = format!("launch failed: {error}");
                        state.finish(spec.id, Outcome::Failed(reason.clone()));
                        view.event(format!("[{}/{}] failed: {reason}", stage.name, spec.name));
                        if stage.failure == FailurePolicy::FailFast {
                            halt = Some(reason);
                        }
                    }
                }
            }
            view.publish(&state, Some(stage_index));
            if active.is_empty() && next == stage.commands.len() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        state.finish_stage(stage_index, cancelled != 0 || !errors.is_empty());
        view.event(format!(
            "Stage {} {}",
            stage.name,
            state.stage(stage_index).label()
        ));
        if !matches!(state.stage(stage_index), StageState::Succeeded) {
            pipeline_stop = Some(halt.unwrap_or_else(|| format!("stage {} failed", stage.name)));
        }
    }
    if let Some(reason) = &pipeline_stop {
        for (i, stage) in plan.stages.iter().enumerate() {
            if matches!(state.stage(i), StageState::Skipped(_)) {
                view.event(format!("Stage {} skipped: {reason}", stage.name));
                for command in &stage.commands {
                    view.event(format!(
                        "[{}/{}] not started: {reason}",
                        stage.name, command.name
                    ));
                }
            }
        }
    }
    let exit_code = if !errors.is_empty() {
        2
    } else if cancelled == libc::SIGTERM {
        143
    } else if cancelled != 0 {
        130
    } else if pipeline_stop.is_some() {
        1
    } else {
        0
    };
    view.publish(&state, None);
    lock(&view.0).done = true;
    RunResult {
        state,
        exit_code,
        logs: logs.to_owned(),
        errors,
    }
}

pub fn prepare_logs(parent: Option<&Path>) -> io::Result<PathBuf> {
    capture::run_directory(parent)
}
