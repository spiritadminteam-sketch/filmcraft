//! VST3 audio effect hosting: load a `.vst3` module, list its audio effect classes, create an
//! instance, read its parameters and process stereo audio through it.
//!
//! This runs only inside `filmcraft-plugin-host`, the helper process FilmCraft talks to over
//! pipes (docs/adr/0002-plugin-hosting.md): third-party plug-in code can crash or hang, and a
//! crash there must never take the editor (and the user's work) with it.
//!
//! The VST3 interfaces come from the `vst3` crate (MIT OR Apache-2.0), generated from Steinberg's
//! VST3 SDK headers (MIT since SDK 3.8). Everything public here is safe; the `unsafe` FFI calls
//! are contained in this module, each with a `// SAFETY:` comment.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, BusDirections_, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentHandlerTrait,
    IComponentTrait, IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait, IHostApplication, IHostApplicationTrait, IParamValueQueue,
    IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, MediaTypes_, ParameterInfo, ProcessContext, ProcessData, ProcessModes_, ProcessSetup,
    SpeakerArr, SpeakerArrangement, String128, SymbolicSampleSizes_,
};
use vst3::Steinberg::{
    FIDString, FUnknown, IBStream, IBStream_, IBStreamTrait, IPluginBaseTrait, IPluginFactory, IPluginFactory2, IPluginFactory2Trait, IPluginFactoryTrait,
    PClassInfo, PClassInfo2, TUID, int32, int64, kInvalidArgument, kNotImplemented, kResultFalse, kResultOk, tresult,
};
use vst3::{Class, ComPtr, ComWrapper, Interface};

/// Largest block [`Instance::process`] takes (samples per channel).
pub const MAX_BLOCK: usize = 4096;
/// Most parameters read from one plug-in (a hostile or buggy count is capped).
const MAX_PARAMS: i32 = 4096;
/// Most classes read from one factory.
const MAX_CLASSES: i32 = 1024;
/// Most audio buses per direction.
const MAX_BUSES: i32 = 16;

/// One audio effect class of a module.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassInfo {
    /// The class id (16 bytes), as the factory reports it.
    pub cid: [u8; 16],
    pub name: String,
    pub vendor: String,
    /// VST3 sub-categories, e.g. `Fx|Dynamics`.
    pub sub_categories: String,
    pub version: String,
}

/// One parameter of an instance.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo {
    pub id: u32,
    pub title: String,
    pub units: String,
    /// 0 = continuous, otherwise the number of steps.
    pub step_count: i32,
    /// Normalised (0…1).
    pub default: f64,
    pub flags: i32,
}

impl ParamInfo {
    /// Hidden or read-only parameters are not shown or automated.
    pub fn is_user_param(&self) -> bool {
        // ParameterInfo flags (VST3 `ivsteditcontroller.h`)
        const K_IS_READ_ONLY: i32 = 1 << 1;
        const K_IS_HIDDEN: i32 = 1 << 4;
        const K_IS_PROGRAM_CHANGE: i32 = 1 << 15;
        self.flags & (K_IS_READ_ONLY | K_IS_HIDDEN | K_IS_PROGRAM_CHANGE) == 0
    }
}

fn cstr(bytes: &[i8]) -> String {
    let b: Vec<u8> = bytes.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&b).trim().to_string()
}

fn wstr(s: &String128) -> String {
    let n = s.iter().position(|c| *c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(s.get(..n).unwrap_or(&[])).trim().to_string()
}

fn tuid_bytes(t: &TUID) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (o, c) in out.iter_mut().zip(t.iter()) {
        *o = *c as u8;
    }
    out
}

fn bytes_tuid(b: &[u8; 16]) -> TUID {
    let mut out: TUID = [0; 16];
    for (o, c) in out.iter_mut().zip(b.iter()) {
        *o = *c as i8;
    }
    out
}

/// The binary inside a `.vst3` bundle (a folder) for this system, or the file itself (the older
/// single-file layout on Windows).
pub fn module_binary(path: &Path) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if !path.is_dir() {
        return Err(format!("{}: not found", path.display()));
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let arch = if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") { "arm64-win" } else { "x86_64-win" }
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-linux"
    } else {
        "x86_64-linux"
    };
    let dir = path.join("Contents").join(arch);
    let ext = if cfg!(target_os = "windows") { "vst3" } else { "so" };
    let exact = dir.join(format!("{stem}.{ext}"));
    if exact.is_file() {
        return Ok(exact);
    }
    // some bundles name the binary differently from the bundle
    let found = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) && p.is_file());
    found.ok_or_else(|| format!("{}: no {ext} binary for this system in the bundle", path.display()))
}

struct ModuleInner {
    factory: Option<ComPtr<IPluginFactory>>,
    lib: Option<libloading::Library>,
}

impl Drop for ModuleInner {
    fn drop(&mut self) {
        // the factory is released before the module is unloaded
        drop(self.factory.take());
        let Some(lib) = self.lib.take() else { return };
        // SAFETY: `ExitDll` / `ModuleExit` take no arguments and return a bool per the VST3
        // module ABI; looking the symbol up does not call anything.
        unsafe {
            #[cfg(target_os = "windows")]
            if let Ok(exit) = lib.get::<unsafe extern "system" fn() -> bool>(b"ExitDll\0") {
                exit();
            }
            #[cfg(not(target_os = "windows"))]
            if let Ok(exit) = lib.get::<unsafe extern "system" fn() -> bool>(b"ModuleExit\0") {
                exit();
            }
        }
        drop(lib);
    }
}

/// A loaded `.vst3` module. Instances keep it loaded.
#[derive(Clone)]
pub struct Module {
    inner: Rc<ModuleInner>,
    pub path: PathBuf,
}

impl Module {
    /// Load the module at `path` (a `.vst3` bundle folder or file) and get its factory.
    pub fn load(path: &Path) -> Result<Module, String> {
        let bin = module_binary(path)?;
        // SAFETY: loading a library runs its initialisers; that is the point of hosting a plug-in,
        // and this only ever runs in the separate plug-in host process, never in the editor.
        let lib = unsafe { libloading::Library::new(&bin) }.map_err(|e| format!("{}: {e}", bin.display()))?;
        // SAFETY: the VST3 module entry points have exactly these signatures (VST3 SDK
        // `moduleinit`): `InitDll()` / `ModuleEntry(handle)` return a bool.
        unsafe {
            #[cfg(target_os = "windows")]
            if let Ok(init) = lib.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0") {
                if !init() {
                    return Err(format!("{}: InitDll failed", bin.display()));
                }
            }
            #[cfg(not(target_os = "windows"))]
            if let Ok(init) = lib.get::<unsafe extern "system" fn(*mut c_void) -> bool>(b"ModuleEntry\0") {
                if !init(std::ptr::null_mut()) {
                    return Err(format!("{}: ModuleEntry failed", bin.display()));
                }
            }
        }
        // SAFETY: `GetPluginFactory` is the VST3 factory entry point: no arguments, returns an
        // owned (already add-ref'd) `IPluginFactory*` or null.
        let raw = unsafe {
            let get = lib.get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0").map_err(|e| format!("{}: {e}", bin.display()))?;
            get()
        };
        // SAFETY: `raw` is null or an owned COM pointer returned by the factory entry point.
        let factory = unsafe { ComPtr::from_raw(raw) }.ok_or_else(|| format!("{}: no plug-in factory", bin.display()))?;
        Ok(Module { inner: Rc::new(ModuleInner { factory: Some(factory), lib: Some(lib) }), path: path.to_path_buf() })
    }

    /// The module's audio effect classes (instruments and non-audio classes are left out).
    pub fn classes(&self) -> Vec<ClassInfo> {
        let Some(f) = &self.inner.factory else { return Vec::new() };
        let f2: Option<ComPtr<IPluginFactory2>> = f.cast();
        // SAFETY: `countClasses` takes no arguments.
        let n = unsafe { f.countClasses() }.clamp(0, MAX_CLASSES);
        let mut out = Vec::new();
        for i in 0..n {
            let info = match &f2 {
                Some(f2) => {
                    // SAFETY: PClassInfo2 is plain data; zeroed is a valid value of it.
                    let mut ci: PClassInfo2 = unsafe { std::mem::zeroed() };
                    // SAFETY: `ci` is a valid, writable PClassInfo2 for the call.
                    if unsafe { f2.getClassInfo2(i, &mut ci) } != kResultOk {
                        continue;
                    }
                    if cstr(&ci.category) != "Audio Module Class" {
                        continue;
                    }
                    ClassInfo {
                        cid: tuid_bytes(&ci.cid),
                        name: cstr(&ci.name),
                        vendor: cstr(&ci.vendor),
                        sub_categories: cstr(&ci.subCategories),
                        version: cstr(&ci.version),
                    }
                }
                None => {
                    // SAFETY: PClassInfo is plain data; zeroed is a valid value of it.
                    let mut ci: PClassInfo = unsafe { std::mem::zeroed() };
                    // SAFETY: `ci` is a valid, writable PClassInfo for the call.
                    if unsafe { f.getClassInfo(i, &mut ci) } != kResultOk {
                        continue;
                    }
                    if cstr(&ci.category) != "Audio Module Class" {
                        continue;
                    }
                    ClassInfo { cid: tuid_bytes(&ci.cid), name: cstr(&ci.name), vendor: String::new(), sub_categories: String::new(), version: String::new() }
                }
            };
            // effects only: instruments need MIDI input, which a clip effect does not have
            let sub = info.sub_categories.to_ascii_lowercase();
            if sub.contains("instrument") && !sub.contains("fx") {
                continue;
            }
            out.push(info);
        }
        out
    }

    fn create<I: Interface>(&self, cid: &[u8; 16]) -> Option<ComPtr<I>> {
        let factory = self.inner.factory.as_ref()?;
        let tuid = bytes_tuid(cid);
        let mut obj: *mut c_void = std::ptr::null_mut();
        // SAFETY: `tuid` and `I::IID` are 16-byte ids that live for the call; `obj` is a valid
        // out-pointer. On success the factory returns an owned reference in `obj`.
        let r = unsafe { factory.createInstance(tuid.as_ptr() as FIDString, I::IID.as_ptr() as FIDString, &mut obj) };
        if r != kResultOk {
            return None;
        }
        // SAFETY: on success `obj` is null or an owned pointer to an `I`.
        unsafe { ComPtr::from_raw(obj as *mut I) }
    }

    /// Create, initialise and activate an instance of class `cid` for stereo processing at
    /// `sample_rate`, in blocks of up to [`MAX_BLOCK`] samples.
    pub fn instantiate(&self, cid: &[u8; 16], sample_rate: f64) -> Result<Instance, String> {
        Instance::new(self, cid, sample_rate)
    }
}

// ---- host-side COM objects ----------------------------------------------------------------

struct HostApp;
impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}
impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        // SAFETY: the plug-in passes a writable String128 (VST3 contract); null is rejected.
        let Some(out) = (unsafe { name.as_mut() }) else { return kInvalidArgument };
        for (o, c) in out.iter_mut().zip("FilmCraft".encode_utf16().chain(std::iter::once(0))) {
            *o = c;
        }
        kResultOk
    }
    unsafe fn createInstance(&self, _cid: *mut TUID, _iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        // SAFETY: `obj` is the plug-in's out-pointer; it is set to null when not null.
        if let Some(o) = unsafe { obj.as_mut() } {
            *o = std::ptr::null_mut();
        }
        kResultFalse
    }
}

struct Handler;
impl Class for Handler {
    type Interfaces = (IComponentHandler,);
}
impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, _id: u32) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, _id: u32, _v: f64) -> tresult {
        kResultOk
    }
    unsafe fn endEdit(&self, _id: u32) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        kResultOk
    }
}

/// An in-memory `IBStream` (component state → controller).
struct MemStream {
    data: RefCell<Vec<u8>>,
    pos: Cell<usize>,
}
impl Class for MemStream {
    type Interfaces = (IBStream,);
}
impl IBStreamTrait for MemStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, read: *mut int32) -> tresult {
        let data = self.data.borrow();
        let pos = self.pos.get().min(data.len());
        let n = (num_bytes.max(0) as usize).min(data.len() - pos);
        if n > 0 && !buffer.is_null() {
            // SAFETY: the plug-in passes a buffer of at least `num_bytes` writable bytes; `n` is at
            // most that and within `data`.
            unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().add(pos), buffer as *mut u8, n) };
        }
        self.pos.set(pos + n);
        // SAFETY: optional out-pointer, written only when not null.
        if let Some(r) = unsafe { read.as_mut() } {
            *r = n as int32;
        }
        kResultOk
    }
    unsafe fn write(&self, buffer: *mut c_void, num_bytes: int32, written: *mut int32) -> tresult {
        let n = num_bytes.max(0) as usize;
        // a plug-in state larger than 256 MB is refused (capped allocation)
        if buffer.is_null() || self.pos.get().saturating_add(n) > 256 << 20 {
            return kResultFalse;
        }
        // SAFETY: the plug-in passes `num_bytes` readable bytes at `buffer`.
        let src = unsafe { std::slice::from_raw_parts(buffer as *const u8, n) };
        let mut data = self.data.borrow_mut();
        let pos = self.pos.get();
        if data.len() < pos + n {
            data.resize(pos + n, 0);
        }
        if let Some(dst) = data.get_mut(pos..pos + n) {
            dst.copy_from_slice(src);
        }
        self.pos.set(pos + n);
        // SAFETY: optional out-pointer, written only when not null.
        if let Some(w) = unsafe { written.as_mut() } {
            *w = n as int32;
        }
        kResultOk
    }
    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let len = self.data.borrow().len() as i64;
        let base = match mode as u32 {
            m if m == IBStream_::IStreamSeekMode_::kIBSeekSet as u32 => 0,
            m if m == IBStream_::IStreamSeekMode_::kIBSeekCur as u32 => self.pos.get() as i64,
            m if m == IBStream_::IStreamSeekMode_::kIBSeekEnd as u32 => len,
            _ => return kInvalidArgument,
        };
        let p = base.saturating_add(pos).clamp(0, len);
        self.pos.set(p as usize);
        // SAFETY: optional out-pointer, written only when not null.
        if let Some(r) = unsafe { result.as_mut() } {
            *r = p;
        }
        kResultOk
    }
    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        // SAFETY: optional out-pointer, written only when not null.
        if let Some(p) = unsafe { pos.as_mut() } {
            *p = self.pos.get() as i64;
        }
        kResultOk
    }
}

struct Queue {
    id: Cell<u32>,
    points: RefCell<Vec<(i32, f64)>>,
}
impl Class for Queue {
    type Interfaces = (IParamValueQueue,);
}
impl IParamValueQueueTrait for Queue {
    unsafe fn getParameterId(&self) -> u32 {
        self.id.get()
    }
    unsafe fn getPointCount(&self) -> int32 {
        self.points.borrow().len() as int32
    }
    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut f64) -> tresult {
        let pts = self.points.borrow();
        let Some(&(o, v)) = usize::try_from(index).ok().and_then(|i| pts.get(i)) else { return kInvalidArgument };
        // SAFETY: the plug-in's out-pointers; written only when not null.
        unsafe {
            if let Some(p) = offset.as_mut() {
                *p = o;
            }
            if let Some(p) = value.as_mut() {
                *p = v;
            }
        }
        kResultOk
    }
    unsafe fn addPoint(&self, offset: int32, value: f64, index: *mut int32) -> tresult {
        let mut pts = self.points.borrow_mut();
        if pts.len() >= 64 {
            return kResultFalse;
        }
        pts.push((offset, value));
        // SAFETY: optional out-pointer, written only when not null.
        if let Some(i) = unsafe { index.as_mut() } {
            *i = pts.len() as int32 - 1;
        }
        kResultOk
    }
}

/// Parameter changes for one process call (one point per parameter at offset 0).
struct Changes {
    queues: RefCell<Vec<ComWrapper<Queue>>>,
    used: Cell<usize>,
}
impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}
impl Changes {
    fn new() -> Changes {
        Changes { queues: RefCell::new(Vec::new()), used: Cell::new(0) }
    }
    fn set(&self, changes: &[(u32, f64)]) {
        let mut qs = self.queues.borrow_mut();
        while qs.len() < changes.len() {
            qs.push(ComWrapper::new(Queue { id: Cell::new(0), points: RefCell::new(Vec::new()) }));
        }
        for (q, (id, v)) in qs.iter().zip(changes) {
            q.id.set(*id);
            let mut p = q.points.borrow_mut();
            p.clear();
            p.push((0, v.clamp(0.0, 1.0)));
        }
        self.used.set(changes.len());
    }
}
impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> int32 {
        self.used.get() as int32
    }
    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        let qs = self.queues.borrow();
        let Some(i) = usize::try_from(index).ok().filter(|i| *i < self.used.get()) else { return std::ptr::null_mut() };
        qs.get(i).and_then(|q| q.as_com_ref::<IParamValueQueue>()).map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
    unsafe fn addParameterData(&self, _id: *const u32, _index: *mut int32) -> *mut IParamValueQueue {
        // output parameter changes are not read back
        std::ptr::null_mut()
    }
}

// ---- instances ----------------------------------------------------------------------------

struct Bus {
    channels: usize,
    data: Vec<Vec<f32>>,
    ptrs: Vec<*mut f32>,
}

impl Bus {
    fn new(channels: usize) -> Bus {
        let mut data: Vec<Vec<f32>> = (0..channels).map(|_| vec![0.0; MAX_BLOCK]).collect();
        let ptrs = data.iter_mut().map(|c| c.as_mut_ptr()).collect();
        Bus { channels, data, ptrs }
    }
}

/// A running plug-in instance (component, processor and controller).
pub struct Instance {
    comp: ComPtr<IComponent>,
    proc_: ComPtr<IAudioProcessor>,
    ctrl: Option<ComPtr<IEditController>>,
    /// The controller is a separate object (initialised and terminated on its own).
    separate_ctrl: bool,
    connections: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    _host: ComWrapper<HostApp>,
    _handler: ComWrapper<Handler>,
    changes: ComWrapper<Changes>,
    out_changes: ComWrapper<Changes>,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    sample_rate: f64,
    latency: u32,
    active: bool,
    _module: Module,
}

fn channels_of(arr: SpeakerArrangement) -> usize {
    (arr.count_ones() as usize).min(32)
}

impl Instance {
    fn new(module: &Module, cid: &[u8; 16], sample_rate: f64) -> Result<Instance, String> {
        if !(sample_rate.is_finite() && (8000.0..=768_000.0).contains(&sample_rate)) {
            return Err(format!("unsupported sample rate {sample_rate}"));
        }
        let comp: ComPtr<IComponent> = module.create(cid).ok_or("the plug-in did not create its component")?;
        let host = ComWrapper::new(HostApp);
        let host_ptr = host.as_com_ref::<IHostApplication>().map_or(std::ptr::null_mut(), |r| r.as_ptr()) as *mut FUnknown;
        // SAFETY: `host_ptr` points at our host object, alive for the instance's lifetime.
        if unsafe { comp.initialize(host_ptr) } != kResultOk {
            return Err("the plug-in failed to initialise".into());
        }
        let handler = ComWrapper::new(Handler);
        let mut inst = Instance {
            proc_: match comp.cast::<IAudioProcessor>() {
                Some(p) => p,
                None => {
                    // SAFETY: balanced with the `initialize` above.
                    unsafe { comp.terminate() };
                    return Err("the plug-in is not an audio processor".into());
                }
            },
            comp,
            ctrl: None,
            separate_ctrl: false,
            connections: None,
            _host: host,
            _handler: handler,
            changes: ComWrapper::new(Changes::new()),
            out_changes: ComWrapper::new(Changes::new()),
            inputs: Vec::new(),
            outputs: Vec::new(),
            sample_rate,
            latency: 0,
            active: false,
            _module: module.clone(),
        };
        inst.attach_controller(module, host_ptr);
        inst.setup()?;
        Ok(inst)
    }

    fn attach_controller(&mut self, module: &Module, host_ptr: *mut FUnknown) {
        if let Some(c) = self.comp.cast::<IEditController>() {
            self.ctrl = Some(c);
        } else {
            let mut tuid: TUID = [0; 16];
            // SAFETY: `tuid` is a writable 16-byte id.
            if unsafe { self.comp.getControllerClassId(&mut tuid) } == kResultOk {
                if let Some(c) = module.create::<IEditController>(&tuid_bytes(&tuid)) {
                    // SAFETY: as for the component.
                    if unsafe { c.initialize(host_ptr) } == kResultOk {
                        self.ctrl = Some(c);
                        self.separate_ctrl = true;
                    }
                }
            }
        }
        let Some(ctrl) = &self.ctrl else { return };
        if self.separate_ctrl {
            if let (Some(a), Some(b)) = (self.comp.cast::<IConnectionPoint>(), ctrl.cast::<IConnectionPoint>()) {
                // SAFETY: both are live connection points of this instance; disconnected on drop.
                unsafe {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                }
                self.connections = Some((a, b));
            }
            // hand the component's state to the controller so its parameters match
            let stream = ComWrapper::new(MemStream { data: RefCell::new(Vec::new()), pos: Cell::new(0) });
            if let Some(s) = stream.as_com_ref::<IBStream>() {
                // SAFETY: `s` is our stream object, alive across both calls.
                unsafe {
                    if self.comp.getState(s.as_ptr()) == kResultOk {
                        stream.pos.set(0);
                        ctrl.setComponentState(s.as_ptr());
                    }
                }
            }
        }
        if let Some(h) = self._handler.as_com_ref::<IComponentHandler>() {
            // SAFETY: the handler object lives as long as the instance.
            unsafe { ctrl.setComponentHandler(h.as_ptr()) };
        }
    }

    fn setup(&mut self) -> Result<(), String> {
        let audio = MediaTypes_::kAudio as i32;
        let (din, dout) = (BusDirections_::kInput as i32, BusDirections_::kOutput as i32);
        // SAFETY: plain queries on the live component.
        let (nin, nout) = unsafe { (self.comp.getBusCount(audio, din).clamp(0, MAX_BUSES), self.comp.getBusCount(audio, dout).clamp(0, MAX_BUSES)) };
        if nout < 1 {
            return Err("the plug-in has no audio output".into());
        }
        let mut ins: Vec<SpeakerArrangement> = (0..nin).map(|_| SpeakerArr::kStereo).collect();
        let mut outs: Vec<SpeakerArrangement> = (0..nout).map(|_| SpeakerArr::kStereo).collect();
        // SAFETY: the arrays hold `nin` / `nout` arrangements and live for the call.
        let ok = unsafe { self.proc_.setBusArrangements(ins.as_mut_ptr(), nin, outs.as_mut_ptr(), nout) };
        // whatever was accepted, use what the plug-in reports now
        for (i, a) in ins.iter_mut().enumerate() {
            // SAFETY: `a` is a writable arrangement.
            unsafe { self.proc_.getBusArrangement(din, i as i32, a) };
        }
        for (i, a) in outs.iter_mut().enumerate() {
            // SAFETY: as above.
            unsafe { self.proc_.getBusArrangement(dout, i as i32, a) };
        }
        let _ = ok;
        let main_out = outs.first().map_or(0, |a| channels_of(*a));
        if main_out == 0 {
            return Err("the plug-in's main output has no channels".into());
        }
        // SAFETY: plain calls on the live component; main buses on, the rest (side chains) off.
        unsafe {
            for i in 0..nin {
                self.comp.activateBus(audio, din, i, u8::from(i == 0));
            }
            for i in 0..nout {
                self.comp.activateBus(audio, dout, i, u8::from(i == 0));
            }
            if self.proc_.canProcessSampleSize(SymbolicSampleSizes_::kSample32 as i32) != kResultOk {
                return Err("the plug-in cannot process 32-bit audio".into());
            }
        }
        self.inputs = ins.iter().map(|a| Bus::new(channels_of(*a))).collect();
        self.outputs = outs.iter().map(|a| Bus::new(channels_of(*a))).collect();
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            maxSamplesPerBlock: MAX_BLOCK as i32,
            sampleRate: self.sample_rate,
        };
        // SAFETY: `setup` is a valid ProcessSetup for the call.
        if unsafe { self.proc_.setupProcessing(&mut setup) } != kResultOk {
            return Err("the plug-in refused the processing setup".into());
        }
        // SAFETY: plain calls on the live component.
        unsafe {
            if self.comp.setActive(1) != kResultOk {
                return Err("the plug-in failed to activate".into());
            }
            self.active = true;
            let r = self.proc_.setProcessing(1);
            if r != kResultOk && r != kNotImplemented && r != kResultFalse {
                return Err("the plug-in failed to start processing".into());
            }
            self.latency = self.proc_.getLatencySamples();
        }
        Ok(())
    }

    /// Processing latency in samples.
    pub fn latency(&self) -> u32 {
        self.latency
    }

    /// The plug-in's parameters (all of them; see [`ParamInfo::is_user_param`]).
    pub fn params(&self) -> Vec<ParamInfo> {
        let Some(c) = &self.ctrl else { return Vec::new() };
        // SAFETY: plain query.
        let n = unsafe { c.getParameterCount() }.clamp(0, MAX_PARAMS);
        let mut out = Vec::new();
        for i in 0..n {
            // SAFETY: ParameterInfo is plain data; zeroed is a valid value of it.
            let mut pi: ParameterInfo = unsafe { std::mem::zeroed() };
            // SAFETY: `pi` is writable for the call.
            if unsafe { c.getParameterInfo(i, &mut pi) } != kResultOk {
                continue;
            }
            let default = if pi.defaultNormalizedValue.is_finite() { pi.defaultNormalizedValue.clamp(0.0, 1.0) } else { 0.0 };
            out.push(ParamInfo { id: pi.id, title: wstr(&pi.title), units: wstr(&pi.units), step_count: pi.stepCount, default, flags: pi.flags });
        }
        out
    }

    /// The display string of parameter `id` at normalised `value` (e.g. "-12.0 dB").
    pub fn param_string(&self, id: u32, value: f64) -> Option<String> {
        let c = self.ctrl.as_ref()?;
        let mut s: String128 = [0; 128];
        // SAFETY: `s` is a writable String128.
        (unsafe { c.getParamStringByValue(id, value.clamp(0.0, 1.0), &mut s) } == kResultOk).then(|| wstr(&s))
    }

    /// Process `frames` samples of stereo audio: `left` / `right` are read and replaced by the
    /// plug-in's output. `changes` are (parameter id, normalised value) set at the start of the
    /// block. At most [`MAX_BLOCK`] samples per call (longer input is processed in pieces).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], changes: &[(u32, f64)]) -> Result<(), String> {
        let total = left.len().min(right.len());
        let mut done = 0;
        let mut first = true;
        while done < total {
            let n = (total - done).min(MAX_BLOCK);
            let (l, r) = (&mut left[done..done + n], &mut right[done..done + n]);
            self.process_block(l, r, if first { changes } else { &[] })?;
            first = false;
            done += n;
        }
        if total == 0 && !changes.is_empty() {
            self.process_block(&mut [], &mut [], changes)?;
        }
        Ok(())
    }

    fn process_block(&mut self, left: &mut [f32], right: &mut [f32], changes: &[(u32, f64)]) -> Result<(), String> {
        let n = left.len().min(right.len()).min(MAX_BLOCK);
        if let Some(c) = &self.ctrl {
            for (id, v) in changes {
                // SAFETY: plain call; keeps the controller's view in step with the processor.
                unsafe { c.setParamNormalized(*id, v.clamp(0.0, 1.0)) };
            }
        }
        self.changes.set(changes);
        self.out_changes.set(&[]);
        // inputs: main bus gets the audio (mono plug-ins get the mix), others silence
        for (bi, bus) in self.inputs.iter_mut().enumerate() {
            for (ci, ch) in bus.data.iter_mut().enumerate() {
                let Some(dst) = ch.get_mut(..n) else { continue };
                if bi == 0 {
                    match (bus.channels, ci) {
                        (1, _) => dst.iter_mut().zip(left.iter().zip(right.iter())).for_each(|(d, (a, b))| *d = 0.5 * (a + b)),
                        (_, 0) => dst.copy_from_slice(&left[..n]),
                        (_, 1) => dst.copy_from_slice(&right[..n]),
                        _ => dst.fill(0.0),
                    }
                } else {
                    dst.fill(0.0);
                }
            }
        }
        for bus in &mut self.outputs {
            for ch in &mut bus.data {
                if let Some(d) = ch.get_mut(..n) {
                    d.fill(0.0);
                }
            }
        }
        let mut ins: Vec<AudioBusBuffers> = self
            .inputs
            .iter_mut()
            .map(|b| AudioBusBuffers {
                numChannels: b.channels as i32,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 { channelBuffers32: b.ptrs.as_mut_ptr() },
            })
            .collect();
        let mut outs: Vec<AudioBusBuffers> = self
            .outputs
            .iter_mut()
            .map(|b| AudioBusBuffers {
                numChannels: b.channels as i32,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 { channelBuffers32: b.ptrs.as_mut_ptr() },
            })
            .collect();
        // SAFETY: ProcessContext is plain data; zeroed is a valid value of it.
        let mut ctx: ProcessContext = unsafe { std::mem::zeroed() };
        ctx.sampleRate = self.sample_rate;
        ctx.tempo = 120.0;
        ctx.timeSigNumerator = 4;
        ctx.timeSigDenominator = 4;
        let changes_ptr = self.changes.as_com_ref::<IParameterChanges>().map_or(std::ptr::null_mut(), |r| r.as_ptr());
        let out_changes_ptr = self.out_changes.as_com_ref::<IParameterChanges>().map_or(std::ptr::null_mut(), |r| r.as_ptr());
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: n as i32,
            numInputs: ins.len() as i32,
            numOutputs: outs.len() as i32,
            inputs: if ins.is_empty() { std::ptr::null_mut() } else { ins.as_mut_ptr() },
            outputs: outs.as_mut_ptr(),
            inputParameterChanges: changes_ptr,
            outputParameterChanges: out_changes_ptr,
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: &mut ctx,
        };
        // SAFETY: every buffer pointer in `data` points at `MAX_BLOCK` floats owned by this
        // instance's buses, the bus arrays and `ctx` outlive the call, and `n <= MAX_BLOCK`.
        let r = unsafe { self.proc_.process(&mut data) };
        if r != kResultOk {
            return Err(format!("the plug-in failed to process ({r})"));
        }
        let Some(main) = self.outputs.first() else { return Ok(()) };
        let l = main.data.first();
        let rr = main.data.get(1).or(l);
        if let (Some(l), Some(rr)) = (l, rr) {
            left[..n].copy_from_slice(&l[..n]);
            right[..n].copy_from_slice(&rr[..n]);
        }
        Ok(())
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: tear-down in the order the VST3 SDK host examples use, on live objects.
        unsafe {
            if self.active {
                self.proc_.setProcessing(0);
                self.comp.setActive(0);
            }
            if let Some((a, b)) = self.connections.take() {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(c) = self.ctrl.take() {
                if self.separate_ctrl {
                    c.terminate();
                }
            }
            self.comp.terminate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_and_ids() {
        assert_eq!(cstr(&[b'A' as i8, b'b' as i8, 0, b'x' as i8]), "Ab");
        let mut s: String128 = [0; 128];
        for (o, c) in s.iter_mut().zip("Gain".encode_utf16()) {
            *o = c;
        }
        assert_eq!(wstr(&s), "Gain");
        let id = [1u8, 2, 250, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9];
        assert_eq!(tuid_bytes(&bytes_tuid(&id)), id);
        assert_eq!(channels_of(SpeakerArr::kStereo), 2);
    }

    #[test]
    fn missing_module_is_an_error() {
        assert!(Module::load(Path::new("/nonexistent/Nope.vst3")).is_err());
    }

    #[test]
    fn mem_stream_round_trip() {
        let s = MemStream { data: RefCell::new(Vec::new()), pos: Cell::new(0) };
        let mut buf = *b"hello";
        let mut n = 0;
        // SAFETY: test buffers live for the calls.
        unsafe {
            assert_eq!(s.write(buf.as_mut_ptr() as *mut c_void, 5, &mut n), kResultOk);
            assert_eq!(n, 5);
            let mut p = 0;
            s.seek(0, IBStream_::IStreamSeekMode_::kIBSeekSet as i32, &mut p);
            let mut out = [0u8; 8];
            s.read(out.as_mut_ptr() as *mut c_void, 8, &mut n);
            assert_eq!(n, 5);
            assert_eq!(&out[..5], b"hello");
        }
    }
}
