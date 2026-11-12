//! Thin FFI over libngspice's shared API (see ngspice/sharedspice.h).
//! libngspice is a process-wide singleton, so every call goes through ENGINE.

use crate::schematic::Prepared;
use serde::Serialize;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

#[repr(C)]
struct Complex {
    re: f64,
    im: f64,
}

#[repr(C)]
struct VectorInfo {
    name: *const c_char,
    typ: c_int,
    flags: i16,
    real: *const f64,
    comp: *const Complex,
    len: c_int,
}

type CharCb = extern "C" fn(*const c_char, c_int, *mut c_void) -> c_int;
type ExitCb = extern "C" fn(c_int, bool, bool, c_int, *mut c_void) -> c_int;
type BgCb = extern "C" fn(bool, c_int, *mut c_void) -> c_int;

/// libngspice, loaded at runtime (not linked): the app starts without ngspice installed, finds
/// `ngspice.dll` on Windows, and can load a *fresh* copy after a fatal error (ngspice then
/// "awaits to be reset or detached" and never recovers in place).
struct Api {
    _lib: libloading::Library,
    command: unsafe extern "C" fn(*const c_char) -> c_int,
    circ: unsafe extern "C" fn(*const *const c_char) -> c_int,
    vec_info: unsafe extern "C" fn(*const c_char) -> *const VectorInfo,
    cur_plot: unsafe extern "C" fn() -> *const c_char,
    all_vecs: unsafe extern "C" fn(*const c_char) -> *const *const c_char,
    running: unsafe extern "C" fn() -> bool,
}

static ENGINE: Mutex<Option<Api>> = Mutex::new(None);
/// ngspice reported a fatal error (ControlledExit): the next call loads a fresh copy.
static DEAD: AtomicBool = AtomicBool::new(false);
static LOADS: AtomicUsize = AtomicUsize::new(0);
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
static BG_DONE: AtomicBool = AtomicBool::new(true);
/// Counts runs: each one destroys the results of the one before (see `Run::id`, `trace`).
static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The latest run: its id, its current sensors (trace expressions name part currents, `I(R1)`,
/// `Ib(Q1)`) and, for a `.step` run, each step's plot.
static LATEST: Mutex<Latest> = Mutex::new(Latest { id: 0, sensors: None, steps: None, pars: 0, shown: Vec::new(), ended: Vec::new() });
/// The latest run, for its traces and its .raw export: its id, current sensors, a .step run's
/// steps' plots, how many `.meas par()` helpers it has (their vectors are not results), the plots
/// its results are shown from (one per step) and, per step, its label and the plot ngspice ended
/// it in (a .noise run's totals, beside its spectrum).
#[derive(Clone)]
struct Latest {
    id: u64,
    sensors: Option<std::sync::Arc<[crate::schematic::Sensor]>>,
    steps: Option<std::sync::Arc<[String]>>,
    pars: usize,
    shown: Vec<String>,
    ended: Vec<(String, String)>,
}
/// Counts Halt presses, per window. An operation (a simulation, a root locus) of a window notes
/// its count when it begins (`begin`) and is halted once the count moves on (`halted_since`): a
/// Halt stops what that window was running (or waiting to run) when it was pressed, never
/// something started afterwards, nor another window's. Only the run thread acts on it (`bg_halt`,
/// see `run_until`), so libngspice is never entered concurrently.
static HALTS: Mutex<std::collections::BTreeMap<String, u64>> = Mutex::new(std::collections::BTreeMap::new());

fn halts(who: &str) -> u64 {
    HALTS.lock().map_or(0, |h| h.get(who).copied().unwrap_or(0))
}

/// An operation of window `who` begins (call it before netlisting, so a Halt during that counts too).
pub fn begin(who: &str) -> u64 {
    halts(who)
}

/// Whether window `who` pressed Halt since its operation that began at `op`.
pub fn halted_since(who: &str, op: u64) -> bool {
    halts(who) != op
}

extern "C" fn on_char(s: *const c_char, _: c_int, _: *mut c_void) -> c_int {
    let line = unsafe { CStr::from_ptr(s) }.to_string_lossy();
    let line = line.strip_prefix("stdout ").or_else(|| line.strip_prefix("stderr ")).unwrap_or(&line);
    if let Ok(mut log) = LOG.lock() {
        log.push(line.to_string());
    }
    0
}

extern "C" fn on_stat(_: *const c_char, _: c_int, _: *mut c_void) -> c_int {
    0
}

extern "C" fn on_exit(status: c_int, _: bool, _: bool, _: c_int, _: *mut c_void) -> c_int {
    DEAD.store(true, Ordering::SeqCst);
    RUNS.fetch_add(1, Ordering::SeqCst); // (its results are gone: run_id says so at once, before the reload)
    if let Ok(mut log) = LOG.lock() {
        log.push(format!("ngspice stopped after a fatal error (status {status}); it is reloaded for the next run"));
    }
    0
}

extern "C" fn on_bg(no_run: bool, _: c_int, _: *mut c_void) -> c_int {
    if no_run {
        BG_DONE.store(true, Ordering::SeqCst);
    }
    0
}

/// Where libngspice may live: $NGSPICE_LIB, next to the executable (bundled), package managers.
fn candidates() -> Vec<std::path::PathBuf> {
    let file = if cfg!(windows) { "ngspice.dll" } else if cfg!(target_os = "macos") { "libngspice.dylib" } else { "libngspice.so.0" };
    let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf()));
    let mut out: Vec<std::path::PathBuf> = std::env::var_os("NGSPICE_LIB").map(Into::into).into_iter().collect();
    if let Some(d) = exe {
        // bundled: next to the exe, macOS Frameworks/Resources, the Windows installer's ngspice/ folder
        // (ngspice's own Spice64_dll layout: dll-vs/ngspice.dll + lib/ngspice/*.cm)
        out.extend([d.join(file), d.join("../Frameworks").join(file), d.join("../Resources").join(file), d.join("resources").join(file), d.join("ngspice/dll-vs").join(file), d.join("ngspice/bin").join(file)]);
    }
    out.extend(
        ["/opt/homebrew/lib", "/usr/local/lib", "/opt/local/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu", "/usr/lib", "C:/Spice64_dll/dll-vs", "C:/Spice64/bin"]
            .map(|d| std::path::Path::new(d).join(file)),
    );
    out
}

/// Where reload copies go: set at startup to the app's cache dir (exec allowed, private);
/// falls back to the temp dir.
static SCRATCH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
pub fn set_scratch_dir(dir: std::path::PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = SCRATCH.set(dir);
}

/// Open a library by path. Windows: resolve its own imports (libomp, sndfile, samplerate) from its
/// folder, which the default search order skips.
unsafe fn open(p: &std::path::Path) -> Result<libloading::Library, String> {
    #[cfg(windows)]
    let lib = {
        let p = std::path::absolute(p).map_err(|e| e.to_string())?; // the flag needs an absolute path
        libloading::os::windows::Library::load_with_flags(p, libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH).map(Into::into)
    };
    #[cfg(not(windows))]
    let lib = libloading::Library::new(p);
    // libloading's message is just "LoadLibraryExW failed": add the OS reason
    lib.map_err(|e| match std::error::Error::source(&e) { Some(why) => format!("{e} ({why})"), None => e.to_string() })
}

fn load() -> Result<Api, String> {
    let n = LOADS.load(Ordering::SeqCst); // successful loads so far: > 0 means this is a reload
    if n == 0 {
        // copies left by earlier sessions (Windows cannot delete a loaded DLL)
        for e in SCRATCH.get().and_then(|d| std::fs::read_dir(d).ok()).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with("nomospice-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut errors = vec![];
    // the first file that exists *and loads* (a stale wrong-architecture copy must not block others)
    let found = candidates().into_iter().filter(|p| p.is_file()).find_map(|p| match unsafe { open(&p) } {
        Ok(lib) => Some((p, lib)),
        Err(e) => {
            errors.push(format!("{}: {e}", p.display()));
            None
        }
    });
    // code models (XSPICE: POLY sources, behavioural diodes, digital) live next to the library
    // (lib/ngspice beside libngspice) or, in ngspice's Windows package, in ../lib/ngspice
    let cm_dirs: Vec<std::path::PathBuf> = found.as_ref().and_then(|(p, _)| p.parent()).map(|d| vec![d.join("ngspice"), d.join("../lib/ngspice")]).unwrap_or_default();
    let lib = match found {
        // a reload must not get the dead image back from the loader's cache: load a fresh copy
        // `_orig` stays loaded until the copy is: its imports (Windows: libomp, sndfile, samplerate,
        // not beside the copy) resolve to the modules already in memory
        Some((path, _orig)) if n > 0 => {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
            let name = format!("nomospice-{}-{nanos}-{}", std::process::id(), path.file_name().unwrap().to_string_lossy());
            let dirs = SCRATCH.get().cloned().into_iter().chain([std::env::temp_dir()]);
            let mut last = String::new();
            let lib = dirs.map(|d| d.join(&name)).find_map(|copy| {
                let res = std::fs::copy(&path, &copy).map_err(|e| e.to_string())
                    .and_then(|_| unsafe { open(&copy) });
                // unix: the mapping stays valid after unlink, so no copy is left behind
                if cfg!(unix) || res.is_err() {
                    let _ = std::fs::remove_file(&copy);
                }
                res.map_err(|e| last = e).ok()
            });
            lib.ok_or(last)?
        }
        Some((_, lib)) => lib,
        // only reachable by name: cannot load a fresh copy, so a dead engine cannot be replaced
        None if n > 0 => return Err("ngspice stopped after a fatal error and cannot be reloaded from the system path: restart nomospice (or set NGSPICE_LIB)".into()),
        // not at a known path: let the system loader search (ld.so.cache, LD_LIBRARY_PATH, PATH)
        None => ["libngspice.so.0", "libngspice.so", "libngspice.dylib", "ngspice.dll"].iter()
            .find_map(|n| unsafe { libloading::Library::new(n) }.map_err(|e| errors.push(format!("{n}: {e}"))).ok())
            .ok_or_else(|| format!("libngspice not found or not loadable (install ngspice or set NGSPICE_LIB):\n{}", errors.join("\n")))?,
    };
    unsafe {
        macro_rules! sym { ($n:literal) => { *lib.get($n).map_err(|e| e.to_string())? } }
        let init: unsafe extern "C" fn(Option<CharCb>, Option<CharCb>, Option<ExitCb>, *const c_void, *const c_void, Option<BgCb>, *mut c_void) -> c_int = sym!(b"ngSpice_Init");
        let api = Api {
            command: sym!(b"ngSpice_Command"),
            circ: sym!(b"ngSpice_Circ"),
            vec_info: sym!(b"ngGet_Vec_Info"),
            cur_plot: sym!(b"ngSpice_CurPlot"),
            all_vecs: sym!(b"ngSpice_AllVecs"),
            running: sym!(b"ngSpice_running"),
            _lib: lib,
        };
        init(Some(on_char), Some(on_stat), Some(on_exit), null(), null(), Some(on_bg), null_mut());
        api.command("set ngbehavior=ltpsa");
        // libngspice runs spinit (which loads the code models) only from its compiled-in prefix;
        // packaged builds (Homebrew's Cellar path, a bundle, Spice64) often miss it: load them here.
        // Vendor macro models (PSpice POLY sources in nearly every opamp) depend on them.
        if !codemodels_loaded(&api) {
            for cm in ["spice2poly", "analog", "digital", "xtradev", "xtraevt", "table", "tlines"] {
                let f = cm_dirs.iter().map(|d| d.join(format!("{cm}.cm"))).find(|f| f.is_file());
                if let Some(f) = f {
                    // forward slashes in single quotes (C:/Program Files/...): ngspice's command lexer
                    // splits words on spaces, keeps double quotes literally, ignores `\ ` there
                    api.command(&format!("codemodel '{}'", f.display().to_string().replace('\\', "/")));
                }
            }
        }
        take_log(); // init chatter
        LOADS.fetch_add(1, Ordering::SeqCst);
        Ok(api)
    }
}

/// The engine, (re)loaded on demand.
fn engine() -> Result<MutexGuard<'static, Option<Api>>, Vec<String>> {
    let mut g = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() || DEAD.swap(false, Ordering::SeqCst) {
        // ponytail: the dead image is unloaded but its memory may stay mapped (~10 MB per fatal error)
        *g = None;
        *g = Some(load().map_err(|e| vec![e])?);
    }
    Ok(g)
}

/// Whether spinit already loaded the code models (a poly source then parses).
fn codemodels_loaded(api: &Api) -> bool {
    take_log();
    api.command("devhelp -csv");
    take_log().iter().any(|l| l.to_lowercase().contains("spice2poly"))
}

impl Api {
    fn command(&self, cmd: &str) -> bool {
        let c = CString::new(cmd).unwrap_or_default();
        unsafe { (self.command)(c.as_ptr()) == 0 }
    }
}

fn take_log() -> Vec<String> {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

#[derive(Serialize, Debug, Default)]
pub struct Run {
    /// Which run this is: only the latest one's vectors can be traced
    pub id: u64,
    /// ngspice plot name: tran1, ac1, dc1, op1 ...
    pub plot: String,
    /// Scale vector (time / frequency / *-sweep), empty for .op
    pub scale: String,
    pub vectors: Vec<String>,
    /// Plots without a scale (.op, .tf, .pz, .noise totals): every vector's value (re, im) and
    /// unit ("V"/"A" from ngspice's vector type, "" otherwise).
    pub op: Vec<(String, f64, f64, String)>,
    /// ngspice gave up part-way (e.g. "Timestep too small"): the data is partial.
    pub aborted: bool,
    /// `.meas` results: name, value (None: failed), the rest of ngspice's line (trig/targ, from/to),
    /// whether the value is on the x axis (a TRIG/TARG span, a WHEN point)
    pub meas: Vec<(String, Option<f64>, String, bool)>,
    pub log: Vec<String>,
    /// A `.step` run: each step's label (`R=1k`), in order; traces hold one curve per step.
    pub steps: Vec<String>,
    /// Said about the run beyond ngspice's log (a `.step` it could not run, so it ran once).
    pub notes: Vec<String>,
}

/// The deck's `.meas` statements (their + lines joined): name (lowercase, as ngspice reports it) →
/// whether the result is a point on the x axis (WHEN, not FIND/DERIV … WHEN).
fn meas_names(deck: &str) -> std::collections::HashMap<String, bool> {
    crate::schematic::statements(deck, &[".meas", ".measure"]).iter().filter_map(|l| {
        let t: Vec<String> = l.split_whitespace().map(str::to_ascii_lowercase).collect();
        let k = crate::schematic::meas_name_at(&t);
        let at_x = t.iter().skip(k + 1).any(|w| w == "when") && !t.iter().skip(k + 1).any(|w| ["find", "deriv", "derivative"].contains(&w.as_str()));
        t.first().filter(|w| w.starts_with(".meas")).and_then(|_| Some((t.get(k)?.clone(), at_x)))
    }).collect()
}

/// The `.meas` results in ngspice's report (`name = value targ= … trig= …`, `… failed!`), for the
/// deck's measurement `names` only: other `x = y` lines are not results. From the report's
/// "Measurements for … Analysis" heading on; older ngspice (36) prints none for AC: then all lines.
fn measurements(log: &[String], names: &std::collections::HashMap<String, bool>) -> Vec<(String, Option<f64>, String, bool)> {
    let mut out: Vec<(String, Option<f64>, String, bool)> = vec![];
    // from the report's heading where there is one (an earlier `x = y` echo is not a result)
    let from = log.iter().position(|l| l.trim().starts_with("Measurements for")).unwrap_or(0);
    for t in log[from..].iter().map(|l| l.trim()) {
        if t.is_empty() || t.starts_with("Measurements for") {
            continue;
        }
        let at_x = |n: &str| names.get(&n.to_ascii_lowercase()).copied();
        // `.meas tran x … failed!`, `meas x failed!` or `x failed!`
        if let Some(stmt) = t.strip_suffix("failed!") {
            let w: Vec<&str> = stmt.split_whitespace().collect();
            let name = if w.first().is_some_and(|f| f.starts_with('.')) { w.get(crate::schematic::meas_name_at(&w)) } else { w.last() };
            if let Some((name, x)) = name.and_then(|n| Some((n, at_x(n)?))) {
                if !out.iter().any(|o| o.0.eq_ignore_ascii_case(name)) {
                    out.push((name.to_string(), None, "failed".into(), x));
                }
            }
            continue;
        }
        // `name = <number> …` (also a name like error_pct)
        let result = t.split_once('=').and_then(|(n, rest)| {
            let x = at_x(n.trim())?;
            let rest = rest.trim();
            let (v, info) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            if v.eq_ignore_ascii_case("failed") {
                return Some((n.trim().to_string(), None, "failed".into(), x)); // `name = failed` (PARAM, expressions)
            }
            // a span (trig/targ) or a WHEN point lies on the x axis
            Some((n.trim().to_string(), Some(v.trim_end_matches(',').parse::<f64>().ok()?), info.trim().to_string(), x || info.contains("targ=")))
        });
        // the first result wins (a later line of the same name is not a second one); it replaces a
        // failure reported before it (a check pass)
        if let Some(r) = result {
            match out.iter().position(|o| o.0.eq_ignore_ascii_case(&r.0)) {
                Some(k) if out[k].1.is_none() && r.1.is_some() => out[k] = r,
                Some(_) => {}
                None => out.push(r),
            }
        }
    }
    out
}

#[derive(Serialize, Debug)]
pub struct Trace {
    pub x: Vec<f64>,
    pub re: Vec<f64>,
    pub im: Option<Vec<f64>>,
    /// a level: plain arithmetic of numbers (`0.5`, `3.3/2`), one real value spread over the x range,
    /// drawn as a reference line on the top pane. Not `vdd/2` (the node v(vdd)), nor a measured
    /// scalar with its own unit (`mean(i(vsense_r1))`, `v(out)[25]`)
    pub constant: bool,
    /// ngspice vector type (sim.h): 3 = voltage, 4 = current
    #[serde(skip)]
    pub typ: i32,
    /// a `.step` run: which steps (indices into `Run::steps`) its curves are, end to end
    pub steps: Option<Vec<usize>>,
}

fn read(api: &Api, name: &str) -> Option<Trace> {
    let c = CString::new(name).ok()?;
    let v = unsafe { (api.vec_info)(c.as_ptr()).as_ref()? };
    let n = v.len.max(0) as usize;
    if !v.real.is_null() {
        let re = unsafe { std::slice::from_raw_parts(v.real, n) }.to_vec();
        return Some(Trace { x: vec![], re, im: None, constant: false, typ: v.typ, steps: None });
    }
    if !v.comp.is_null() {
        let c = unsafe { std::slice::from_raw_parts(v.comp, n) };
        return Some(Trace { x: vec![], re: c.iter().map(|z| z.re).collect(), im: Some(c.iter().map(|z| z.im).collect()), constant: false, typ: v.typ, steps: None });
    }
    None
}

/// Load a netlist, run it to completion (background thread, so `halt` can stop it).
/// Takes a [`Prepared`] netlist only: includes inlined, `.control`/`*#` removed (see `schematic::prepare`).
pub fn run(netlist: &Prepared) -> Result<Run, Vec<String>> {
    run_until(netlist, &|| false)
}

/// [`run`], stopped (`bg_halt`, partial data) as soon as `stop()` says so: polled while it runs.
/// A deck with `.step` runs once per step; the steps' plots stay (traces read them all).
pub fn run_until(netlist: &Prepared, stop: &dyn Fn() -> bool) -> Result<Run, Vec<String>> {
    // (a .step it cannot run, a model parameter: the analysis once, as before, said in the log)
    // (also one whose first step cannot be made: `.step V9` of no such source)
    let (steps, unstepped) = match netlist.steps().and_then(|s| s.map(|s| s.at(0).map(|_| s)).transpose()) {
        Ok(s) => (s, None),
        Err(e) => (None, Some(format!("Warning: .step not run (the analysis runs once): {e}"))),
    };
    let sensors: std::sync::Arc<[crate::schematic::Sensor]> = netlist.sensors().into();
    let text = netlist.text(); // (the decks' statements: the same in every step)
    let g = engine()?;
    let api = g.as_ref().unwrap();
    api.command("destroy all");
    api.command("remcirc");
    // ponytail: a .step run keeps every step's plot in ngspice until the next run (traces read them
    // when asked): steps × full vectors in memory, bounded by MAX_STEPS; free them if that bites
    let id = RUNS.fetch_add(1, Ordering::SeqCst) + 1; // the previous run's results are gone now
    take_log();
    let total = steps.as_ref().map_or(1, |s| s.total);
    // (label, plot, log) per step done; the steps' logs in their order, failed ones too
    let mut done: Vec<(String, String, Vec<String>)> = vec![];
    let mut steplog: Vec<String> = vec![];
    for k in 0..total {
        if k > 0 && stop() {
            break; // (halted between steps: not a failure)
        }
        // each step's deck when it runs (a big deck: not a thousand copies at once)
        let step = steps.as_ref().map(|s| s.at(k)).transpose().map_err(|e| vec![format!(".step: {e}")]);
        let (label, deck) = match &step {
            Ok(Some((l, p))) => (l.as_str(), p.text()),
            Ok(None) => ("", text),
            Err(e) if done.is_empty() => return Err(unstepped.into_iter().chain(e.iter().cloned()).collect()),
            Err(e) => {
                steplog.extend(e.iter().cloned());
                break;
            }
        };
        if k > 0 {
            api.command("remcirc"); // (its plot stays)
        }
        // (a step whose analysis made no plot: ngspice's current plot is still the step before's;
        // halted before it made one: just halted)
        match one_run(api, deck, stop).and_then(|r| if !done.iter().any(|d| d.1 == r.0) { Ok(r) } else if r.2 { Err(vec!["simulation halted".into()]) } else { Err(r.1) }) {
            Ok((plot, log, halted)) => {
                steplog.push(format!("--- .step {label}"));
                steplog.extend(log.iter().cloned());
                done.push((label.to_string(), plot, log));
                if halted || stop() {
                    break;
                }
            }
            Err(log) if steps.is_none() => return Err(unstepped.into_iter().chain(log).collect()),
            Err(log) if log == ["simulation halted"] => break, // (halted while it began: not a failure)
            // a step that fails (R=0: singular): said, and the next ones still run, as in LTspice
            Err(log) => steplog.extend(std::iter::once(format!("--- .step {label} failed")).chain(log)),
        }
    }
    if done.is_empty() {
        return Err(unstepped.into_iter().chain(if steplog.is_empty() { vec!["simulation halted".into()] } else { steplog }).collect());
    }
    // .noise ends in its totals plot (noise2, noise4 …); the spectrum users plot is the one before it
    // (a step halted in its spectrum: that plot, it has a frequency scale)
    let has = |p: &str, v: &str| plot_vectors(api, p, &[], 0).0.iter().any(|x| x == v);
    let plots: Vec<String> = done.iter().map(|d| match d.1.strip_prefix("noise").and_then(|n| n.parse::<u32>().ok()) {
        Some(n) if n >= 2 && !has(&d.1, "frequency") => format!("noise{}", n - 1),
        _ => d.1.clone(),
    }).collect();
    // ngspice makes a helper source per par() in its output lines, named pa_00, pa_01 …
    // (cheap checks first, without a lowercase copy of the deck: most decks have neither)
    let pars = if !crate::schematic::contains_ci(text, "par(") { 0 } else {
        crate::schematic::statements(text, &crate::schematic::PAR_STATEMENTS).iter().map(|l| crate::schematic::par_calls(l)).sum()
    };
    *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = Latest {
        id,
        sensors: Some(sensors.clone()),
        steps: steps.is_some().then(|| plots.clone().into()),
        pars,
        shown: plots.clone(),
        ended: done.iter().map(|d| (d.0.clone(), d.1.clone())).collect(),
    };
    let plot0 = &done[0].1;
    let sensors = &sensors[..];
    // a sensor's current vector -> its name (`vsense_q1_2#branch` -> `Ib(Q1)`)
    let named: std::collections::HashMap<String, &str> = sensors.iter().map(|s| (format!("{}#branch", s.source), s.label.as_str())).collect();
    api.command(&format!("setplot {plot0}"));
    let (_, mut scale) = plot_vectors(api, plot0, sensors, pars);
    // one frequency (`.ac lin 1 50 50`): phasors, shown as a table like .op instead of a 1-point plot
    let phasors = scale == "frequency" && read(api, "frequency").is_some_and(|f| f.re.len() == 1);
    if phasors {
        scale.clear();
    }
    // no scale (.op, .tf, .pz, .noise totals ...): single values, shown as a table (per step, named)
    let mut op = vec![];
    let mut truncated = false;
    if scale.is_empty() {
        'steps: for (label, plot, _) in &done {
            api.command(&format!("setplot {plot}"));
            let (vs, sc) = plot_vectors(api, plot, sensors, pars);
            if !sc.is_empty() && !phasors {
                continue; // (a .noise step halted in its spectrum: no totals)
            }
            for v in vs {
                let Some(t) = read(api, &v) else { continue };
                // a sensor's current is reported by its name (`I(R1)`, `Ib(Q1)`)
                let name = named.get(&v).map_or(v.clone(), |l| l.to_string());
                let name = if label.is_empty() { name } else { format!("{name} [{label}]") };
                // types are reliable only for .op and .ac (ngspice marks .tf/.pz/.sens/noise values as "voltage")
                let unit = match (plot.starts_with("op") || phasors, t.typ) { (true, 3) => "V", (true, 4) => "A", _ => "" }.to_string();
                if let Some(&re) = t.re.first() {
                    op.push((name, re, t.im.and_then(|im| im.first().copied()).unwrap_or(0.0), unit));
                }
                if steps.is_some() && op.len() > MAX_ROWS {
                    op.pop();
                    truncated = true;
                    break 'steps;
                }
            }
        }
    }
    // the plot traces come from (the spectrum of a .noise), its vectors
    let plot = plots[0].clone();
    api.command(&format!("setplot {plot}"));
    let (vectors, scale2) = plot_vectors(api, &plot, sensors, pars);
    if !scale.is_empty() || plot != *plot0 {
        scale = scale2;
    }
    let log: Vec<String> = if steps.is_none() { unstepped.into_iter().chain(done[0].2.iter().cloned()).collect() } else { steplog };
    let aborted = log.iter().any(|l| {
        let l = l.to_lowercase();
        l.contains("aborted") || l.contains("timestep too small") || l.contains("simulation interrupted")
    }) || done.len() < total;
    // .meas: per step, named with it (`tdelay [R=1k]`)
    let meas = if !crate::schematic::contains_ci(text, ".meas") { vec![] } else {
        let names = meas_names(text);
        done.iter().flat_map(|(l, _, log)| measurements(log, &names).into_iter().map(move |m| if l.is_empty() { m } else { (format!("{} [{l}]", m.0), m.1, m.2, m.3) })).collect()
    };
    // sensor currents listed by their names, which trace expressions take (`I(R1)`, not `vsense_r1#branch`)
    let mut vectors: Vec<String> = vectors.into_iter().map(|v| named.get(&v).map_or(v, |l| l.to_string())).collect();
    vectors.sort();
    let mut notes: Vec<String> = vec![];
    let mut log = log;
    if truncated {
        notes.push(format!("Warning: the results table shows its first {MAX_ROWS} values (a value per node per step)"));
        log.push(notes[0].clone());
    }
    notes.extend(log.iter().filter(|l| l.starts_with("Warning: .step") || l.starts_with("--- .step") && l.ends_with("failed")).cloned());
    Ok(Run { id, plot, scale, vectors, op, aborted, meas, log, steps: if steps.is_none() { vec![] } else { done.iter().map(|d| d.0.clone()).collect() }, notes })
}

/// Load `deck`, run it (background thread, so `halt` can stop it): its plot, its log, whether it
/// was halted.
fn one_run(api: &Api, deck: &str, stop: &dyn Fn() -> bool) -> Result<(String, Vec<String>, bool), Vec<String>> {
    // (the last gate, whatever pass wrote the deck: no line ngspice would take for commands, nor a
    // file it would open itself, unread by the expander)
    let opens = |l: &str| [".inc", ".lib"].iter().any(|p| l.trim_start().get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)));
    if let Some(l) = deck.lines().find(|l| crate::schematic::command_line(l) || opens(l)) {
        return Err(vec![format!("`{l}`: not passed to ngspice (security)")]);
    }
    let lines: Vec<CString> = deck.lines().map(|l| CString::new(l).unwrap_or_default()).collect();
    let mut ptrs: Vec<*const c_char> = lines.iter().map(|l| l.as_ptr()).collect();
    ptrs.push(null());
    if unsafe { (api.circ)(ptrs.as_ptr()) } != 0 {
        return Err(take_log());
    }
    if stop() {
        return Err(vec!["simulation halted".into()]);
    }
    BG_DONE.store(false, Ordering::SeqCst);
    if !api.command("bg_run") {
        return Err(take_log());
    }
    let started = std::time::Instant::now();
    let mut halted = false;
    // BG_DONE comes from the callback; ngSpice_running() covers a thread that never started.
    while !BG_DONE.load(Ordering::SeqCst) && (started.elapsed() < Duration::from_millis(500) || unsafe { (api.running)() }) {
        if !halted && stop() {
            halted = true;
            api.command("bg_halt");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let plot = cstr(unsafe { (api.cur_plot)() });
    let log = take_log();
    if plot.is_empty() || plot == "const" {
        return Err(log);
    }
    Ok((plot, log, halted))
}

fn plot_vectors(api: &Api, plot: &str, sensors: &[crate::schematic::Sensor], pars: usize) -> (Vec<String>, String) {
    let mut vectors = all_vectors(api, plot);
    // current sensors (schematic.rs netlist_for_sim) are an implementation detail: their extra
    // nodes (copies of the pin-1 nets), their .sens entries (`vsense_r1_freq` ...) and a sensed
    // inductor's native branch (it misses parallel parasitics) are not listed
    const SENS: [&str; 7] = ["", "_freq", "_phase", "_pwr", "_z0", "_acmag", "_acphase"];
    vectors.retain(|v| !sensors.iter().any(|s| {
        &s.node == v || SENS.iter().any(|x| *v == format!("{}{x}", s.source)) || *v == format!("{}#branch", s.inst) || (!s.hold.is_empty() && *v == s.hold)
    }));
    // the helper sources ngspice makes for `.meas … par('…')`: pa_00, bpa_00#branch … one per par()
    let helpers: std::collections::HashSet<String> = (0..pars).flat_map(|k| [format!("pa_{k:02}"), format!("bpa_{k:02}#branch")]).collect();
    // (and a Laplace source's states, and the app's own trace scratch vector, should a trace have left it)
    vectors.retain(|v| !helpers.contains(v) && !crate::laplace::internal(v) && v != "nmsp_trace");
    vectors.sort();
    let scale = ["time", "frequency", "v-sweep", "i-sweep", "temp-sweep", "res-sweep"]
        .into_iter()
        .find(|s| !plot.starts_with("op") && vectors.iter().any(|v| v == s))
        .unwrap_or_default()
        .to_string();
    (vectors, scale)
}

/// Every vector of `plot`, as ngspice has them.
fn all_vectors(api: &Api, plot: &str) -> Vec<String> {
    let mut out = vec![];
    let cplot = CString::new(plot).unwrap_or_default();
    let mut p = unsafe { (api.all_vecs)(cplot.as_ptr()) };
    while !p.is_null() && unsafe { !(*p).is_null() } {
        out.push(cstr(unsafe { *p }));
        p = unsafe { p.add(1) };
    }
    out
}

/// Now as a .raw file's Date line has it: `Sun Nov  8 04:30:06 2026` (UTC).
fn raw_date() -> String {
    raw_date_at(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64)
}
fn raw_date_at(t: i64) -> String {
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // (civil from days, H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + i64::from(m <= 2);
    const WD: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MO: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {} {d:2} {:02}:{:02}:{:02} {y}", WD[days.rem_euclid(7) as usize], MO[(m - 1) as usize], secs / 3600, secs / 60 % 60, secs % 60)
}

/// What a .raw file calls ngspice's plot (`tran1`: its analysis, as ngspice and LTspice write it).
fn raw_plotname(plot: &str) -> &'static str {
    match plot.trim_end_matches(|c: char| c.is_ascii_digit()) {
        "tran" => "Transient Analysis",
        "ac" => "AC Analysis",
        "dc" => "DC transfer characteristic",
        "op" => "Operating Point",
        "noise" => "Noise Spectral Density Curves",
        "sens" => "Sensitivity Analysis",
        "tf" => "Transfer Function",
        "pz" => "Pole-Zero Analysis",
        _ => "Analysis",
    }
}

/// ponytail: the whole file is built in memory under the engine lock; stream it plot by plot if
/// exports of many big steps come to need it.
/// Run `run`'s results as a .raw file in ngspice's binary format (a text header, then each point's
/// values as little-endian doubles; complex: real and imaginary), which other tools read: the
/// vectors the results list (`v(out)`, `i(v1)`), the part currents under their names (`I(R1)`,
/// `Ib(Q1)`), a .step run's steps one plot each, in their order, named by their step. Written
/// here from the vectors as read: ngspice's results, settings and current plot are not changed.
pub fn export_raw(run: u64) -> Result<(Vec<u8>, Vec<String>), String> {
    const GONE: &str = "the results were replaced by a later run (a root locus or another simulation): run the simulation again";
    let latest = LATEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if latest.id != run {
        return Err(GONE.into());
    }
    let sensors = latest.sensors.clone().unwrap_or_else(|| std::sync::Arc::from([]));
    let named: std::collections::HashMap<String, &str> = sensors.iter().map(|s| (format!("{}#branch", s.source), s.label.as_str())).collect();
    let g = engine().map_err(|e| e.join("\n"))?;
    let api = g.as_ref().unwrap();
    if RUNS.load(Ordering::SeqCst) != run {
        return Err(GONE.into()); // (another run since, or ngspice reloaded: its results are gone)
    }
    // the run's plots, one per step as shown, and a .noise run's totals beside each spectrum;
    // the plot ngspice reads from now put back afterwards
    let current = cstr(unsafe { (api.cur_plot)() });
    let stepped = latest.steps.as_ref().is_some_and(|s| !s.is_empty());
    let plots: Vec<(String, String)> = latest.shown.iter().enumerate().flat_map(|(k, p)| {
        let (label, end) = latest.ended.get(k).cloned().unwrap_or_default();
        let label = if stepped { format!(" ({label})") } else { String::new() };
        std::iter::once((p.clone(), label.clone())).chain((!end.is_empty() && end != *p && p.starts_with("noise")).then(|| (end, label)))
    }).collect();
    let pars = latest.pars;
    let mut dropped = std::collections::BTreeSet::new();
    let mut out = vec![];
    let date = raw_date();
    for (p, step) in &plots {
        api.command(&format!("setplot {p}"));
        let (vectors, scale) = plot_vectors(api, p, &sensors, pars);
        // the scale first; the app's own scratch vector (nmsp_trace) not a result
        let order = std::iter::once(scale.clone()).filter(|s| !s.is_empty()).chain(vectors.into_iter().filter(|v| *v != scale));
        let mut cols: Vec<(String, Trace)> = vec![];
        for v in order {
            match read(api, &v) {
                Some(t) => cols.push((v, t)),
                None => {
                    dropped.insert(v);
                }
            }
        }
        // (the scale unreadable: no plot to write; a column of another length than the scale's, or
        // than most, without one: not this plot's data, said, but for single values, a .meas's)
        if !scale.is_empty() && cols.first().map(|c| &c.0) != Some(&scale) {
            return Err(format!("{p}: ngspice gives no {scale} to write the results over"));
        }
        let n = if scale.is_empty() { cols.iter().map(|c| c.1.re.len()).max().unwrap_or(0) } else { cols.first().map_or(0, |c| c.1.re.len()) };
        let (cols, other): (Vec<(String, Trace)>, Vec<(String, Trace)>) = cols.into_iter().partition(|c| c.1.re.len() == n);
        dropped.extend(other.into_iter().filter(|c| c.1.re.len() != 1).map(|c| c.0));
        let complex = cols.iter().any(|c| c.1.im.is_some());
        // (Command: ngspice: readers take ngspice's layout by it, all values doubles)
        // (a .noise run's totals: integrated, no frequency scale)
        let plotname = if p.starts_with("noise") && scale.is_empty() { "Integrated Noise" } else { raw_plotname(p) };
        out.extend(format!("Title: nomospice\nDate: {date}\nCommand: ngspice (nomospice)\nPlotname: {plotname}{step}\nFlags: {}\nNo. Variables: {}\nNo. Points: {n}\nVariables:\n", if complex { "complex" } else { "real" }, cols.len()).bytes());
        // (v(…) only for the nodes of an analysis over time, frequency or a sweep, or .op: ngspice
        // types .tf, .sens and .pz values as voltages too)
        let nodes = matches!(p.trim_end_matches(|c: char| c.is_ascii_digit()), "tran" | "ac" | "dc" | "op");
        for (j, (v, t)) in cols.iter().enumerate() {
            // (ngspice's names: a node v(out), a branch i(v1); a part current by its own, I(R1))
            let name = match (named.get(v), v.strip_suffix("#branch")) {
                (Some(l), _) => l.to_string(),
                (None, Some(b)) => format!("i({b})"),
                (None, None) if *v == scale => v.clone(),
                (None, None) if nodes && t.typ == 3 && !v.contains(['(', '#', '@', '[']) => format!("v({v})"),
                _ => v.clone(),
            };
            // (ngspice's type names, as its own raw files have them)
            const TYPES: [&str; 23] = ["notype", "time", "frequency", "voltage", "current", "voltage-density", "current-density", "sqr-voltage-density", "sqr-current-density", "sqr-voltage", "sqr-current", "pole", "zero", "s-param", "temp-sweep", "res-sweep", "impedance", "admittance", "power", "phase", "decibel", "capacitance", "charge"];
            let kind = TYPES.get(t.typ as usize).copied().unwrap_or("notype");
            out.extend(format!("\t{j}\t{name}\t{kind}\n").bytes());
        }
        out.extend(b"Binary:\n");
        for i in 0..n {
            for (_, t) in &cols {
                out.extend(t.re[i].to_le_bytes());
                if complex {
                    out.extend(t.im.as_ref().map_or(0.0, |im| im[i]).to_le_bytes());
                }
            }
        }
    }
    api.command(&format!("setplot {current}"));
    Ok((out, dropped.into_iter().collect()))
}

/// Evaluate an ngspice expression (e.g. `v(out)`, `i(vsense_r1)`, `v(a)-v(b)`) in the results of
/// run `run` (an error once another run, such as a root locus step, replaced them).
pub fn trace(expr: &str, scale: &str, run: u64) -> Result<Trace, String> {
    // part currents by name (`I(R1)-I(R2)`, `Ib(Q1)`): the latest run's sensors; messages show the
    // expression as written, ngspice's with the names (not the hidden sensor sources)
    // (the sensors of the run asked about; another run's: the results are gone anyway, see below)
    let Latest { id: of, sensors, steps, .. } = LATEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let sensors = sensors.filter(|_| of == run).unwrap_or_else(|| std::sync::Arc::from([]));
    let steps: std::sync::Arc<[String]> = steps.filter(|_| of == run).unwrap_or_else(|| std::sync::Arc::from([])); // (another run's: gone, the RUNS check below says so)
    let (named, problem) = crate::schematic::sensor_currents(expr, &sensors);
    let (shown, expr) = (expr, named.as_str());
    let unname = |log: String| sensors.iter().fold(log, |l, s| l.replace(&format!("i({})", s.source), &s.label).replace(&format!("{}#branch", s.source), &s.label));
    // ngspice's command line is csh-like (`;` chains, backquotes run sh, `!` history, `$` vars):
    // only characters an expression needs may pass.
    let ok = |c: char| c.is_ascii_alphanumeric() || " _()[]@#+-*/^.,:\"'<>=&|%".contains(c);
    if !expr.chars().all(ok) || expr.trim().is_empty() {
        return Err(format!("invalid expression: {shown}"));
    }
    let g = engine().map_err(|e| e.join("\n"))?;
    let api = g.as_ref().unwrap();
    if RUNS.load(Ordering::SeqCst) != run {
        return Err("the results were replaced by a later run (a root locus or another simulation): run the simulation again".into());
    }
    if let Some(why) = problem {
        return Err(why); // (a part current the circuit has not, or a subcircuit's I(U1))
    }
    // the plot as it is, or (a .step run) each step's in turn
    let mut each = vec![];
    for plot in steps.iter().map(Some).chain(steps.is_empty().then_some(None)) {
        if let Some(p) = plot {
            api.command(&format!("setplot {p}"));
        }
        api.command("unlet nmsp_trace"); // a failed `let` would otherwise leave the previous trace behind
        take_log(); // after unlet: it may print leftovers of the finished run ("Background thread stopped")
        api.command(&format!("let nmsp_trace = {expr}"));
        let mut t = match read(api, "nmsp_trace") {
            Some(t) => t,
            None => {
                let log = take_log().join("\n");
                let why = if log.is_empty() { format!("cannot evaluate {shown}") } else { unname(log) };
                if steps.is_empty() || each.is_empty() {
                    return Err(why); // (the first step already: the expression, not a step's data)
                }
                // one step of several (halted, aborted: no data there): a gap; all of them: the error
                let _ = why; // (the first step already returned it: the expression is fine, this step has no data)
                Trace { x: vec![], re: vec![], im: None, constant: false, typ: 0, steps: None }
            }
        };
        if !scale.is_empty() {
            t.x = read(api, scale).map(|s| s.re).unwrap_or_default();
        }
        // a constant (`1`, a reference level) is one value: a line across the x range
        if t.re.len() == 1 && t.x.len() > 1 {
            let n = t.x.len();
            t.constant = t.im.is_none() && numbers_only(expr);
            t.re = vec![t.re[0]; n];
            if let Some(im) = &mut t.im {
                *im = vec![im[0]; n];
            }
        }
        api.command("unlet nmsp_trace"); // (read: not left in the plot, among the results, a .raw export)
        if !steps.is_empty() {
            // each step thinned as it is read (its own samples, evenly): steps × points stays bounded
            let cap = 4 * (STEP_VALUES / steps.len()).clamp(200, 200_000);
            let sweep = t.x.windows(2).all(|w| w[1] >= w[0]) || t.x.windows(2).all(|w| w[1] <= w[0]); // (not a nested .dc)
            // (a gap step's x too: thinned like the others', so equal grids stay equal)
            if t.x.len() > cap && (t.re.len() == t.x.len() || t.re.is_empty()) && sweep {
                let keep = even(t.x.len(), cap);
                let pick = |v: &[f64]| if v.is_empty() { vec![] } else { keep.iter().map(|&k| v[k]).collect::<Vec<f64>>() };
                (t.x, t.re, t.im) = (pick(&t.x), pick(&t.re), t.im.as_deref().map(pick));
            }
        }
        each.push((each.len(), t));
    }
    if each.len() == 1 {
        let (k, mut t) = each.pop().unwrap();
        t.steps = (!steps.is_empty()).then(|| vec![k]); // (a .step run of one step: named like the others)
        Ok(t)
    } else {
        steps_joined(each)
    }
}

/// The steps' curves of one trace as one, end to end over a shared x (each step's own where they
/// agree, a .ac or .dc; else, .tran picking its own time points, the union of their x, each step
/// interpolated onto it), so the plot shows them as a family like a nested .dc. At most
/// `STEP_VALUES` values in all: many steps thin their curves (each keeps a share of its own samples).
const STEP_VALUES: usize = 2_000_000;
/// `n` indices spread evenly over `0..len` (the first and the last among them); all if `len <= n`.
pub fn even(len: usize, n: usize) -> Vec<usize> {
    if len <= n || n < 2 { (0..len).collect() } else { (0..n).map(|k| k * (len - 1) / (n - 1)).collect() }
}

/// The results table's rows at most (a stepped .op of a big circuit: a value per node per step).
const MAX_ROWS: usize = 20_000;
fn steps_joined(each: Vec<(usize, Trace)>) -> Result<Trace, String> {
    let (index, each): (Vec<usize>, Vec<Trace>) = each.into_iter().unzip();
    let same = each.windows(2).all(|w| w[0].x.len() == w[1].x.len() && w[0].x.iter().zip(&w[1].x).all(|(a, b)| (a - b).abs() <= 1e-12 * a.abs().max(b.abs())));
    // (a sweep down, `.dc V1 5 0 -0.1`: worked on as -x)
    let dir = if each[0].x.len() > 1 && each[0].x[1] < each[0].x[0] { -1.0 } else { 1.0 };
    let rising = |x: &[f64]| x.windows(2).all(|w| dir * w[1] >= dir * w[0]);
    if !same && !each.iter().all(|t| rising(&t.x)) {
        // a nested .dc: steps halted early (their x the start of the others') left out; steps that
        // sweep other ranges have no shared axis
        let first = each[0].x.clone();
        if each.iter().any(|t| t.x != first && !first.starts_with(&t.x)) {
            return Err("the steps sweep different ranges (a nested .dc): no shared axis to plot them on".into());
        }
        let whole: Vec<(usize, Trace)> = index.into_iter().zip(each).filter(|(_, t)| t.x == first).collect();
        return if whole.len() > 1 { steps_joined(whole) } else { whole.into_iter().next().map(|(k, mut t)| (t.steps = Some(vec![k]), t).1).ok_or_else(|| "no step to show".to_string()) };
    }
    // (a curve's share of the budget; evenly by index, keeping the first and last)
    let cap = (STEP_VALUES / each.len()).clamp(200, 200_000);
    let thin = |v: &[f64], n: usize| even(v.len(), n);
    if same {
        // the same x for all (.ac, .dc): every curve at the same thinned indices, exact; a table
        // (.op: no x, a value per step) or a nested .dc (thinning would cut across its segments) whole
        let len = each.iter().map(|t| t.x.len().max(t.re.len())).max().unwrap_or(0);
        // (a nested .dc, not thinned: as many whole steps as the budget holds)
        let (index, each) = if !each[0].x.is_empty() && !rising(&each[0].x) && len * each.len() > STEP_VALUES {
            let n = (STEP_VALUES / len.max(1)).max(1);
            (index.into_iter().take(n).collect::<Vec<_>>(), each.into_iter().take(n).collect::<Vec<_>>())
        } else {
            (index, each)
        };
        let keep: Vec<usize> = if each[0].x.is_empty() || !rising(&each[0].x) { (0..len).collect() } else { thin(&each[0].x, cap) };
        // (missing values, a curve shorter than its x, `v(out)[0,9]`: gaps, not a panic)
        // (a gap step, no data: NaN of the full length, so the curves after it keep their places)
        let pick = |v: &[f64]| keep.iter().map(|&k| v.get(k).copied().unwrap_or(f64::NAN)).collect::<Vec<f64>>();
        let complex = each.iter().filter(|t| !t.re.is_empty()).all(|t| t.im.is_some());
        return Ok(Trace {
            x: if each[0].x.is_empty() { vec![] } else { each.iter().flat_map(|_| pick(&each[0].x)).collect() },
            re: each.iter().flat_map(|t| pick(&t.re)).collect(),
            im: complex.then(|| each.iter().flat_map(|t| pick(t.im.as_deref().unwrap_or_default())).collect()),
            constant: each.iter().all(|t| t.constant),
            typ: each[0].typ,
            steps: Some(index),
        });
    }
    // the union of the steps' x, each thinned to its share first: every curve keeps samples of its
    // own; very many steps (a share under 20): the whole union thinned evenly instead
    let per = cap / each.len();
    let mut grid: Vec<f64> = each.iter().flat_map(|t| thin(&t.x, if per >= 20 { per } else { usize::MAX }).into_iter().map(|k| t.x[k])).collect();
    grid.sort_by(|a, b| (dir * a).total_cmp(&(dir * b)));
    grid.dedup_by(|a, b| (*a - *b).abs() <= 1e-12 * a.abs().max(b.abs()));
    if grid.len() > cap {
        grid = thin(&grid, cap).into_iter().map(|k| grid[k]).collect();
    }
    // linear between samples, the end value beyond (a step that stopped early), none where a step
    // has no data (a gap)
    let onto = |x: &[f64], y: &[f64]| -> Vec<f64> {
        if x.len() < 2 || y.len() < x.len() {
            return vec![f64::NAN; grid.len()]; // (no curve: a gap; NaN reaches the UI as null)
        }
        let (lo, hi) = (dir * x[0], dir * x[x.len() - 1]);
        let mut k = 0;
        grid.iter().map(|&g| {
            let g = dir * g;
            // beyond what the step simulated (halted, aborted): a gap, not its last value held
            if g < lo - 1e-12 * lo.abs() || g > hi + 1e-12 * hi.abs() {
                return f64::NAN;
            }
            while k + 2 < x.len() && dir * x[k + 1] < g {
                k += 1;
            }
            let (x0, x1) = (dir * x[k], dir * x[k + 1]);
            let f = if x1 == x0 { 0.0 } else { ((g - x0) / (x1 - x0)).clamp(0.0, 1.0) };
            y[k] + f * (y[k + 1] - y[k])
        }).collect()
    };
    let complex = each.iter().filter(|t| !t.re.is_empty()).all(|t| t.im.is_some()); // (a gap step has none)
    Ok(Trace {
        x: each.iter().flat_map(|_| grid.iter().copied()).collect(),
        re: each.iter().flat_map(|t| onto(&t.x, &t.re)).collect(),
        im: complex.then(|| each.iter().flat_map(|t| onto(&t.x, t.im.as_deref().unwrap_or_default())).collect()),
        constant: each.iter().all(|t| t.constant),
        typ: each[0].typ,
        steps: Some(index),
    })
}

/// Only numbers (exponents, SPICE suffixes: `1e-3`, `10k`), operators and parentheses: a level the
/// user typed, not a vector, a measurement or a function of them (`tdelay`, `mean(i(r1))`).
fn numbers_only(expr: &str) -> bool {
    let b = expr.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() || b[i] == b'.' {
            i += 1; // the number, its exponent sign and its suffix
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || (b"+-".contains(&b[i]) && b"eE".contains(&b[i - 1]))) {
                i += 1;
            }
        } else if b" +-*/^()".contains(&b[i]) {
            i += 1;
        } else {
            return false;
        }
    }
    true
}

/// The latest run's id: the one whose results ngspice holds (a trace of another fails).
pub fn current_run() -> u64 {
    RUNS.load(Ordering::SeqCst)
}

/// Request a stop of what window `who` runs now; `run_until` sends `bg_halt` from its own thread.
pub fn halt(who: &str) {
    if let Ok(mut h) = HALTS.lock() {
        *h.entry(who.to_string()).or_insert(0) += 1;
    }
}

/// Tests that run simulations share the one ngspice engine: they take this lock.
#[cfg(test)]
pub static TEST_SIM: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_report() {
        let log: Vec<String> = ["Measurements for Transient Analysis", "ratio               =   failed", "error_pct           =  1.20000e-02", "Error: measure  w  (WHEN) : out of interval", "    Analysis is not possible.", " meas w failed!", ".meas tran p avg -i(v1) failed!", "tplh                =  5.4e-10 targ=  3.0e-08 trig=  3.0e-08", "tnom = 27"]
            .map(String::from).into();
        let names = meas_names(".meas tran ratio PARAM a/b\n.meas tran error_pct AVG x ; when settled\n.meas tran\n* note\n+ w WHEN v(a)=1\n.MEAS p AVG -i(v1)\n.meas tran tplh TRIG …");
        assert_eq!(names.get("error_pct"), Some(&false), "a comment's words do not count");
        assert_eq!(names.get("w"), Some(&true), "a name on a + line; WHEN is on the x axis");
        let m = measurements(&log, &names);
        let names: Vec<&str> = m.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(names, ["ratio", "error_pct", "w", "p", "tplh"], "{m:?}");
        assert!(m[0].1.is_none() && m[4].3 && !m[1].3, "a failed PARAM listed; trig/targ on the x axis, AVG not");
        assert_eq!(m[4].2, "targ=  3.0e-08 trig=  3.0e-08");
        // ngspice 36 reports AC results without the heading
        let log: Vec<String> = ["Warning: v1: no DC value, transient time 0 value used", ".meas ac p2 max mag(v(out)) failed!", "peak                =  3.174444e+00 at=  5.011872e+02"].map(String::from).into();
        let m = measurements(&log, &meas_names(".meas ac peak MAX vm(out)\n.meas ac p2 MAX mag(v(out))"));
        assert_eq!((m.len(), m[0].0.as_str(), m[1].1), (2, "p2", Some(3.174444)), "{m:?}");
        // a failure printed before the result (a check pass): the result counts, once
        let log: Vec<String> = [".meas tran t1 TRIG v(a) VAL=1 RISE=1 TARG v(b) VAL=1 RISE=1 failed!", "t1 failed!", "t1                  =  3e-08 targ=  4e-08 trig=  1e-08"].map(String::from).into();
        let m = measurements(&log, &meas_names(".meas tran t1 TRIG v(a) VAL=1 RISE=1 TARG v(b) VAL=1 RISE=1"));
        assert_eq!((m.len(), m[0].1), (1, Some(3e-8)), "{m:?}");
    }

    #[test]
    fn levels_are_numbers_only() {
        for e in ["0.5", "-3", "500m", "1e-3*2", "2.5E+2", "(1+2)*0.5", "10k/2", "3.3 / 2"] {
            assert!(numbers_only(e), "{e}");
        }
        for e in ["tdelay", "ratio*2", "vdd/2", "mean(v(in))", "v(out)[25]", "2*pi", "sqrt(2)"] {
            assert!(!numbers_only(e), "{e}");
        }
    }

    #[test]
    fn raw_dates() {
        assert_eq!(raw_date_at(0), "Thu Jan  1 00:00:00 1970");
        assert_eq!(raw_date_at(1794112206), "Sun Nov  8 04:30:06 2026");
        assert_eq!(raw_date_at(951868799), "Tue Feb 29 23:59:59 2000");
    }

    #[test]
    fn codemodel_path_with_spaces_loads() {
        let _s = TEST_SIM.lock();
        let Some(cm) = ["/opt/homebrew/lib/ngspice/table.cm", "/usr/lib/x86_64-linux-gnu/ngspice/table.cm"].into_iter().find(|p| std::path::Path::new(p).is_file()) else { return };
        let dir = std::env::temp_dir().join(format!("nomospice cm {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("table copy.cm");
        std::fs::copy(cm, &f).unwrap();
        let g = engine().unwrap();
        take_log();
        g.as_ref().unwrap().command(&format!("codemodel '{}'", f.display().to_string().replace('\\', "/")));
        let log = take_log().join("\n").to_lowercase();
        assert!(!log.contains("too many args") && !log.contains("couldn't be loaded") && !log.contains("error"), "{log}");
        std::fs::remove_dir_all(dir).ok();
    }
}
