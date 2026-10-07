use crate::{model::CommandId, process::Stream};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use vte::{Params, Perform};

pub fn run_directory(parent: Option<&Path>) -> io::Result<PathBuf> {
    if let Some(parent) = parent {
        fs::create_dir_all(parent)?;
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix("forkstr-");
    let dir = match parent {
        Some(parent) => builder.tempdir_in(parent)?,
        None => builder.tempdir()?,
    };
    Ok(dir.keep())
}

fn file(path: &Path) -> io::Result<BufWriter<File>> {
    Ok(BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?,
    ))
}

pub fn transcript_path(dir: &Path, id: CommandId) -> PathBuf {
    dir.join(format!("{}-{}.ansi", id.stage + 1, id.command + 1))
}

pub struct Capture {
    raw: BufWriter<File>,
    transcript: BufWriter<File>,
    events: BufWriter<File>,
    normalizers: [Normalizer; 3],
    started: Instant,
    offset: u64,
    last_partial: Instant,
}

impl Capture {
    pub fn new(dir: &Path, id: CommandId) -> io::Result<Self> {
        let stem = format!("{}-{}", id.stage + 1, id.command + 1);
        Ok(Self {
            raw: file(&dir.join(format!("{stem}.raw")))?,
            transcript: file(&transcript_path(dir, id))?,
            events: file(&dir.join(format!("{stem}.jsonl")))?,
            normalizers: std::array::from_fn(|_| Normalizer::new()),
            started: Instant::now(),
            offset: 0,
            last_partial: Instant::now(),
        })
    }

    pub fn ingest(&mut self, stream: Stream, bytes: &[u8]) -> io::Result<Vec<u8>> {
        self.raw.write_all(bytes)?;
        writeln!(
            self.events,
            "{}",
            serde_json::json!({ "type": "output", "stream": stream, "offset": self.offset, "length": bytes.len(), "micros": self.started.elapsed().as_micros() })
        )?;
        self.offset += bytes.len() as u64;
        let normalizer = &mut self.normalizers[stream_index(stream)];
        normalizer.process(bytes);
        let output = normalizer.take_output();
        self.transcript.write_all(&output)?;
        Ok(output)
    }

    pub fn partials(&mut self) -> Vec<(Stream, Vec<u8>)> {
        if self.last_partial.elapsed() < Duration::from_millis(100) {
            return Vec::new();
        }
        self.last_partial = Instant::now();
        [Stream::Stdout, Stream::Stderr, Stream::Pty]
            .into_iter()
            .filter_map(|stream| {
                let n = &mut self.normalizers[stream_index(stream)];
                if !n.line.dirty || n.line.cells.is_empty() {
                    return None;
                }
                n.line.dirty = false;
                Some((stream, n.line.render()))
            })
            .collect()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        writeln!(
            self.events,
            "{}",
            serde_json::json!({ "type": "resize", "rows": rows, "cols": cols, "micros": self.started.elapsed().as_micros() })
        )
    }

    pub fn finish(&mut self) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        for normalizer in &mut self.normalizers {
            normalizer.finish();
            output.extend(normalizer.take_output());
        }
        self.transcript.write_all(&output)?;
        self.raw.flush()?;
        self.transcript.flush()?;
        self.events.flush()?;
        Ok(output)
    }
}

fn stream_index(stream: Stream) -> usize {
    match stream {
        Stream::Stdout => 0,
        Stream::Stderr => 1,
        Stream::Pty => 2,
    }
}

/// An append-only projection for reports, deliberately separate from the pane
/// emulator. Raw capture is authoritative for arbitrary cursor-based programs.
pub struct Normalizer {
    parser: vte::Parser,
    line: Line,
    filter: OutputFilter,
}
impl Default for Normalizer {
    fn default() -> Self {
        Self::new()
    }
}
impl Normalizer {
    pub fn new() -> Self {
        Self {
            parser: vte::Parser::new(),
            line: Line::default(),
            filter: OutputFilter::default(),
        }
    }
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser
            .advance(&mut self.line, &self.filter.filter(bytes));
    }
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.line.output)
    }
    pub fn finish(&mut self) {
        // Flush a partial UTF-8 sequence as replacement text without treating an
        // unfinished escape sequence as printable terminal control bytes.
        let empty = self.line.cells.is_empty();
        let previous = self.line.output.len();
        self.parser.advance(&mut self.line, b"\n");
        if empty && &self.line.output[previous..] == b"\n" {
            self.line.output.truncate(previous);
        }
        if !self.line.cells.is_empty() {
            self.line.flush();
        }
    }
}

#[derive(Clone)]
struct StyledChar {
    value: char,
    style: Arc<str>,
}
#[derive(Default)]
struct Line {
    cells: Vec<StyledChar>,
    cursor: usize,
    style: Arc<str>,
    output: Vec<u8>,
    dirty: bool,
}
impl Line {
    fn render(&self) -> Vec<u8> {
        let mut out = String::new();
        let mut style: &str = "";
        for cell in &self.cells {
            if cell.style.as_ref() != style {
                out.push_str("\x1b[0m");
                out.push_str(&cell.style);
                style = &cell.style;
            }
            out.push(cell.value);
        }
        if !style.is_empty() {
            out.push_str("\x1b[0m");
        }
        out.into_bytes()
    }
    fn flush(&mut self) {
        self.output.extend(self.render());
        self.output.push(b'\n');
        self.cells.clear();
        self.cursor = 0;
        self.dirty = false;
    }
}
impl Perform for Line {
    fn print(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        if self.cursor >= 8192 {
            self.flush();
        }
        while self.cells.len() <= self.cursor {
            self.cells.push(StyledChar {
                value: ' ',
                style: Arc::from(""),
            });
        }
        self.cells[self.cursor] = StyledChar {
            value: c,
            style: self.style.clone(),
        };
        self.cursor += 1;
        self.dirty = true;
    }
    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.flush(),
            b'\r' => {
                self.cursor = 0;
            }
            b'\x08' => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            b'\t' => {
                let spaces = 8 - self.cursor % 8;
                for _ in 0..spaces {
                    self.print(' ');
                }
            }
            _ => {}
        }
    }
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore || !intermediates.is_empty() {
            return;
        }
        let first = params
            .iter()
            .next()
            .and_then(|p| p.first())
            .copied()
            .unwrap_or(0) as usize;
        match action {
            'm' => {
                let values: Vec<String> = params
                    .iter()
                    .map(|p| p.iter().map(u16::to_string).collect::<Vec<_>>().join(":"))
                    .collect();
                let code = format!("\x1b[{}m", values.join(";"));
                let mut style = if first == 0 || self.style.len() > 256 {
                    String::new()
                } else {
                    self.style.to_string()
                };
                style.push_str(&code);
                self.style = Arc::from(style);
            }
            'C' => self.cursor = (self.cursor + first.max(1)).min(8192),
            'D' => self.cursor = self.cursor.saturating_sub(first.max(1)),
            'G' => self.cursor = first.max(1).saturating_sub(1).min(8192),
            'K' => {
                match first {
                    0 => self.cells.truncate(self.cursor),
                    1 => {
                        for c in self.cells.iter_mut().take(self.cursor + 1) {
                            c.value = ' ';
                        }
                    }
                    2 => {
                        self.cells.clear();
                    }
                    _ => {}
                }
                self.dirty = true;
            }
            _ => {}
        }
    }
}

/// Only generated SGR controls reach reports; this strips them for plain sinks.
pub fn without_ansi(bytes: &[u8]) -> String {
    struct Text(String);
    impl Perform for Text {
        fn print(&mut self, c: char) {
            if !c.is_control() {
                self.0.push(c);
            }
        }
        fn execute(&mut self, b: u8) {
            if b == b'\n' || b == b'\t' {
                self.0.push(b as char);
            }
        }
    }
    let mut text = Text(String::new());
    vte::Parser::new().advance(&mut text, &OutputFilter::default().filter(bytes));
    text.0
}

/// Discard unsupported terminal strings before feeding parsers whose OSC
/// accumulator is otherwise unbounded. Raw capture is written before this filter.
#[derive(Default)]
pub struct OutputFilter {
    state: FilterState,
}

#[derive(Default)]
enum FilterState {
    #[default]
    Text,
    Escape,
    String {
        osc: bool,
        escape: bool,
    },
}

impl OutputFilter {
    pub fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            match &mut self.state {
                FilterState::Text => {
                    if byte == 27 {
                        self.state = FilterState::Escape;
                    } else {
                        output.push(byte);
                    }
                }
                FilterState::Escape => {
                    match byte {
                        b']' | b'P' | b'^' | b'_' | b'X' => {
                            // CAN clears any previously pending parser escape.
                            output.push(24);
                            self.state = FilterState::String {
                                osc: byte == b']',
                                escape: false,
                            };
                        }
                        27 => output.push(27),
                        _ => {
                            output.extend([27, byte]);
                            self.state = FilterState::Text;
                        }
                    }
                }
                FilterState::String { osc, escape } => {
                    if (*osc && byte == 7) || (*escape && byte == b'\\') || matches!(byte, 24 | 26)
                    {
                        self.state = FilterState::Text;
                    } else {
                        *escape = byte == 27;
                    }
                }
            }
        }
        output
    }
}
