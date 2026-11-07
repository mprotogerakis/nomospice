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
/// Counts Halt presses. An operation (a simulation, a root locus) notes the count when it begins
/// (`begin`) and is halted once the count moves on (`halted_since`): a Halt stops what was running
/// when it was pressed, never something started afterwards, and no later start can undo it. Only
/// the run thread acts on it (`bg_halt`, see `run_until`), so libngspice is never entered concurrently.
static HALTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// An operation begins (call it before netlisting, so a Halt during that counts too).
pub fn begin() -> u64 {
    HALTS.load(Ordering::SeqCst)
}

/// Whether Halt was pressed since the operation that began at `op`.
pub fn halted_since(op: u64) -> bool {
    HALTS.load(Ordering::SeqCst) != op
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
/// deck's measurement `names` only: ngspice prints failure messages before the results, and other
/// `x = y` lines are not results.
fn measurements(log: &[String], names: &std::collections::HashMap<String, bool>) -> Vec<(String, Option<f64>, String, bool)> {
    let mut out = vec![];
    let mut on = false;
    for t in log.iter().map(|l| l.trim()) {
        if t.starts_with("Measurements for") {
            on = true;
            continue;
        }
        if !on || t.is_empty() {
            continue;
        }
        let at_x = |n: &str| names.get(&n.to_ascii_lowercase()).copied();
        // `.meas tran x … failed!`, `meas x failed!` or `x failed!`
        if let Some(stmt) = t.strip_suffix("failed!") {
            let w: Vec<&str> = stmt.split_whitespace().collect();
            let name = if w.first().is_some_and(|f| f.starts_with('.')) { w.get(crate::schematic::meas_name_at(&w)) } else { w.last() };
            if let Some((name, x)) = name.and_then(|n| Some((n, at_x(n)?))) {
                out.push((name.to_string(), None, "failed".into(), x));
            }
            continue;
        }
        // `name = <number> …` (also a name like error_pct)
        let result = t.split_once('=').and_then(|(n, rest)| {
            let x = at_x(n.trim())?;
            let rest = rest.trim();
            let (v, info) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            // a span (trig/targ) or a WHEN point lies on the x axis
            Some((n.trim().to_string(), Some(v.trim_end_matches(',').parse::<f64>().ok()?), info.trim().to_string(), x || info.contains("targ=")))
        });
        // (first one wins: a later line of the same name in the log is not a second result)
        out.extend(result.filter(|r| !out.iter().any(|o| o.0.eq_ignore_ascii_case(&r.0))));
    }
    out
}

#[derive(Serialize, Debug)]
pub struct Trace {
    pub x: Vec<f64>,
    pub re: Vec<f64>,
    pub im: Option<Vec<f64>>,
    /// ngspice vector type (sim.h): 3 = voltage, 4 = current
    #[serde(skip)]
    pub typ: i32,
}

fn read(api: &Api, name: &str) -> Option<Trace> {
    let c = CString::new(name).ok()?;
    let v = unsafe { (api.vec_info)(c.as_ptr()).as_ref()? };
    let n = v.len.max(0) as usize;
    if !v.real.is_null() {
        let re = unsafe { std::slice::from_raw_parts(v.real, n) }.to_vec();
        return Some(Trace { x: vec![], re, im: None, typ: v.typ });
    }
    if !v.comp.is_null() {
        let c = unsafe { std::slice::from_raw_parts(v.comp, n) };
        return Some(Trace { x: vec![], re: c.iter().map(|z| z.re).collect(), im: Some(c.iter().map(|z| z.im).collect()), typ: v.typ });
    }
    None
}

/// Load a netlist, run it to completion (background thread, so `halt` can stop it).
/// Takes a [`Prepared`] netlist only: includes inlined, `.control`/`*#` removed (see `schematic::prepare`).
pub fn run(netlist: &Prepared) -> Result<Run, Vec<String>> {
    run_until(netlist, &|| false)
}

/// [`run`], stopped (`bg_halt`, partial data) as soon as `stop()` says so: polled while it runs.
pub fn run_until(netlist: &Prepared, stop: &dyn Fn() -> bool) -> Result<Run, Vec<String>> {
    let sensors = netlist.sensors().to_vec();
    let netlist = netlist.text();
    let g = engine()?;
    let api = g.as_ref().unwrap();
    api.command("destroy all");
    api.command("remcirc");
    let id = RUNS.fetch_add(1, Ordering::SeqCst) + 1; // the previous run's results are gone now
    take_log();
    let lines: Vec<CString> = netlist.lines().map(|l| CString::new(l).unwrap_or_default()).collect();
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
    let mut plot = cstr(unsafe { (api.cur_plot)() });
    let log = take_log();
    if plot.is_empty() || plot == "const" {
        return Err(log);
    }
    let sensors = &sensors[..];
    // ngspice makes a helper source per par() in its output lines, named pa_00, pa_01 …
    // (cheap checks first, without a lowercase copy of the deck: most decks have neither)
    let pars = if !crate::schematic::contains_ci(netlist, "par(") { 0 } else {
        crate::schematic::statements(netlist, &crate::schematic::PAR_STATEMENTS).iter().map(|l| crate::schematic::par_calls(l)).sum()
    };
    let (mut vectors, mut scale) = plot_vectors(api, &plot, sensors, pars);
    // one frequency (`.ac lin 1 50 50`): phasors, shown as a table like .op instead of a 1-point plot
    let phasors = scale == "frequency" && read(api, "frequency").is_some_and(|f| f.re.len() == 1);
    if phasors {
        scale.clear();
    }
    // no scale (.op, .tf, .pz, .noise totals ...): single values, shown as a table
    let op = if scale.is_empty() {
        vectors.iter().filter_map(|v| {
            let t = read(api, v)?;
            // a sensor's current is reported under its device (`r1#branch` -> I(r1))
            let name = sensors.iter().find(|s| v.strip_suffix("#branch") == Some(s.source.as_str())).map_or(v.clone(), |s| format!("{}#branch", s.inst));
            // types are reliable only for .op and .ac (ngspice marks .tf/.pz/.sens/noise values as "voltage")
            let unit = match (plot.starts_with("op") || phasors, t.typ) { (true, 3) => "V", (true, 4) => "A", _ => "" }.to_string();
            Some((name, *t.re.first()?, t.im.and_then(|im| im.first().copied()).unwrap_or(0.0), unit))
        }).collect()
    } else {
        vec![]
    };
    // .noise ends in noise2 (totals); the spectrum users want to plot is noise1 (we `destroy all` per run)
    if plot == "noise2" && api.command("setplot noise1") {
        plot = "noise1".into();
        (vectors, scale) = plot_vectors(api, &plot, sensors, pars);
    }
    let aborted = log.iter().any(|l| {
        let l = l.to_lowercase();
        l.contains("aborted") || l.contains("timestep too small") || l.contains("simulation interrupted")
    });
    let meas = if crate::schematic::contains_ci(netlist, ".meas") { measurements(&log, &meas_names(netlist)) } else { vec![] };
    Ok(Run { id, plot, scale, vectors, op, aborted, meas, log })
}

fn plot_vectors(api: &Api, plot: &str, sensors: &[crate::schematic::Sensor], pars: usize) -> (Vec<String>, String) {
    let mut vectors = vec![];
    let cplot = CString::new(plot).unwrap_or_default();
    let mut p = unsafe { (api.all_vecs)(cplot.as_ptr()) };
    while !p.is_null() && unsafe { !(*p).is_null() } {
        vectors.push(cstr(unsafe { *p }));
        p = unsafe { p.add(1) };
    }
    // current sensors (schematic.rs netlist_for_sim) are an implementation detail: their extra
    // nodes (copies of the pin-1 nets), their .sens entries (`vsense_r1_freq` ...) and a sensed
    // inductor's native branch (it misses parallel parasitics) are not listed
    const SENS: [&str; 7] = ["", "_freq", "_phase", "_pwr", "_z0", "_acmag", "_acphase"];
    vectors.retain(|v| !sensors.iter().any(|s| {
        &s.node == v || SENS.iter().any(|x| *v == format!("{}{x}", s.source)) || *v == format!("{}#branch", s.inst)
    }));
    // the helper sources ngspice makes for `.meas … par('…')`: pa_00, bpa_00#branch … one per par()
    let helpers: std::collections::HashSet<String> = (0..pars).flat_map(|k| [format!("pa_{k:02}"), format!("bpa_{k:02}#branch")]).collect();
    vectors.retain(|v| !helpers.contains(v));
    vectors.sort();
    let scale = ["time", "frequency", "v-sweep", "i-sweep", "temp-sweep", "res-sweep"]
        .into_iter()
        .find(|s| !plot.starts_with("op") && vectors.iter().any(|v| v == s))
        .unwrap_or_default()
        .to_string();
    (vectors, scale)
}

/// Evaluate an ngspice expression (e.g. `v(out)`, `i(vsense_r1)`, `v(a)-v(b)`) in the results of
/// run `run` (an error once another run, such as a root locus step, replaced them).
pub fn trace(expr: &str, scale: &str, run: u64) -> Result<Trace, String> {
    // ngspice's command line is csh-like (`;` chains, backquotes run sh, `!` history, `$` vars):
    // only characters an expression needs may pass.
    let ok = |c: char| c.is_ascii_alphanumeric() || " _()[]@#+-*/^.,:\"'<>=&|%".contains(c);
    if !expr.chars().all(ok) || expr.trim().is_empty() {
        return Err(format!("invalid expression: {expr}"));
    }
    let g = engine().map_err(|e| e.join("\n"))?;
    let api = g.as_ref().unwrap();
    if RUNS.load(Ordering::SeqCst) != run {
        return Err("the results were replaced by a later run (a root locus or another simulation): run the simulation again".into());
    }
    api.command("unlet nmsp_trace"); // a failed `let` would otherwise leave the previous trace behind
    take_log(); // after unlet: it may print leftovers of the finished run ("Background thread stopped")
    api.command(&format!("let nmsp_trace = {expr}"));
    let mut t = read(api, "nmsp_trace").ok_or_else(|| {
        let log = take_log().join("\n");
        if log.is_empty() { format!("cannot evaluate {expr}") } else { log }
    })?;
    if !scale.is_empty() {
        t.x = read(api, scale).map(|s| s.re).unwrap_or_default();
    }
    // a constant (`1`, a reference level) is one value: a line across the x range
    if t.re.len() == 1 && t.x.len() > 1 {
        let n = t.x.len();
        t.re = vec![t.re[0]; n];
        if let Some(im) = &mut t.im {
            *im = vec![im[0]; n];
        }
    }
    Ok(t)
}

/// The latest run's id: the one whose results ngspice holds (a trace of another fails).
pub fn current_run() -> u64 {
    RUNS.load(Ordering::SeqCst)
}

/// Request a stop of whatever runs now; `run_until` sends `bg_halt` from its own thread.
pub fn halt() {
    HALTS.fetch_add(1, Ordering::SeqCst);
}

/// Tests that run simulations share the one ngspice engine: they take this lock.
#[cfg(test)]
pub static TEST_SIM: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_report() {
        let log: Vec<String> = ["Measurements for Transient Analysis", "error_pct           =  1.20000e-02", "Error: measure  w  (WHEN) : out of interval", "    Analysis is not possible.", " meas w failed!", ".meas tran p avg -i(v1) failed!", "tplh                =  5.4e-10 targ=  3.0e-08 trig=  3.0e-08", "tnom = 27"]
            .map(String::from).into();
        let names = meas_names(".meas tran error_pct AVG x ; when settled\n.meas tran\n* note\n+ w WHEN v(a)=1\n.MEAS p AVG -i(v1)\n.meas tran tplh TRIG …");
        assert_eq!(names.get("error_pct"), Some(&false), "a comment's words do not count");
        assert_eq!(names.get("w"), Some(&true), "a name on a + line; WHEN is on the x axis");
        let m = measurements(&log, &names);
        let names: Vec<&str> = m.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(names, ["error_pct", "w", "p", "tplh"], "{m:?}");
        assert!(m[3].3 && !m[0].3, "trig/targ on the x axis, AVG not");
        assert_eq!(m[3].2, "targ=  3.0e-08 trig=  3.0e-08");
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
