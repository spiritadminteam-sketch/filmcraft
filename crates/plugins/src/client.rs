//! The FilmCraft side of the plug-in host: starts `filmcraft-plugin-host`, sends it requests and
//! reads its answers with time-outs. A host that crashes, hangs or answers nonsense is killed and
//! forgotten; the next request starts a fresh one. Nothing here can take the editor down.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::proto::{self, Frame, ProcessRequest, ProcessResponse, Request, Response};

/// How long one audio block may take.
pub const PROCESS_TIMEOUT: Duration = Duration::from_secs(3);
/// How long creating an instance may take (some plug-ins load large resources).
pub const CREATE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long scanning one module may take.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(60);

/// The helper executable's file name.
pub fn host_exe_name() -> &'static str {
    if cfg!(windows) { "filmcraft-plugin-host.exe" } else { "filmcraft-plugin-host" }
}

/// Where the helper is: `FILMCRAFT_PLUGIN_HOST`, or next to the running executable (or one
/// folder up, for `cargo test` binaries in `target/*/deps`).
pub fn host_exe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("FILMCRAFT_PLUGIN_HOST").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(host_exe_name()), dir.parent()?.join(host_exe_name())].into_iter().find(|p| p.is_file())
}

struct Io {
    stdin: ChildStdin,
    rx: Receiver<Frame>,
}

/// One running helper process.
pub struct Host {
    child: Mutex<Child>,
    io: Mutex<Io>,
    alive: Arc<AtomicBool>,
    /// Distinguishes host processes, so instances of a dead host are never sent to a new one.
    pub generation: u64,
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

fn current() -> &'static Mutex<Option<Arc<Host>>> {
    static H: Mutex<Option<Arc<Host>>> = Mutex::new(None);
    &H
}

/// The running host, started if needed.
pub fn host() -> Result<Arc<Host>, String> {
    let mut cur = current().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(h) = cur.as_ref().filter(|h| h.is_alive()) {
        return Ok(h.clone());
    }
    let h = Arc::new(Host::spawn()?);
    *cur = Some(h.clone());
    Ok(h)
}

/// Stop the running host (instances of it fall back to passing audio through).
pub fn shutdown() {
    if let Some(h) = current().lock().unwrap_or_else(PoisonError::into_inner).take() {
        h.kill();
    }
}

impl Host {
    /// Start a new host process (scans use their own, so a slow scan never blocks playback).
    pub fn spawn() -> Result<Host, String> {
        let exe = host_exe().ok_or_else(|| format!("{} is missing next to FilmCraft", host_exe_name()))?;
        let mut cmd = Command::new(&exe);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console window flashes up
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
        let stdin = child.stdin.take().ok_or("no pipe to the plug-in host")?;
        let mut stdout = child.stdout.take().ok_or("no pipe from the plug-in host")?;
        let alive = Arc::new(AtomicBool::new(true));
        let (tx, rx) = channel();
        let flag = alive.clone();
        std::thread::Builder::new()
            .name("plugin-host-reader".into())
            .spawn(move || {
                while let Ok(Some(f)) = proto::read_frame(&mut stdout) {
                    if tx.send(f).is_err() {
                        break;
                    }
                }
                flag.store(false, Ordering::SeqCst);
            })
            .map_err(|e| e.to_string())?;
        let host = Host { child: Mutex::new(child), io: Mutex::new(Io { stdin, rx }), alive, generation: GENERATION.fetch_add(1, Ordering::Relaxed) };
        match host.request(&Request::Hello, Duration::from_secs(10))? {
            Response::Hello { version } if version == proto::VERSION => Ok(host),
            Response::Hello { version } => {
                host.kill();
                Err(format!("plug-in host speaks protocol {version}, FilmCraft {}", proto::VERSION))
            }
            other => {
                host.kill();
                Err(format!("unexpected answer from the plug-in host: {other:?}"))
            }
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let mut c = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = c.kill();
        let _ = c.wait();
    }

    fn exchange(&self, send: impl FnOnce(&mut dyn Write) -> std::io::Result<()>, timeout: Duration) -> Result<Frame, String> {
        if !self.is_alive() {
            return Err("the plug-in host stopped".into());
        }
        let io = &mut *self.io.lock().unwrap_or_else(PoisonError::into_inner);
        // drop anything stale (an answer that arrived after an earlier time-out)
        while io.rx.try_recv().is_ok() {}
        if let Err(e) = send(&mut io.stdin) {
            self.kill();
            return Err(format!("plug-in host: {e}"));
        }
        match io.rx.recv_timeout(timeout) {
            Ok(f) => Ok(f),
            Err(RecvTimeoutError::Timeout) => {
                // a plug-in that hangs: stop the whole host rather than wait forever
                self.kill();
                Err("the plug-in stopped responding".into())
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.kill();
                Err("the plug-in host crashed".into())
            }
        }
    }

    /// Send a control request and wait for its answer.
    pub fn request(&self, req: &Request, timeout: Duration) -> Result<Response, String> {
        match self.exchange(|w| proto::write_json(w, req), timeout)? {
            Frame::Json(j) => serde_json::from_slice(&j).map_err(|e| format!("plug-in host: {e}")),
            _ => {
                self.kill();
                Err("plug-in host: unexpected frame".into())
            }
        }
    }

    /// Process one block.
    pub fn process(&self, req: &ProcessRequest) -> Result<ProcessResponse, String> {
        match self.exchange(|w| proto::write_process(w, req), PROCESS_TIMEOUT)? {
            Frame::Processed(p) => Ok(p),
            _ => {
                self.kill();
                Err("plug-in host: unexpected frame".into())
            }
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // closing stdin ends the host's loop; make sure it is gone
        self.kill();
    }
}
