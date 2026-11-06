// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod git;
mod library;
mod mcp;
mod ngspice;
mod schematic;

use schematic::{Item, Library, Netlist, SymbolDef};
use serde::Serialize;
use std::sync::Mutex;

struct Lib(Mutex<Library>);

// Async commands run off the main thread (symbol lookup may hit the disk).
#[tauri::command]
async fn symbols(names: Vec<String>, dir: Option<String>, lib: tauri::State<'_, Lib>) -> Result<Vec<SymbolDef>, String> {
    let mut lib = lib.0.lock().unwrap_or_else(|e| e.into_inner());
    lib.sync(); // library sources switched off: their symbols go
    lib.set_dir(dir.as_deref().map(std::path::Path::new), true); // a symbol saved there meanwhile counts
    names.iter().for_each(|n| _ = lib.get(n));
    Ok(lib.0.values().cloned().collect())
}

#[tauri::command]
async fn examples() -> Vec<(String, Vec<Item>)> {
    schematic::EXAMPLES.iter().map(|(n, s)| (n.to_string(), schematic::parse_asc(s))).collect()
}

/// Off the main thread: reading an iCloud-evicted or network file can block for a while.
#[tauri::command]
async fn open_asc(path: String) -> Result<Vec<Item>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        Ok(schematic::parse_asc(&schematic::decode(&bytes)))
    })
    .await
    .map_err(|e| e.to_string())?
}

static SAVES: Mutex<()> = Mutex::new(());

/// Write a uniquely named temp file next to the target, sync, rename: a full disk or crash never
/// truncates the user's file. Mode, and on macOS xattrs (Finder tags) and ACLs, carry over (the
/// temp file is private until then). Symlinks are written through; a file we may not write is
/// refused like a plain write. Where a rename cannot preserve the file (hard
/// links) or the folder is not writable, it falls back to an in-place write.
fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let _serial = SAVES.lock().unwrap_or_else(|e| e.into_inner());
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let exists = path.exists();
    if exists {
        std::fs::OpenOptions::new().write(true).open(&path)?; // ownership, ACLs, read-only: as fs::write
    }
    #[cfg(unix)]
    if std::fs::metadata(&path).is_ok_and(|m| std::os::unix::fs::MetadataExt::nlink(&m) > 1) {
        return std::fs::write(&path, data); // a rename would split the hard link
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let prefix = format!("{name}.nomospice-");
    if let Some(dir) = path.parent().filter(|_| !name.is_empty()) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) && is_stale(&e.path()) {
                let _ = std::fs::remove_file(e.path()); // leftover of an interrupted save
            }
        }
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let mut tmp = path.clone().into_os_string();
    tmp.push(format!(".nomospice-{}-{nanos}", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    if exists {
        // private until copy_metadata applies the original mode; a new file just follows the umask
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    }
    let mut f = match opts.open(&tmp) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return std::fs::write(&path, data), // folder not writable
        Err(e) => return Err(e),
    };
    let res = f.write_all(data).and_then(|_| f.sync_all()).and_then(|_| {
        if exists {
            copy_metadata(&path, &tmp)?;
        }
        std::fs::rename(&tmp, &path)
    });
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Mode (via chmod, so the umask cannot strip group/other bits) and, on macOS, ACLs and
/// xattrs such as Finder tags. The modification time stays the new one.
fn copy_metadata(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::set_permissions(to, std::fs::metadata(from)?.permissions())?;
    #[cfg(target_os = "macos")]
    {
        use std::ffi::{c_char, c_int, c_void, CString};
        use std::os::unix::ffi::OsStrExt;
        extern "C" {
            fn copyfile(from: *const c_char, to: *const c_char, state: *mut c_void, flags: u32) -> c_int;
        }
        const COPYFILE_ACL: u32 = 1 << 0;
        const COPYFILE_XATTR: u32 = 1 << 2;
        let (a, b) = (CString::new(from.as_os_str().as_bytes())?, CString::new(to.as_os_str().as_bytes())?);
        if unsafe { copyfile(a.as_ptr(), b.as_ptr(), std::ptr::null_mut(), COPYFILE_ACL | COPYFILE_XATTR) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Older than [`STALE`]: safe to treat as left behind by a dead process.
fn is_stale(p: &std::path::Path) -> bool {
    std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default() > STALE)
}

#[tauri::command]
async fn path_exists(path: String) -> bool {
    std::path::Path::new(&path).exists()
}

#[tauri::command]
async fn save_asc(path: String, items: Vec<Item>) -> Result<(), String> {
    let data = schematic::encode(&schematic::write_asc(&items)); // Latin-1 like LTspice, UTF-16 if needed
    tauri::async_runtime::spawn_blocking(move || write_atomic(std::path::Path::new(&path), &data))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Live netlist for the UI; includes are resolved too, only to surface their warnings.
/// SVG export (schematic, plots): only .svg files, written like a schematic save.
#[tauri::command]
async fn export_svg(path: String, svg: String) -> Result<(), String> {
    if !path.to_lowercase().ends_with(".svg") {
        return Err(format!("{path}: not an .svg file"));
    }
    tauri::async_runtime::spawn_blocking(move || write_atomic(std::path::Path::new(&path), svg.as_bytes()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn netlist(items: Vec<Item>, dir: Option<String>, lib: tauri::State<'_, Lib>) -> Result<Netlist, String> {
    let mut nl = {
        let mut lib = lib.0.lock().unwrap_or_else(|e| e.into_inner());
        lib.set_dir(dir.as_deref().map(std::path::Path::new), false);
        schematic::netlist(&items, &mut lib)
    };
    schematic::prepare_warnings(&nl.text, dir.as_deref().map(std::path::Path::new), &mut nl.warnings);
    Ok(nl)
}

#[derive(Serialize)]
struct Sim {
    netlist: Netlist,
    /// What ngspice actually got (sensors, inlined includes): the log's line numbers refer to it.
    simulated: String,
    run: Option<ngspice::Run>,
    log: Vec<String>,
    /// What a root locus can vary: the schematic's .params and R/C/L parts (`schematic::sweepable`)
    sweepable: Vec<String>,
}

/// A root locus loads one .pz deck after another. A sweep holds this lock for its whole run, so a
/// simulation (which stops it via SWEEP_GEN) waits for it; traces check the run id instead.
static SWEEP: Mutex<()> = Mutex::new(());
/// Bumped to stop a running sweep at its next step (a new simulation, or the locus panel's Halt).
static SWEEP_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `dir`: folder of the .asc, for relative `.include`/`.lib` paths.
#[tauri::command]
async fn simulate(items: Vec<Item>, dir: Option<String>, lib: tauri::State<'_, Lib>) -> Result<Sim, String> {
    let op = ngspice::begin(); // a Halt pressed from here on (netlisting, include expansion) still counts
    // the UI gets the clean netlist (same nets/insts/probes); the one with current sensors runs
    // both netlists from one state of the library (the UI's nets, insts, probes match what runs)
    let (mut netlist, sim) = {
        let mut lib = lib.0.lock().unwrap_or_else(|e| e.into_inner());
        lib.set_dir(dir.as_deref().map(std::path::Path::new), false);
        (schematic::netlist(&items, &mut lib), schematic::netlist_for_sim(&items, &mut lib))
    };
    // the newer request wins: a running root locus stops at its next step, then this runs
    SWEEP_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let prepared = sim.prepared(dir.as_deref().map(std::path::Path::new), &mut netlist.warnings);
        let simulated = prepared.text().to_string();
        let _sweep = SWEEP.lock().unwrap_or_else(|e| e.into_inner());
        let res = ngspice::run_until(&prepared, &|| ngspice::halted_since(op));
        // for the s-plane view's root locus; from the user's deck, not the libraries
        let pz = res.as_ref().is_ok_and(|r| r.plot.starts_with("pz"));
        let sweepable = if pz { schematic::sweepable(&netlist.text, &netlist.insts) } else { vec![] };
        match res {
            Ok(mut run) => Sim { netlist, simulated, sweepable, log: std::mem::take(&mut run.log), run: Some(run) },
            Err(log) => Sim { netlist, simulated, sweepable, run: None, log },
        }
    }).await.map_err(|e| e.to_string())
}

/// Root locus: the schematic's `.pz` analysis once per value of `target` (a `.param` name or an
/// R/C/L instance). Per step: the (re, im) of every pole, or None where `.pz` failed
/// (it often does near a pole crossing; the view leaves a gap). Halt stops after the current step.
#[tauri::command]
async fn root_locus(items: Vec<Item>, dir: Option<String>, target: String, values: Vec<f64>, lib: tauri::State<'_, Lib>) -> Result<Vec<Option<Vec<(f64, f64)>>>, String> {
    let op = ngspice::begin();
    let gen = SWEEP_GEN.load(std::sync::atomic::Ordering::SeqCst); // at once: a cancel while netlisting counts
    let sim = {
        let mut lib = lib.0.lock().unwrap_or_else(|e| e.into_inner());
        lib.set_dir(dir.as_deref().map(std::path::Path::new), false);
        schematic::netlist_for_sim(&items, &mut lib)
    };
    // Halt, a new simulation or the panel's Halt: also mid-step (bg_halt)
    let stop = move || ngspice::halted_since(op) || SWEEP_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen;
    tauri::async_runtime::spawn_blocking(move || {
        let deck = sim.prepared(dir.as_deref().map(std::path::Path::new), &mut vec![]);
        match schematic::pz_statement(deck.text()) {
            None => return Err("the schematic has no .pz analysis (Simulation command → Poles/zeros)".into()),
            // the type (pz, pol, zer) is the last token before an inline comment
            Some(l) if l.split_whitespace().take_while(|t| !t.starts_with(['$', ';'])).last() == Some("zer") => return Err("the .pz analysis computes zeros only: a root locus needs the poles (Poles/zeros → Compute)".into()),
            _ => {}
        }
        // where the value sits in the deck: found once, then spliced per step
        let deck = deck.only_pz();
        // the user's own .param of that name wins over a part of it; a library's does not
        let own = schematic::sweepable(&sim.text, &[]).iter().any(|n| n.eq_ignore_ascii_case(target.trim()));
        let spans = schematic::vary_spans(deck.text(), &target, own).ok_or_else(|| format!("`{target}` is neither a .param nor an R, C or L with a numeric value in the schematic"))?;
        let _sweep = SWEEP.lock().unwrap_or_else(|e| e.into_inner());
        let (mut steps, mut first_error, mut stopped) = (vec![], None, false);
        for &v in &values {
            if stop() {
                stopped = true;
                break;
            }
            let step = deck.at(&spans, v);
            match ngspice::run_until(&step, &stop) {
                // interrupted by Halt (bg_halt ends the run with partial data) or cancelled: not a step
                _ if stop() => {
                    stopped = true;
                    break;
                }
                // aborted by ngspice itself (no convergence near a crossing): a gap, the sweep goes on
                Ok(run) if run.aborted => steps.push(None),
                Ok(run) => steps.push(Some(run.op.into_iter().filter(|o| o.0.starts_with("pole")).map(|o| (o.1, o.2)).collect())),
                Err(log) => {
                    first_error.get_or_insert(log.join("\n"));
                    steps.push(None);
                }
            }
        }
        match first_error {
            // all failed (and not merely stopped early): the reason, not an empty locus
            Some(e) if !stopped && steps.iter().all(Option::is_none) => Err(e),
            _ => Ok(steps),
        }
    }).await.map_err(|e| e.to_string())?
}

/// Save As into another folder: the schematic's own symbols (`<name>.asy` next to it) that `names`
/// use and the new folder lacks; `copy`: copy them there (else only list them).
#[tauri::command]
async fn folder_symbols(from: String, to: String, names: Vec<String>, copy: bool) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut out: Vec<String> = vec![];
        for n in names {
            let file = format!("{}.asy", n.rsplit(['\\', '/']).next().unwrap_or(&n));
            let (src, dst) = (std::path::Path::new(&from).join(&file), std::path::Path::new(&to).join(&file));
            if src.is_file() && !dst.exists() && !out.contains(&file) {
                if copy {
                    std::fs::copy(&src, &dst).map_err(|e| format!("{file}: {e}"))?;
                }
                out.push(file);
            }
        }
        Ok(out)
    }).await.map_err(|e| e.to_string())?
}

/// Stops a running root locus at its next step (its panel's Halt; the toolbar Halt stops everything).
#[tauri::command]
fn cancel_locus() {
    SWEEP_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// `run`: the id of the Run the trace is for (ngspice keeps only the latest run's results).
#[tauri::command]
async fn trace(expr: String, scale: String, run: u64) -> Result<ngspice::Trace, String> {
    tauri::async_runtime::spawn_blocking(move || ngspice::trace(&expr, &scale, run)).await.map_err(|e| e.to_string())?
}

static ARGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// `nomospice file.asc [--run]` (paths made absolute at startup)
#[tauri::command]
fn cli_args() -> Vec<String> {
    ARGS.get().cloned().unwrap_or_default()
}

#[tauri::command]
fn halt() {
    ngspice::halt();
}

// Crash/quit recovery: the frontend mirrors unsaved work to recovery-<id>.json, because some
// exits (Dock > Quit, logout, crashes) cannot be intercepted to ask about unsaved changes.
// - Liveness: each instance holds an exclusive lock on its recovery-<id>.lock while running, so
//   a .lock we *can* lock belongs to a dead instance and its .json is an orphan.
// - Data is written to a temp file, synced and renamed: a crash mid-write keeps the last snapshot.
// - <id> = pid + start time, so a reused pid never touches an older file.
// - An orphan offered for restore stays on disk (and locked by us) until the user decided.
type Slot = Mutex<Option<(std::fs::File, std::path::PathBuf)>>;
static OWN: Slot = Mutex::new(None);
static CLAIM: Slot = Mutex::new(None);
const STALE: std::time::Duration = std::time::Duration::from_secs(60);

fn recovery_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // unsaved work lives here: private to the user, whatever the umask
    #[cfg(unix)]
    std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700)).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Lock held for the whole save/clear, so the two never interleave.
fn own(app: &tauri::AppHandle) -> Result<std::sync::MutexGuard<'static, Option<(std::fs::File, std::path::PathBuf)>>, String> {
    let mut slot = OWN.lock().map_err(|e| e.to_string())?;
    if slot.is_none() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
        let base = recovery_dir(app)?.join(format!("recovery-{}-{t}", std::process::id()));
        let lock = std::fs::File::create(base.with_extension("lock")).map_err(|e| e.to_string())?;
        if let Err(e) = lock.try_lock() {
            let _ = std::fs::remove_file(base.with_extension("lock")); // no lock: do not leak the file
            return Err(e.to_string());
        }
        *slot = Some((lock, base.with_extension("json")));
    }
    Ok(slot)
}

#[tauri::command]
async fn recovery_save(app: tauri::AppHandle, doc: serde_json::Value) -> Result<(), String> {
    // blocking file IO + fsync: keep it off the async workers other commands share
    tauri::async_runtime::spawn_blocking(move || {
        let slot = own(&app)?;
        write_atomic(&slot.as_ref().unwrap().1, doc.to_string().as_bytes()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn recovery_clear() -> Result<(), String> {
    let slot = OWN.lock().map_err(|e| e.to_string())?;
    match slot.as_ref() {
        Some((_, path)) if path.exists() => std::fs::remove_file(path).map_err(|e| e.to_string()),
        _ => Ok(()), // never saved anything: nothing to create or remove
    }
}

/// Newest unsaved document of a dead instance. It stays on disk, locked by us, until
/// `recovery_resolve`; older orphans wait for the next launch.
#[tauri::command]
async fn recovery_take(app: tauri::AppHandle) -> Option<serde_json::Value> {
    let dir = recovery_dir(&app).ok()?;
    let own = OWN.lock().ok()?.as_ref().map(|(_, p)| p.with_extension("lock"));
    let mut orphans = vec![];
    for e in std::fs::read_dir(&dir).ok()?.flatten() {
        let p = e.path();
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = p.extension().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
        if !name.starts_with("recovery-") || Some(&p) == own.as_ref() {
            continue;
        }
        if name.contains(".nomospice-") && is_stale(&p) {
            let _ = std::fs::remove_file(&p); // interrupted write_atomic
        }
        if ext != "lock" {
            continue;
        }
        let Ok(f) = std::fs::File::open(&p) else { continue };
        if f.try_lock().is_err() {
            continue; // a live instance owns it
        }
        let json = p.with_extension("json");
        match std::fs::metadata(&json).and_then(|m| m.modified()) {
            Ok(t) => orphans.push((t, f, p)),
            // clean exit; "old" so a lock file another instance just created is never removed
            Err(_) if is_stale(&p) => drop((f, std::fs::remove_file(&p))),
            Err(_) => {}
        }
    }
    orphans.sort_by_key(|o| std::cmp::Reverse(o.0));
    for (_, f, lock) in orphans {
        let json = lock.with_extension("json");
        let Ok(bytes) = std::fs::read(&json) else { continue }; // transient read error: keep it
        match serde_json::from_slice(&bytes) {
            Ok(doc) => {
                *CLAIM.lock().ok()? = Some((f, lock));
                return Some(doc);
            }
            Err(_) => {
                // corrupt: drop it (still locked by us) so older valid sessions get their turn
                let _ = std::fs::remove_file(json);
                let _ = std::fs::remove_file(&lock);
            }
        }
    }
    None
}

/// The user answered the restore prompt. `keep`: the orphan becomes this instance's recovery
/// file (atomic rename, no round trip through the UI); otherwise it is deleted. Both happen while
/// still holding the orphan's lock, so no other instance can claim it in between.
#[tauri::command]
async fn recovery_resolve(app: tauri::AppHandle, keep: bool) -> Result<(), String> {
    let Some((f, lock)) = CLAIM.lock().map_err(|e| e.to_string())?.take() else { return Ok(()) };
    let json = lock.with_extension("json");
    if keep {
        let slot = own(&app)?;
        std::fs::rename(&json, &slot.as_ref().unwrap().1).map_err(|e| e.to_string())?;
    } else {
        let _ = std::fs::remove_file(&json);
    }
    let _ = std::fs::remove_file(&lock);
    drop(f);
    Ok(())
}

// ---------------------------------------------------------------- model libraries

fn library_config(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("libraries.json"))
}

#[tauri::command]
async fn library_sources() -> Vec<library::Source> {
    library::sources()
}

#[tauri::command]
async fn library_set_sources(app: tauri::AppHandle, sources: Vec<library::Source>) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(&sources).map_err(|e| e.to_string())?;
    write_atomic(&library_config(&app)?, &json).map_err(|e| e.to_string())?;
    library::set_sources(sources);
    Ok(())
}

#[tauri::command]
async fn library_search(query: String, kind: Option<String>, limit: usize) -> Vec<library::Entry> {
    // the first call may build the index (~0.2 s): off the async workers
    tauri::async_runtime::spawn_blocking(move || {
        let ix = library::index();
        ix.as_ref().map(|ix| ix.search(&query, kind.as_deref(), limit).into_iter().cloned().collect()).unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
async fn library_text(name: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ix = library::index();
        let ix = ix.as_ref()?;
        ix.text(ix.get(&name)?)
    })
    .await
    .ok()?
}

/// Micro-Cap 12 (freeware since Spectrum Software closed in 2019; redistributable only as the
/// complete, unmodified package): download the original CD from the Internet Archive and unpack
/// its LIBRARY folder for local use. Uses the system's curl and tar/unzip; nothing is executed.
#[tauri::command]
async fn library_fetch_microcap(app: tauri::AppHandle) -> Result<Vec<library::Source>, String> {
    use tauri::Manager;
    const URL: &str = "https://archive.org/download/mc12cd_202110/mc12cd.zip";
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    let target = app.path().app_data_dir().map_err(|e| e.to_string())?.join("libraries").join("Micro-Cap 12");
    static FETCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FETCHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err("the Micro-Cap 12 library is already being downloaded".into());
    }
    let dir = target.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        // Err((ran, message)): `ran` = the tool started (so a failure says something about the input)
        let run = |cmd: &mut std::process::Command, ok: &[i32]| match cmd.status() {
            Ok(s) if s.success() || s.code().is_some_and(|c| ok.contains(&c)) => Ok(()),
            Ok(s) => Err((true, format!("{cmd:?} failed ({s})"))),
            Err(e) => Err((false, format!("{cmd:?}: {e}"))),
        };
        std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let zip = cache.join("mc12cd.zip");
        if !zip.is_file() {
            let part = cache.join("mc12cd.zip.part");
            run(git::command("curl").args(["-fsSL", "-o"]).arg(&part).arg(URL), &[]).map_err(|e| e.1)?;
            std::fs::rename(&part, &zip).map_err(|e| e.to_string())?;
        }
        // a damaged download must not be reused forever
        let unpack = |zip: &std::path::Path, tmp: &std::path::Path| if cfg!(target_os = "linux") {
            run(git::command("unzip").args(["-q", "-o"]).arg(zip).arg("LIBRARY/*").arg("-d").arg(tmp), &[1]) // 1 = warnings only
        } else {
            run(git::command("tar").arg("-xf").arg(zip).arg("-C").arg(tmp).arg("LIBRARY"), &[]) // bsdtar reads zip
        };
        let tmp = cache.join("mc12-unpack");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        if let Err((ran, e)) = unpack(&zip, &tmp) {
            if ran {
                let _ = std::fs::remove_file(&zip); // the archive itself is bad: download again next time
            }
            return Err(e);
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(tmp.join("LIBRARY"), &dir).map_err(|e| e.to_string())
    })
    .await;
    FETCHING.store(false, std::sync::atomic::Ordering::SeqCst);
    res.map_err(|e| e.to_string())??;
    let mut list = library::sources();
    list.retain(|s| s.path != target);
    list.push(library::Source { name: "Micro-Cap 12".into(), path: target, enabled: true });
    library_set_sources(app, list.clone()).await?;
    Ok(list)
}

// ---------------------------------------------------------------- git

#[tauri::command]
async fn git_status(path: String) -> git::Status {
    tauri::async_runtime::spawn_blocking(move || git::status(std::path::Path::new(&path))).await.unwrap_or_default()
}

#[tauri::command]
async fn git_log(path: String) -> Result<Vec<git::Commit>, String> {
    tauri::async_runtime::spawn_blocking(move || git::log(std::path::Path::new(&path), 200)).await.map_err(|e| e.to_string())?
}

/// The schematic as it was in a commit.
#[tauri::command]
async fn git_show(path: String, hash: String) -> Result<Vec<Item>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        git::show(std::path::Path::new(&path), &hash).map(|b| schematic::parse_asc(&schematic::decode(&b)))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn git_commit(path: String, message: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git::commit(std::path::Path::new(&path), &message)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn git_init(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || git::init(std::path::Path::new(&path))).await.map_err(|e| e.to_string())?
}

/// The frontend confirmed (or had nothing unsaved): really quit.
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

/// macOS: ⌘Q goes straight to applicationWillTerminate (no way to veto), so the app menu's
/// Quit item is replaced by one that asks the frontend about unsaved changes first. ⌘W closes
/// the document, not the only window (which would quit the app).
#[cfg(target_os = "macos")]
fn install_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, MenuItemKind};
    let menu = Menu::default(app.handle())?;
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
        if let Some(last) = app_menu.items()?.pop() {
            app_menu.remove(&last)?;
        }
        app_menu.append(&MenuItem::with_id(app, "quit", "Quit nomospice", true, Some("CmdOrCtrl+Q"))?)?;
        // where macOS users look for settings: after About
        let libraries = MenuItem::with_id(app, "libraries", "Model Libraries…", true, Some("CmdOrCtrl+,"))?;
        app_menu.insert_items(&[&libraries, &tauri::menu::PredefinedMenuItem::separator(app)?], 2)?;
    }
    for sub in menu.items()? {
        let MenuItemKind::Submenu(sub) = sub else { continue };
        for it in sub.items()? {
            if let MenuItemKind::Predefined(p) = &it {
                if p.text()? == "Close Window" {
                    sub.remove(p)?;
                }
            }
        }
        if sub.text()? == "File" {
            sub.append(&MenuItem::with_id(app, "close", "Close", true, Some("CmdOrCtrl+W"))?)?;
        }
    }
    app.set_menu(menu)?;
    Ok(())
}

/// Tauri's app_config_dir without a running app (for `--mcp`).
fn config_dir() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?);
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(Into::into).unwrap_or(home.join("AppData/Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(Into::into).unwrap_or(home.join(".config"))
    };
    Some(base.join("de.nomospice.app"))
}

fn main() {
    // a panic fails only its command; no console on Windows, so keep it for bug reports
    let log = config_dir().map(|d| d.join("panic.log"));
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(p) = &log {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                use std::io::Write;
                let _ = writeln!(f, "nomospice {}: {info}", env!("CARGO_PKG_VERSION"));
            }
        }
        default_hook(info);
    }));
    let started_in = std::env::current_dir().unwrap_or_default();
    let args = std::env::args().skip(1).map(|a| std::fs::canonicalize(&a).map_or(a, |p| p.display().to_string()));
    ARGS.set(args.collect()).ok();
    // ngspice reads `.spiceinit` from the working directory on init; a folder you merely opened a
    // project from must not be able to inject commands. (ngSpice_nospiceinit() segfaults in ngspice-47.)
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let _ = std::env::set_current_dir(home);
    }
    if std::env::args().any(|a| a == "--mcp") {
        // MCP server on stdio instead of the GUI (same libraries as the app)
        if let Some(list) = config_dir().and_then(|d| std::fs::read(d.join("libraries.json")).ok()).and_then(|b| serde_json::from_slice(&b).ok()) {
            library::set_sources(list);
        }
        return mcp::serve(started_in);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Lib(Mutex::new(Library::builtin())))
        // updates from the GitHub releases (latest.json, signed with the key in ~/.tauri; see scripts/publish-release.sh)
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![symbols, examples, path_exists, open_asc, save_asc, export_svg, netlist, simulate, root_locus, cancel_locus, folder_symbols, trace, halt, cli_args, quit, recovery_save, recovery_take, recovery_resolve, recovery_clear,
            library_sources, library_set_sources, library_search, library_text, library_fetch_microcap,
            git_status, git_log, git_show, git_commit, git_init])
        .setup(|app| {
            use tauri::Manager;
            if let Ok(dir) = app.path().app_cache_dir() {
                ngspice::set_scratch_dir(dir);
            }
            // saved library sources (else: detected ones); warm the index in the background
            if let Some(list) = library_config(app.handle()).ok().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()) {
                library::set_sources(list);
            }
            std::thread::spawn(|| drop(library::index()));
            #[cfg(target_os = "macos")]
            install_menu(app)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            use tauri::Emitter;
            if event.id() == "quit" {
                let _ = app.emit("quit-requested", ());
            }
            if event.id() == "libraries" {
                let _ = app.emit("open-libraries", ());
            }
            if event.id() == "close" {
                let _ = app.emit("close-requested", ());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running nomospice");
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn atomic_save_keeps_file_identity() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("nomospice-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.asc");
        std::fs::write(&f, "old").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o664)).unwrap();
        let link = dir.join("link.asc");
        let _ = std::os::unix::fs::symlink(&f, &link);
        #[cfg(target_os = "macos")]
        assert!(std::process::Command::new("/usr/bin/xattr").args(["-w", "user.nomospice", "tag"]).arg(&f).status().unwrap().success());
        super::write_atomic(&link, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "new");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "link replaced");
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o664);
        #[cfg(target_os = "macos")]
        assert!(std::process::Command::new("/usr/bin/xattr").args(["-p", "user.nomospice"]).arg(&f).output().unwrap().status.success(), "xattr lost");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "temp file left behind");
        std::fs::remove_dir_all(dir).ok();
    }
}
