use crate::{
    capture::without_ansi,
    model::{ColorMode, CommandId, CommandState, Outcome, PaneLayout, Pipeline, Transport, UiMode},
    runner::{Control, Request, Snapshot, View},
};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Widget},
};
use std::{
    io::{self, IsTerminal, Stderr, Write},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub fn resolve_modes(ui: UiMode, transport: Transport) -> io::Result<(bool, Transport)> {
    let usable = io::stdin().is_terminal()
        && io::stderr().is_terminal()
        && std::env::var("TERM").is_ok_and(|t| !t.is_empty() && t != "dumb");
    let panes = match ui {
        UiMode::Auto => usable,
        UiMode::Plain => false,
        UiMode::Panes if usable => true,
        UiMode::Panes => {
            return Err(io::Error::other(
                "pane mode requires terminal stdin, terminal stderr, and a usable TERM",
            ));
        }
    };
    let transport = match transport {
        Transport::Auto if panes => Transport::Pty,
        Transport::Auto => Transport::Pipe,
        other => other,
    };
    Ok((panes, transport))
}

pub fn use_color(mode: ColorMode, terminal: bool) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => terminal && std::env::var_os("NO_COLOR").is_none(),
    }
}

pub fn present(
    plan: &Pipeline,
    panes: bool,
    color: ColorMode,
    transport: Transport,
    view: &Arc<View>,
    control: &Arc<Control>,
    finished: impl Fn() -> bool,
) -> io::Result<()> {
    if panes {
        present_panes(
            plan,
            transport,
            view,
            control,
            finished,
            use_color(color, true),
        )
    } else {
        let colors = use_color(color, io::stderr().is_terminal());
        let mut stderr = io::stderr().lock();
        loop {
            let snapshot = view.take_snapshot();
            for event in &snapshot.events {
                if colors {
                    writeln!(stderr, "{event}\x1b[0m")?;
                } else {
                    writeln!(stderr, "{}", without_ansi(event.as_bytes()))?;
                }
            }
            if snapshot.omitted > 0 {
                writeln!(
                    stderr,
                    "[forkstr] {} live records omitted because the display fell behind; complete output is captured in the run logs",
                    snapshot.omitted
                )?;
            }
            stderr.flush()?;
            if snapshot.done || finished() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
}

struct ScreenGuard {
    terminal: Terminal<CrosstermBackend<Stderr>>,
}
impl ScreenGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let result = (|| {
            let mut stderr = io::stderr();
            execute!(stderr, EnterAlternateScreen, EnableBracketedPaste)?;
            let terminal = Terminal::new(CrosstermBackend::new(stderr))?;
            Ok(Self { terminal })
        })();
        if result.is_err() {
            let _ = disable_raw_mode();
            let _ = execute!(io::stderr(), DisableBracketedPaste, LeaveAlternateScreen);
        }
        result
    }
}
impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

#[derive(Default)]
pub struct Focus {
    selected: Option<CommandId>,
    input: Option<CommandId>,
}
impl Focus {
    pub fn reconcile(&mut self, panes: &[CommandId], running: &[CommandId]) {
        if self.input.is_some_and(|id| !running.contains(&id)) {
            self.input = None;
        }
        if self.selected.is_none_or(|id| !panes.contains(&id)) {
            self.selected = panes.first().copied();
        }
    }
    pub fn cycle(&mut self, panes: &[CommandId], backward: bool) {
        if panes.is_empty() {
            return;
        }
        let index = self
            .selected
            .and_then(|id| panes.iter().position(|i| *i == id))
            .unwrap_or(0);
        self.selected =
            Some(panes[(index + if backward { panes.len() - 1 } else { 1 }) % panes.len()]);
    }
    pub fn enter(&mut self, running: &[CommandId]) {
        self.input = self.selected.filter(|id| running.contains(id));
    }
    pub fn leave(&mut self) {
        self.input = None;
    }
    pub fn input_target(&self) -> Option<CommandId> {
        self.input
    }
}

fn present_panes(
    plan: &Pipeline,
    transport: Transport,
    view: &Arc<View>,
    control: &Arc<Control>,
    finished: impl Fn() -> bool,
    colors: bool,
) -> io::Result<()> {
    crossterm::style::force_color_output(colors);
    let mut guard = ScreenGuard::enter()?;
    let mut focus = Focus::default();
    let mut notice = String::new();
    let mut enlarged = false;
    let mut displayed_stage = None;
    loop {
        let frame_started = Instant::now();
        let snapshot = view.take_snapshot();
        if displayed_stage != snapshot.stage {
            enlarged = false;
            displayed_stage = snapshot.stage;
        }
        if let Some(message) = snapshot
            .events
            .iter()
            .rev()
            .find(|e| e.starts_with("Input rejected:"))
        {
            notice = message.clone();
        }
        let running: Vec<_> = snapshot
            .stage
            .map(|i| {
                plan.stages[i]
                    .commands
                    .iter()
                    .filter(|c| matches!(snapshot.state.command(c.id), CommandState::Running))
                    .map(|c| c.id)
                    .collect()
            })
            .unwrap_or_default();
        let candidates: Vec<_> = snapshot
            .stage
            .map(|i| plan.stages[i].commands.iter().map(|c| c.id).collect())
            .unwrap_or_default();
        focus.reconcile(&candidates, &running);
        let mut sizes = Vec::new();
        guard.terminal.draw(|frame| {
            let area = frame.area();
            let body = Rect::new(
                area.x,
                area.y.saturating_add(3),
                area.width,
                area.height.saturating_sub(5),
            );
            let header = header(plan, &snapshot);
            frame.render_widget(
                Paragraph::new(header),
                Rect::new(area.x, area.y, area.width, area.height.min(3)),
            );
            let all_rects = pane_rects(
                body,
                if enlarged {
                    candidates.len().min(1)
                } else {
                    candidates.len()
                },
                plan.layout,
            );
            let visible: Vec<_> = if all_rects.len() < candidates.len() {
                focus
                    .selected
                    .or_else(|| candidates.first().copied())
                    .into_iter()
                    .collect()
            } else {
                candidates.clone()
            };
            for (id, rect) in visible.iter().zip(all_rects) {
                let command = &plan.stages[id.stage].commands[id.command];
                let selected = focus.selected == Some(*id);
                let title = format!(
                    " {} [{}] {} $ {} ",
                    command.name,
                    snapshot.state.command(*id).label(),
                    if focus.input == Some(*id) {
                        "INPUT"
                    } else if selected {
                        "SELECTED"
                    } else {
                        ""
                    },
                    without_ansi(command.script.as_bytes()).replace('\n', " ")
                );
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(if !colors {
                        Color::Reset
                    } else if selected {
                        Color::Magenta
                    } else {
                        match snapshot.state.command(*id) {
                            CommandState::Running | CommandState::Settling => Color::Blue,
                            CommandState::Finished(Outcome::Succeeded) => Color::Green,
                            CommandState::Finished(Outcome::Failed(_)) => Color::Red,
                            CommandState::Finished(Outcome::Cancelled(_)) => Color::Yellow,
                            _ => Color::DarkGray,
                        }
                    }));
                let inner = block.inner(rect);
                frame.render_widget(block, rect);
                if let Some(screen) = snapshot.screens.get(id) {
                    frame.render_widget(Pane { screen, colors }, inner);
                }
                if inner.width > 0
                    && inner.height > 0
                    && matches!(
                        snapshot.state.command(*id),
                        CommandState::Running | CommandState::Settling
                    )
                {
                    sizes.push((*id, inner.height, inner.width));
                }
            }
            let help = if focus.input.is_some() {
                "INPUT: Ctrl-G observe | Ctrl-C interrupt this command"
            } else {
                "OBSERVE: Tab select | z fullscreen | Esc restore | Enter input | Ctrl-C cancel"
            };
            let footer = if notice.is_empty() {
                help.to_owned()
            } else {
                format!("{help}\n{notice}")
            };
            frame.render_widget(
                Paragraph::new(footer),
                Rect::new(
                    area.x,
                    area.bottom().saturating_sub(2),
                    area.width,
                    area.height.min(2),
                ),
            );
        })?;
        for (id, rows, cols) in sizes {
            control.request(Request::Resize(id, rows, cols));
        }
        if snapshot.done || finished() {
            return Ok(());
        }
        if !event::poll(Duration::from_millis(33))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(id) = focus.input {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('g')
                    {
                        focus.leave();
                    } else if let Some(bytes) = encode_key(key)
                        && !control.request(Request::Input(id, bytes))
                    {
                        notice = "Input queue full; try again".into();
                    }
                } else {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            control.cancel(libc::SIGINT)
                        }
                        KeyCode::Char('z') if key.modifiers.is_empty() => enlarged = !enlarged,
                        KeyCode::Esc => enlarged = false,
                        KeyCode::Tab => focus.cycle(&candidates, false),
                        KeyCode::BackTab => focus.cycle(&candidates, true),
                        KeyCode::Enter if transport == Transport::Pty => {
                            focus.enter(&running);
                            notice = if focus.input_target().is_some() {
                                String::new()
                            } else {
                                "Only running commands accept input".into()
                            };
                        }
                        KeyCode::Enter => notice = "Child input requires PTY transport".into(),
                        _ => {}
                    }
                }
            }
            Event::Paste(text) => {
                if let Some(id) = focus.input
                    && (text.len() > 64 * 1024
                        || !control.request(Request::Input(id, text.into_bytes())))
                {
                    notice = "Paste too large or input queue full".into();
                }
            }
            _ => {}
        }
        thread::sleep(Duration::from_millis(33).saturating_sub(frame_started.elapsed()));
    }
}

fn header(plan: &Pipeline, snapshot: &Snapshot) -> String {
    let stages = plan
        .stages
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}: {}", s.name, snapshot.state.stage(i).label()))
        .collect::<Vec<_>>()
        .join(" | ");
    let current = snapshot
        .stage
        .map(|i| {
            let commands = plan.stages[i]
                .commands
                .iter()
                .map(|c| format!("{}:{}", c.name, snapshot.state.command(c.id).label()))
                .collect::<Vec<_>>()
                .join("  ");
            let mut running = 0;
            let mut queued = 0;
            let mut completed = 0;
            for command in &plan.stages[i].commands {
                match snapshot.state.command(command.id) {
                    CommandState::Queued => queued += 1,
                    CommandState::Running | CommandState::Settling => running += 1,
                    CommandState::Finished(_) => completed += 1,
                }
            }
            format!(
                "Stage {}/{} — {} | {running} active, {queued} queued, {completed} finished\n{commands}",
                i + 1,
                plan.stages.len(),
                plan.stages[i].name
            )
        })
        .unwrap_or_else(|| "Forkstr — completing pipeline".into());
    format!("{stages}\n{current}")
}

pub fn pane_rects(area: Rect, count: usize, layout: PaneLayout) -> Vec<Rect> {
    if count == 0 || area.width < 3 || area.height < 3 {
        return Vec::new();
    }
    let (cols, rows) = match layout {
        PaneLayout::Horizontal => (count, 1),
        PaneLayout::Vertical => (1, count),
        PaneLayout::Auto => {
            let cols = (count as f64).sqrt().ceil() as usize;
            (cols, count.div_ceil(cols))
        }
    };
    if area.width as usize / cols < 20 || area.height as usize / rows < 5 {
        return vec![area];
    }
    (0..count)
        .map(|i| {
            let col = i % cols;
            let row = i / cols;
            let x0 = (area.width as usize * col / cols) as u16;
            let x1 = (area.width as usize * (col + 1) / cols) as u16;
            let y0 = (area.height as usize * row / rows) as u16;
            let y1 = (area.height as usize * (row + 1) / rows) as u16;
            Rect::new(area.x + x0, area.y + y0, x1 - x0, y1 - y0)
        })
        .collect()
}

struct Pane<'a> {
    screen: &'a vt100::Screen,
    colors: bool,
}
impl Widget for Pane<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        for row in 0..area.height {
            for col in 0..area.width {
                if let Some(cell) = self.screen.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let mut style = Style::default()
                        .fg(color(cell.fgcolor()))
                        .bg(color(cell.bgcolor()));
                    for (enabled, modifier) in [
                        (cell.bold(), Modifier::BOLD),
                        (cell.italic(), Modifier::ITALIC),
                        (cell.underline(), Modifier::UNDERLINED),
                        (cell.inverse(), Modifier::REVERSED),
                    ] {
                        if enabled {
                            style = style.add_modifier(modifier);
                        }
                    }
                    if !self.colors {
                        style = Style::default();
                    }
                    let text = if cell.contents().is_empty() {
                        " "
                    } else {
                        cell.contents()
                    };
                    buffer.set_stringn(
                        area.x + col,
                        area.y + row,
                        text,
                        (area.width - col) as usize,
                        style,
                    );
                }
            }
        }
    }
}
fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub fn encode_key(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if c == 'z' {
                return None;
            } // Job suspension is outside the MVP.
            let c = c.to_ascii_uppercase();
            if ('@'..='_').contains(&c) {
                vec![(c as u8) & 0x1f]
            } else {
                return None;
            }
        }
        KeyCode::Char(c) => {
            let mut buffer = [0; 4];
            c.encode_utf8(&mut buffer).as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Backspace => vec![127],
        KeyCode::Esc => vec![27],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    };
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 27);
    }
    Some(bytes)
}
