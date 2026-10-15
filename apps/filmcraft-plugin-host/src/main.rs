//! `filmcraft-plugin-host`: FilmCraft's helper process for third-party audio plug-ins (VST3).
//!
//! FilmCraft starts it with pipes on stdin / stdout and sends requests in the frames of
//! `filmcraft_plugins::proto`: scan a module, create an instance, process a block, destroy an
//! instance. Plug-in code runs only here, so a plug-in that crashes or hangs costs this process,
//! never the editor (docs/adr/0002-plugin-hosting.md). Every request also runs under
//! `catch_unwind`, so a Rust panic answers with an error instead of ending the process.
//!
//! It is not meant to be run by hand; with no FilmCraft on the other end it just waits for input.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::io::{BufReader, BufWriter, Write};

use filmcraft_plugins::proto::{self, Frame, ProcessRequest, ProcessResponse, Request, Response};

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod host {
    use std::collections::HashMap;
    use std::path::Path;

    use filmcraft_platform::vst3host::{Instance, Module};
    use filmcraft_plugins::proto::{self, ParamDesc, PluginInfo, ProcessRequest, ProcessResponse, Response};

    #[derive(Default)]
    pub struct Host {
        modules: HashMap<String, Module>,
        instances: HashMap<u32, Instance>,
        next: u32,
    }

    fn params_of(inst: &Instance) -> Vec<ParamDesc> {
        inst.params()
            .into_iter()
            .filter(|p| p.is_user_param())
            .map(|p| ParamDesc { id: p.id, title: p.title, units: p.units, steps: p.step_count, default: p.default })
            .collect()
    }

    impl Host {
        fn module(&mut self, path: &str) -> Result<Module, String> {
            if let Some(m) = self.modules.get(path) {
                return Ok(m.clone());
            }
            let m = Module::load(Path::new(path))?;
            self.modules.insert(path.to_string(), m.clone());
            Ok(m)
        }

        pub fn scan(&mut self, path: &str) -> Response {
            let m = match Module::load(Path::new(path)) {
                Ok(m) => m,
                Err(message) => return Response::Error { message },
            };
            let plugins = m
                .classes()
                .into_iter()
                .map(|c| {
                    // the parameters come from a short-lived instance
                    let params = m.instantiate(&c.cid, 48000.0).map(|i| params_of(&i)).unwrap_or_default();
                    PluginInfo {
                        path: path.to_string(),
                        cid: proto::cid_hex(&c.cid),
                        name: if c.name.is_empty() { "Plug-in".into() } else { c.name },
                        vendor: c.vendor,
                        category: c.sub_categories,
                        version: c.version,
                        params,
                    }
                })
                .collect();
            Response::Scanned { plugins }
        }

        pub fn create(&mut self, path: &str, cid: &str, sample_rate: f64, params: &[(u32, f64)]) -> Response {
            let Some(cid) = proto::cid_parse(cid) else { return Response::Error { message: format!("bad class id {cid}") } };
            let m = match self.module(path) {
                Ok(m) => m,
                Err(message) => return Response::Error { message },
            };
            let mut inst = match m.instantiate(&cid, sample_rate) {
                Ok(i) => i,
                Err(message) => return Response::Error { message },
            };
            if !params.is_empty() {
                if let Err(message) = inst.process(&mut [], &mut [], params) {
                    return Response::Error { message };
                }
            }
            if self.instances.len() >= 512 {
                return Response::Error { message: "too many plug-in instances".into() };
            }
            self.next = self.next.wrapping_add(1).max(1);
            let id = self.next;
            let latency = inst.latency();
            self.instances.insert(id, inst);
            Response::Created { inst: id, latency }
        }

        pub fn destroy(&mut self, inst: u32) -> Response {
            self.instances.remove(&inst);
            Response::Ok
        }

        pub fn process(&mut self, r: ProcessRequest) -> ProcessResponse {
            let ProcessRequest { inst, changes, mut left, mut right } = r;
            let Some(i) = self.instances.get_mut(&inst) else {
                return ProcessResponse { error: Some(format!("no instance {inst}")), left, right };
            };
            let (l0, r0) = (left.clone(), right.clone());
            match i.process(&mut left, &mut right, &changes) {
                Ok(()) => {
                    // a plug-in producing NaN / infinity would poison the mix: silence it instead
                    for v in left.iter_mut().chain(right.iter_mut()) {
                        if !v.is_finite() {
                            *v = 0.0;
                        }
                    }
                    ProcessResponse { error: None, left, right }
                }
                Err(e) => ProcessResponse { error: Some(e), left: l0, right: r0 },
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod host {
    use filmcraft_plugins::proto::{ProcessRequest, ProcessResponse, Response};

    const NOT_HERE: &str = "VST3 plug-ins are not supported on this system yet";

    #[derive(Default)]
    pub struct Host;

    impl Host {
        pub fn scan(&mut self, _path: &str) -> Response {
            Response::Error { message: NOT_HERE.into() }
        }
        pub fn create(&mut self, _path: &str, _cid: &str, _sample_rate: f64, _params: &[(u32, f64)]) -> Response {
            Response::Error { message: NOT_HERE.into() }
        }
        pub fn destroy(&mut self, _inst: u32) -> Response {
            Response::Ok
        }
        pub fn process(&mut self, r: ProcessRequest) -> ProcessResponse {
            ProcessResponse { error: Some(NOT_HERE.into()), left: r.left, right: r.right }
        }
    }
}

fn panic_message(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_else(|| "internal error".into())
}

fn handle_json(h: &mut host::Host, bytes: &[u8]) -> Response {
    let req: Request = match serde_json::from_slice(bytes) {
        Ok(r) => r,
        Err(e) => return Response::Error { message: format!("bad request: {e}") },
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match req {
        Request::Hello => Response::Hello { version: proto::VERSION },
        Request::Scan { path } => h.scan(&path),
        Request::Create { path, cid, sample_rate, params } => h.create(&path, &cid, sample_rate, &params),
        Request::Destroy { inst } => h.destroy(inst),
    }));
    run.unwrap_or_else(|e| Response::Error { message: panic_message(e.as_ref()) })
}

fn handle_process(h: &mut host::Host, r: ProcessRequest) -> ProcessResponse {
    let (l, rr) = (r.left.clone(), r.right.clone());
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.process(r))).unwrap_or_else(|e| ProcessResponse {
        error: Some(panic_message(e.as_ref())),
        left: l,
        right: rr,
    })
}

fn main() {
    // a panic message on stderr is all FilmCraft could want; stdout carries only frames
    std::panic::set_hook(Box::new(|info| eprintln!("filmcraft-plugin-host: {info}")));
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = BufWriter::new(std::io::stdout().lock());
    let mut h = host::Host::default();
    loop {
        let frame = match proto::read_frame(&mut input) {
            Ok(Some(f)) => f,
            // FilmCraft closed the pipe (or sent garbage): stop
            Ok(None) | Err(_) => break,
        };
        let written = match frame {
            Frame::Json(j) => proto::write_json(&mut output, &handle_json(&mut h, &j)),
            Frame::Process(p) => proto::write_processed(&mut output, &handle_process(&mut h, p)),
            Frame::Processed(_) => proto::write_json(&mut output, &Response::Error { message: "unexpected frame".into() }),
        };
        if written.and_then(|()| output.flush()).is_err() {
            break;
        }
    }
}
