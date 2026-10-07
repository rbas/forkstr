//! Unix process ownership. A child stays unreaped until its group is signalled,
//! preventing its process/group ID from being reused during cleanup.
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use nix::{
    fcntl::{FcntlArg, FdFlag, OFlag, fcntl},
    pty::{Winsize, openpty},
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use serde::Serialize;

use crate::model::{CommandSpec, Transport};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
    Pty,
}
impl Stream {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Pty => "pty",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}
impl Exit {
    pub fn success(self) -> bool {
        matches!(self, Self::Code(0))
    }
    pub fn description(self) -> String {
        match self {
            Self::Code(code) => format!("exit code {code}"),
            Self::Signal(signal) => format!("signal {signal}"),
        }
    }
}

struct Reader {
    file: File,
    stream: Stream,
    eof: bool,
}

pub struct ChildProcess {
    child: Child,
    readers: Vec<Reader>,
    master: Option<File>,
    input: VecDeque<u8>,
    exit: Option<Exit>,
    stopping: Option<Instant>,
    forced: Option<Instant>,
    reaped: bool,
    size: (u16, u16),
}

impl ChildProcess {
    pub fn spawn(spec: &CommandSpec, transport: Transport) -> io::Result<Self> {
        let mut command = Command::new(&spec.shell);
        command
            .arg("-c")
            .arg(&spec.script)
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(&spec.env);
        let pty = if transport == Transport::Pty {
            let pair = openpty(
                Some(&Winsize {
                    ws_row: 24,
                    ws_col: 80,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                }),
                None,
            )
            .map_err(io::Error::from)?;
            fcntl(&pair.master, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;
            fcntl(&pair.slave, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;
            command.stdin(Stdio::from(pair.slave.try_clone()?));
            command.stdout(Stdio::from(pair.slave.try_clone()?));
            command.stderr(Stdio::from(pair.slave));
            if !spec.env.contains_key(std::ffi::OsStr::new("TERM")) {
                command.env("TERM", "xterm");
            }
            Some(File::from(pair.master))
        } else {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            None
        };
        let has_pty = pty.is_some();
        // SAFETY: only async-signal-safe Unix operations occur after fork. The
        // standard library has already installed stdin/out/err at this point.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if has_pty {
                    if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    if libc::tcsetpgrp(0, libc::getpid()) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        // Drop Command now: its owned slave handles must not keep the PTY open.
        drop(command);
        let mut process = Self {
            child,
            readers: Vec::new(),
            master: pty,
            input: VecDeque::new(),
            exit: None,
            stopping: None,
            forced: None,
            reaped: false,
            size: (24, 80),
        };
        if let Some(master) = &process.master {
            nonblocking(master)?;
            process.readers.push(Reader {
                file: master.try_clone()?,
                stream: Stream::Pty,
                eof: false,
            });
        } else {
            if let Some(stdout) = process.child.stdout.take() {
                let file = File::from(OwnedFd::from(stdout));
                nonblocking(&file)?;
                process.readers.push(Reader {
                    file,
                    stream: Stream::Stdout,
                    eof: false,
                });
            }
            if let Some(stderr) = process.child.stderr.take() {
                let file = File::from(OwnedFd::from(stderr));
                nonblocking(&file)?;
                process.readers.push(Reader {
                    file,
                    stream: Stream::Stderr,
                    eof: false,
                });
            }
        }
        Ok(process)
    }

    pub fn is_interactive(&self) -> bool {
        self.master.is_some() && self.exit.is_none() && self.stopping.is_none()
    }

    pub fn queue_input(&mut self, bytes: &[u8]) -> io::Result<()> {
        if !self.is_interactive() {
            return Ok(());
        }
        if self.input.len() + bytes.len() > 64 * 1024 {
            return Err(io::Error::other(
                "input buffer full; paste a smaller amount",
            ));
        }
        self.input.extend(bytes);
        Ok(())
    }

    pub fn flush_input(&mut self) -> io::Result<()> {
        if let Some(master) = &mut self.master
            && !self.input.is_empty()
        {
            match master.write(self.input.make_contiguous()) {
                Ok(n) => {
                    self.input.drain(..n);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) if matches!(e.raw_os_error(), Some(libc::EIO | libc::EPIPE)) => {
                    self.input.clear();
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<bool> {
        let size = (rows.max(1), cols.max(1));
        if self.size == size {
            return Ok(false);
        }
        if let Some(master) = &self.master {
            let winsize = Winsize {
                ws_row: size.0,
                ws_col: size.1,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // SAFETY: master is a live PTY fd and winsize points to initialized data.
            if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as _, &winsize) } == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        self.size = size;
        Ok(true)
    }

    /// Bounded reads preserve fairness when one command produces endless output.
    pub fn read_available(&mut self) -> io::Result<Vec<(Stream, Vec<u8>)>> {
        let mut output = Vec::new();
        for reader in &mut self.readers {
            if reader.eof {
                continue;
            }
            for _ in 0..8 {
                let mut bytes = vec![0; 8192];
                match reader.file.read(&mut bytes) {
                    Ok(0) => {
                        reader.eof = true;
                        break;
                    }
                    Ok(n) => {
                        bytes.truncate(n);
                        output.push((reader.stream, bytes));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e)
                        if matches!(reader.stream, Stream::Pty)
                            && e.raw_os_error() == Some(libc::EIO) =>
                    {
                        reader.eof = true;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(output)
    }

    pub fn observe_exit(&mut self) -> io::Result<Option<Exit>> {
        if self.exit.is_none() {
            // SAFETY: waitid initializes siginfo; zeroing also supports WNOHANG
            // implementations that leave it untouched. WNOWAIT retains ownership
            // of the child's PID until final group cleanup and Child::wait.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id() as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result == -1 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    return Ok(None);
                }
                return Err(error);
            }
            // SAFETY: successful waitid with WEXITED supplies these union fields.
            if unsafe { info.si_pid() } != 0 {
                let status = unsafe { info.si_status() };
                self.exit = Some(if info.si_code == libc::CLD_EXITED {
                    Exit::Code(status)
                } else {
                    Exit::Signal(status)
                });
                self.input.clear();
            }
        }
        Ok(self.exit)
    }

    pub fn stop(&mut self, force: bool) -> io::Result<()> {
        if self.reaped {
            return Ok(());
        }
        self.input.clear();
        if self.stopping.is_none() {
            self.stopping = Some(Instant::now());
            self.signal(Signal::SIGTERM)?;
            self.signal(Signal::SIGCONT)?;
        }
        if force && self.forced.is_none() {
            self.signal(Signal::SIGKILL)?;
            self.forced = Some(Instant::now());
        }
        Ok(())
    }

    /// Complete only after exit and output EOF. Holding the zombie prevents PID
    /// reuse while killing remaining group members, even those with closed output.
    pub fn settle(&mut self) -> io::Result<bool> {
        if self.exit.is_some() {
            self.stop(false)?;
        }
        if self
            .stopping
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(2))
        {
            self.stop(true)?;
        }
        if self.exit.is_some()
            && self.readers.iter().all(|r| r.eof)
            && !group_has_live_members(self.child.id() as i32)?
        {
            self.child.wait()?;
            self.reaped = true;
            return Ok(true);
        }
        if self
            .forced
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(1))
        {
            return Err(io::Error::other(
                "cleanup deadline exceeded; output capture may be incomplete",
            ));
        }
        Ok(false)
    }

    fn signal(&self, signal: Signal) -> io::Result<()> {
        match killpg(Pid::from_raw(self.child.id() as i32), signal) {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
            Err(nix::errno::Errno::EPERM) if !group_has_live_members(self.child.id() as i32)? => {
                Ok(())
            }
            Err(error) => Err(io::Error::other(format!(
                "signal {signal:?} to group {}: {error}",
                self.child.id()
            ))),
        }
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.signal(Signal::SIGKILL);
            // Do not block indefinitely on an uninterruptible kernel task.
            for _ in 0..100 {
                if self.child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

fn nonblocking(file: &File) -> io::Result<()> {
    let flags = fcntl(file, FcntlArg::F_GETFL).map_err(io::Error::from)?;
    fcntl(
        file,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map_err(io::Error::from)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn group_has_live_members(group: i32) -> io::Result<bool> {
    // Darwin killpg can return EPERM when only zombies remain. Verify absence
    // of live members rather than suppressing genuine permission errors.
    const PROC_PGRP_ONLY: u32 = 2;
    let mut pids = vec![0i32; 64];
    loop {
        let bytes = std::mem::size_of_val(pids.as_slice());
        // SAFETY: initialized, writable PID array with its exact byte capacity.
        let used = unsafe {
            libc::proc_listpids(
                PROC_PGRP_ONLY,
                group as u32,
                pids.as_mut_ptr().cast(),
                bytes as i32,
            )
        };
        if used < 0 {
            return Err(io::Error::last_os_error());
        }
        if used as usize >= bytes {
            if pids.len() >= 1_048_576 {
                return Err(io::Error::other("process group too large to inspect"));
            }
            pids.resize(pids.len() * 2, 0);
            continue;
        }
        for &pid in pids
            .iter()
            .take(used as usize / size_of::<i32>())
            .filter(|p| **p > 0)
        {
            // SAFETY: zeroed C structure and matching buffer size passed to libproc.
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = size_of::<libc::proc_bsdinfo>() as i32;
            let received = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    (&mut info as *mut libc::proc_bsdinfo).cast(),
                    size,
                )
            };
            if received == size {
                if info.pbi_pgid == group as u32 && info.pbi_status != libc::SZOMB {
                    return Ok(true);
                }
            } else {
                let error = io::Error::last_os_error();
                if !matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ENOENT)) {
                    return Err(error);
                }
            }
        }
        return Ok(false);
    }
}

#[cfg(target_os = "linux")]
fn group_has_live_members(group: i32) -> io::Result<bool> {
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let text = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(text) => text,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some((_, fields)) = text.rsplit_once(')') {
            let fields: Vec<_> = fields.split_whitespace().take(3).collect();
            if fields.len() == 3
                && fields[2].parse::<i32>().ok() == Some(group)
                && !matches!(fields[0], "Z" | "X")
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
