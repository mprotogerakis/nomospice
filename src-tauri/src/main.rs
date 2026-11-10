// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod git;
mod laplace;
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
    Ok(lib.defs.values().cloned().collect())
}

#[tauri::command]
async fn examples() -> Vec<(String, Vec<Item>)> {
    schematic::EXAMPLES.iter().map(|(n, s)| (n.to_string(), schematic::parse_asc(s))).collect()
}

/// Off the main thread: reading an iCloud-evicted or network file can block for a while.
#[tauri::command]
/// The items, and whether the file was a netlist (a schematic of one directive holding it: the
/// app saves it elsewhere, never over it).
async fn open_asc(path: String) -> Result<(Vec<Item>, bool), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        Ok(schematic::load(&path, &bytes))
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
    // (a symbol's name is how schematics place it: one word, not a built-in symbol's, which wins)
    // (a file there already, opened and edited: written as asked)
    let symbol = schematic::is_symbol(&path);
    if symbol && !std::path::Path::new(&path).exists() {
        let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if stem.contains(char::is_whitespace) {
            return Err(format!("“{stem}”: a symbol's name has no spaces (a schematic places it by name)"));
        }
        if schematic::builtin_symbol(&stem.to_lowercase()) {
            return Err(format!("{stem} is a built-in symbol's name, which a placed {stem} would be: choose another"));
        }
    }
    // (a symbol as one; Latin-1 like LTspice, UTF-16 if needed)
    let data = schematic::encode(&if symbol { schematic::write_asy(&items) } else { schematic::write_asc(&items) });
    tauri::async_runtime::spawn_blocking(move || write_atomic(std::path::Path::new(&path), &data))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// A block symbol for the schematic at `path` (`<name>.asy` beside it), from its port labels:
/// placed in another schematic of that folder, it stands for this one. The folder's symbol of that
/// name (any case) is written over if `replace`; else its file name comes back (None: written).
#[tauri::command]
async fn block_symbol(path: String, items: Vec<Item>, replace: bool) -> Result<Option<String>, String> {
    let path = std::path::PathBuf::from(path);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if schematic::builtin_symbol(&schematic::sym_key(&stem)) {
        return Err(format!("{stem} is the name of a built-in symbol, which a placed {stem} would be: save the schematic under another name first"));
    }
    if stem.contains(char::is_whitespace) {
        return Err(format!("“{stem}”: a symbol's name has no spaces (a schematic places it by name): save the schematic as e.g. {}.asc first", stem.split_whitespace().collect::<Vec<_>>().join("_")));
    }
    let data = schematic::encode(&schematic::block_asy(&items)?);
    tauri::async_runtime::spawn_blocking(move || {
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        let there = schematic::folder_contents(dir).0.remove(&schematic::sym_key(&stem));
        match there {
            Some(p) if !replace => Ok(Some(p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())),
            // (the one there: on a case-sensitive disk another case of it would not be the one used)
            there => write_atomic(&there.unwrap_or_else(|| path.with_extension("asy")), &data).map(|_| None).map_err(|e| e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
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
    let mut nl = schematic::netlist(&items, &mut lib_at(&lib, dir.as_deref()));
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
    /// The schematic folder's symbols changed since last asked (edited there): reload them
    symbols: bool,
}

/// The symbol library, set to the schematic's folder (its own .asy symbols): every netlisting goes
/// through here, so none uses another document's folder.
fn lib_at<'a>(lib: &'a Lib, dir: Option<&str>) -> std::sync::MutexGuard<'a, Library> {
    let mut lib = lib.0.lock().unwrap_or_else(|e| e.into_inner());
    lib.set_dir(dir.map(std::path::Path::new), false);
    lib
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
    let (mut netlist, sim, symbols) = {
        let mut lib = lib.0.lock().unwrap_or_else(|e| e.into_inner());
        // a run looks again: a symbol edited next to the schematic counts (the UI reloads it)
        let symbols = lib.set_dir(dir.as_deref().map(std::path::Path::new), true);
        lib.reread_sheets();
        (schematic::netlist(&items, &mut lib), schematic::netlist_for_sim(&items, &mut lib), symbols)
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
        // (with the parameters only a .step sets: the locus runs on the first step's deck, where they are)
        let stepped = res.as_ref().is_ok_and(|r| !r.steps.is_empty()); // (its .step ran: the locus runs its first step)
        let sweepable = if pz {
            let mut s = schematic::sweepable(&netlist.text, &netlist.insts);
            if stepped {
                s.extend(schematic::step_params(&netlist.steps).into_iter().filter(|p| !s.iter().any(|x| x.eq_ignore_ascii_case(p))).collect::<Vec<_>>());
            }
            s
        } else {
            vec![]
        };
        match res {
            Ok(mut run) => Sim { netlist, simulated, sweepable, symbols, log: std::mem::take(&mut run.log), run: Some(run) },
            Err(log) => Sim { netlist, simulated, sweepable, symbols, run: None, log },
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
        let mut lib = lib_at(&lib, dir.as_deref());
        lib.reread_sheets(); // (a run reads the blocks' schematics again)
        schematic::netlist_for_sim(&items, &mut lib)
    };
    // Halt, a new simulation or the panel's Halt: also mid-step (bg_halt)
    let stop = move || ngspice::halted_since(op) || SWEEP_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen;
    tauri::async_runtime::spawn_blocking(move || {
        let deck = sim.prepared(dir.as_deref().map(std::path::Path::new), &mut vec![]);
        // (a .step deck: its first step, so a parameter only .step defines is there; one .pz per locus step)
        // (a .step that cannot run: the deck as it is, as a simulation runs it once)
        let deck = match deck.steps().ok().flatten().and_then(|s| s.at(0).ok()) {
            Some((_, first)) => first,
            None => deck,
        };
        match schematic::pz_statement(deck.text()) {
            None => return Err("the schematic has no .pz analysis (Simulation command → Poles/zeros)".into()),
            // the type (pz, pol, zer) is the last token (pz_statement cut inline comments)
            Some(l) if l.split_whitespace().last() == Some("zer") => return Err("the .pz analysis computes zeros only: a root locus needs the poles (Poles/zeros → Compute)".into()),
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
/// use, as (file, clash): clash when the new folder has a different file of that name (it would be
/// used instead, and is not overwritten). `copy`: copy the others there (else only list them).
#[tauri::command]
async fn folder_symbols(from: String, to: String, names: Vec<String>, copy: bool, doc: Option<String>) -> Result<Vec<(String, bool)>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // the library's own rule for which .asy files are a folder's symbols
        let (old, old_sheets) = schematic::folder_contents(std::path::Path::new(&from));
        let (new, new_sheets) = schematic::folder_contents(std::path::Path::new(&to));
        let (mut out, mut failed): (Vec<(String, bool)>, Vec<String>) = (vec![], vec![]);
        let mut seen = std::collections::HashSet::new(); // (many parts share a symbol)
        // (a block's schematic goes along with its symbol, and the symbols it uses with it)
        let mut files: Vec<(std::path::PathBuf, Option<&std::path::PathBuf>)> = vec![];
        let mut queue: std::collections::VecDeque<String> = names.into();
        while let Some(n) = queue.pop_front() {
            let key = schematic::sym_key(&n);
            let Some(src) = old.get(&key).filter(|_| seen.insert(key.clone())) else { continue };
            files.push((src.clone(), new.get(&key)));
            // (not where the new folder has its own symbol of that name, which it would turn into a
            // block; not the document itself, a test bench of its own block, which is saved anyway)
            // (the new folder's own symbol of that name: its sheet goes along only if that is this same symbol)
            let bytes = std::fs::read(src).ok();
            let same = new.get(&key).is_none_or(|t| std::fs::read(t).ok() == bytes);
            let def = bytes.map(|b| schematic::parse_asy(&key, &schematic::decode(&b))).filter(|d| same && d.calls_sheet());
            // (the same file however spelled: case, links, `..`)
            let own = |p: &std::path::PathBuf| doc.as_deref().is_some_and(|d| same_file(std::path::Path::new(d), p));
            if let (Some(def), Some(sheet)) = (def, old_sheets.get(&key).filter(|p| !own(p))) {
                // (only one the netlist would take as its block's: not a library part's test bench)
                let items = std::fs::read(sheet).map(|b| schematic::parse_asc(&schematic::decode(&b))).unwrap_or_default();
                if def.sheet_fits(&items).is_ok() {
                    files.push((sheet.clone(), new_sheets.get(&key)));
                    queue.extend(items.into_iter().filter_map(|it| match it { Item::Sym { sym, .. } => Some(sym), _ => None }));
                }
            }
        }
        for (src, there) in files {
            let src = &src;
            let file = src.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            match there {
                Some(there) => {
                    // a different file of that name there: it would be used instead (an identical
                    // one is simply the same symbol: nothing to tell)
                    if std::fs::read(src).ok() != std::fs::read(there).ok() {
                        out.push((file, true));
                    }
                }
                None => {
                    // every file tried: one that fails does not keep the others back
                    if copy {
                        if let Err(e) = std::fs::copy(src, std::path::Path::new(&to).join(&file)) {
                            failed.push(format!("{file}: {e}"));
                        }
                    }
                    out.push((file, false));
                }
            }
        }
        if failed.is_empty() { Ok(out) } else { Err(failed.join("; ")) }
    }).await.map_err(|e| e.to_string())?
}

/// Whether two paths name one file (another case on a case-insensitive disk, a link, `..`).
#[cfg(unix)]
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    matches!((std::fs::metadata(a), std::fs::metadata(b)), (Ok(x), Ok(y)) if (x.dev(), x.ino()) == (y.dev(), y.ino()))
}
#[cfg(not(unix))]
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).map(|c| c.to_string_lossy().to_lowercase());
    matches!((canon(a), canon(b)), (Ok(x), Ok(y)) if x == y)
}

/// Stops a running root locus at its next step (its panel's Halt; the toolbar Halt stops everything).
#[tauri::command]
fn cancel_locus() {
    SWEEP_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// The results of run `run` as a .raw file.
#[tauri::command]
/// Returns the vectors left out (none readable, another length: see ngspice::export_raw).
async fn export_raw(path: String, run: u64) -> Result<Vec<String>, String> {
    if !path.to_lowercase().ends_with(".raw") {
        return Err(format!("{path}: not a .raw file"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let (bytes, dropped) = ngspice::export_raw(run)?;
        write_atomic(std::path::Path::new(&path), &bytes).map_err(|e| format!("{path}: {e}"))?; // (a failed write leaves the file as it was)
        Ok(dropped)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `run`: the id of the Run the trace is for (ngspice keeps only the latest run's results).
#[tauri::command]
async fn trace(expr: String, scale: String, run: u64) -> Result<ngspice::Trace, String> {
    tauri::async_runtime::spawn_blocking(move || ngspice::trace(&expr, &scale, run)).await.map_err(|e| e.to_string())?
}

static ARGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

// ---------------------------------------------------------------- windows: one document each

/// What a new window starts with (its `cli_args`): a file to open, `--restore`, `--new-symbol`.
static PENDING: Mutex<std::collections::BTreeMap<String, Vec<String>>> = Mutex::new(std::collections::BTreeMap::new());
/// The file each window has open (none: untitled), so a file is opened in one window only.
static DOCS: Mutex<std::collections::BTreeMap<String, String>> = Mutex::new(std::collections::BTreeMap::new());
static NEXT_WINDOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
/// The window focused last (while a sheet or the About panel has the focus, it is still the one).
static LAST_FOCUSED: Mutex<String> = Mutex::new(String::new());

/// The first window: `nomospice file.asc [--run]` (paths made absolute at startup); another: what
/// it was opened for (asked once).
#[tauri::command]
fn cli_args(window: tauri::Window) -> Vec<String> {
    if window.label() == "main" {
        return ARGS.get().cloned().unwrap_or_default();
    }
    PENDING.lock().ok().and_then(|mut p| p.remove(window.label())).unwrap_or_default()
}

/// A new window (cascaded from the focused one), starting with `args`.
fn open_window(app: &tauri::AppHandle, args: Vec<String>) -> Result<(), String> {
    use tauri::Manager;
    let label = format!("doc-{}", NEXT_WINDOW.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    PENDING.lock().map_err(|e| e.to_string())?.insert(label.clone(), args);
    let mut b = tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title("nomospice")
        .inner_size(1400.0, 900.0)
        .min_inner_size(900.0, 600.0);
    #[cfg(target_os = "macos")]
    {
        b = b.tabbing_identifier("nomospice"); // Window → Merge All Windows: tabs
    }
    // cascaded from the focused window, past the windows already there
    if let Some(w) = focused(app).and_then(|l| app.get_webview_window(&l)) {
        if let (Ok(p), Ok(k)) = (w.outer_position(), w.scale_factor()) {
            let at: Vec<_> = app.webview_windows().values().filter_map(|o| o.outer_position().ok().map(|q| q.to_logical::<f64>(k))).collect();
            let (mut x, mut y) = (p.to_logical::<f64>(k).x + 28.0, p.to_logical::<f64>(k).y + 28.0);
            while at.iter().any(|q| (q.x - x).abs() < 4.0 && (q.y - y).abs() < 4.0) {
                (x, y) = (x + 28.0, y + 28.0);
            }
            b = b.position(x, y);
        }
    }
    b.build().map(drop).map_err(|e| {
        PENDING.lock().map(|mut p| p.remove(&label)).ok();
        e.to_string()
    })
}

/// (async: building a window from a synchronous command deadlocks on Windows)
#[tauri::command]
async fn new_window(app: tauri::AppHandle, args: Vec<String>) -> Result<(), String> {
    open_window(&app, args)
}

#[tauri::command]
fn window_count(app: tauri::AppHandle) -> usize {
    use tauri::Manager;
    app.webview_windows().len()
}

/// The window's document is now `path` (None: untitled, or not written yet).
#[tauri::command]
fn doc_path(window: tauri::Window, path: Option<String>) {
    if let Ok(mut d) = DOCS.lock() {
        match path {
            Some(p) => d.insert(window.label().to_string(), p),
            None => d.remove(window.label()),
        };
    }
}

/// The window that has `path` open comes to the front: true; none has it: false.
#[tauri::command]
fn focus_doc(app: tauri::AppHandle, path: String) -> bool {
    use tauri::Manager;
    let docs = DOCS.lock().map(|d| d.clone()).unwrap_or_default();
    let Some(label) = docs.iter().find(|(_, p)| *p == &path || same_file(std::path::Path::new(p.as_str()), std::path::Path::new(&path))).map(|(l, _)| l.clone()) else { return false };
    let Some(w) = app.get_webview_window(&label) else { return false };
    let _ = w.unminimize();
    let _ = w.set_focus();
    true
}

/// The window menu commands go to: the focused one, else the one focused last, else any.
fn focused(app: &tauri::AppHandle) -> Option<String> {
    use tauri::Manager;
    let ws = app.webview_windows();
    let last = LAST_FOCUSED.lock().map(|l| l.clone()).unwrap_or_default();
    ws.iter().find(|(_, w)| w.is_focused().unwrap_or(false)).map(|(l, _)| l.clone()).or_else(|| ws.contains_key(&last).then_some(last)).or_else(|| ws.keys().next().cloned())
}

/// A window closed: what it held goes.
fn forget_window(label: &str) {
    recovery_release(label);
    DOCS.lock().map(|mut d| d.remove(label)).ok();
    PENDING.lock().map(|mut p| p.remove(label)).ok();
}

/// The id of the run whose results ngspice holds (a result tab brought back may hold an older one).
#[tauri::command]
fn run_id() -> u64 {
    ngspice::current_run()
}

#[tauri::command]
fn halt() {
    ngspice::halt();
}

// Crash/quit recovery: each window mirrors its unsaved work to recovery-<id>.json, because some
// exits (Dock > Quit, logout, crashes) cannot be intercepted to ask about unsaved changes.
// - Liveness: each window holds an exclusive lock on its recovery-<id>.lock while open, so a .lock
//   we *can* lock belongs to a closed window or a dead instance and its .json is an orphan.
// - Data is written to a temp file, synced and renamed: a crash mid-write keeps the last snapshot.
// - <id> = pid + start time + window, so a reused pid never touches an older file.
// - An orphan offered for restore stays on disk (and locked by us) until the user decided.
type Held = std::collections::BTreeMap<String, (std::fs::File, std::path::PathBuf)>;
static OWN: Mutex<Held> = Mutex::new(Held::new());
static CLAIM: Mutex<Held> = Mutex::new(Held::new());
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

/// Window `label`'s recovery file (its lock taken on first use), under the lock held for the
/// whole save/clear, so the two never interleave.
fn own(app: &tauri::AppHandle, label: &str) -> Result<(std::sync::MutexGuard<'static, Held>, std::path::PathBuf), String> {
    let mut held = OWN.lock().map_err(|e| e.to_string())?;
    if !held.contains_key(label) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
        let base = recovery_dir(app)?.join(format!("recovery-{}-{t}-{label}", std::process::id()));
        let lock = std::fs::File::create(base.with_extension("lock")).map_err(|e| e.to_string())?;
        if let Err(e) = lock.try_lock() {
            let _ = std::fs::remove_file(base.with_extension("lock")); // no lock: do not leak the file
            return Err(e.to_string());
        }
        held.insert(label.to_string(), (lock, base.with_extension("json")));
    }
    let json = held[label].1.clone();
    Ok((held, json))
}

#[tauri::command]
async fn recovery_save(app: tauri::AppHandle, window: tauri::Window, doc: serde_json::Value) -> Result<(), String> {
    // blocking file IO + fsync: keep it off the async workers other commands share
    tauri::async_runtime::spawn_blocking(move || {
        let (_held, json) = own(&app, window.label())?;
        write_atomic(&json, doc.to_string().as_bytes()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn recovery_clear(window: tauri::Window) -> Result<(), String> {
    let held = OWN.lock().map_err(|e| e.to_string())?;
    match held.get(window.label()) {
        Some((_, path)) if path.exists() => std::fs::remove_file(path).map_err(|e| e.to_string()),
        _ => Ok(()), // never saved anything: nothing to create or remove
    }
}

/// A closed window lets go of its recovery file: unlocked, so unsaved work it left (closed
/// without asking: never, but a crash of its page) is an orphan to restore; nothing left: gone.
fn recovery_release(label: &str) {
    if let Some((f, json)) = OWN.lock().ok().and_then(|mut h| h.remove(label)) {
        if !json.exists() {
            let _ = std::fs::remove_file(json.with_extension("lock"));
        }
        drop(f);
    }
    drop(CLAIM.lock().ok().and_then(|mut c| c.remove(label))); // (undecided: stays for the next launch)
}

/// Unsaved documents in `dir` of closed windows and dead instances, newest first, each locked by
/// us while held (dropped: free again). `limit`: at most this many.
fn orphans(dir: &std::path::Path, limit: usize) -> Vec<(std::fs::File, std::path::PathBuf, serde_json::Value)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut found = vec![];
    for p in entries.flatten().map(|e| e.path()) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = p.extension().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
        if !name.starts_with("recovery-") {
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
            continue; // an open window owns it (ours too: a second handle cannot lock it either)
        }
        let json = p.with_extension("json");
        match std::fs::metadata(&json).and_then(|m| m.modified()) {
            Ok(t) => found.push((t, f, p)),
            // clean exit; "old" so a lock file another instance just created is never removed
            Err(_) if is_stale(&p) => drop((f, std::fs::remove_file(&p))),
            Err(_) => {}
        }
    }
    found.sort_by_key(|o| std::cmp::Reverse(o.0));
    let mut out = vec![];
    for (_, f, lock) in found {
        if out.len() == limit {
            break;
        }
        let json = lock.with_extension("json");
        let Ok(bytes) = std::fs::read(&json) else { continue }; // transient read error: keep it
        match serde_json::from_slice(&bytes) {
            Ok(doc) => out.push((f, lock, doc)),
            Err(_) => {
                // corrupt: drop it (still locked by us) so older valid sessions get their turn
                let _ = std::fs::remove_file(json);
                let _ = std::fs::remove_file(&lock);
            }
        }
    }
    out
}

/// The newest unsaved document left by a closed window or a dead instance, for this window. It
/// stays on disk, locked by us, until `recovery_resolve`.
#[tauri::command]
async fn recovery_take(app: tauri::AppHandle, window: tauri::Window) -> Option<serde_json::Value> {
    let (f, lock, doc) = orphans(&recovery_dir(&app).ok()?, 1).pop()?;
    CLAIM.lock().ok()?.insert(window.label().to_string(), (f, lock));
    Some(doc)
}

/// How many more unsaved documents wait to be restored (each in a window of its own).
#[tauri::command]
async fn recovery_count(app: tauri::AppHandle) -> usize {
    recovery_dir(&app).map_or(0, |d| orphans(&d, usize::MAX).len())
}

/// The user answered the restore prompt. `keep`: the orphan becomes this window's recovery
/// file (atomic rename, no round trip through the UI); otherwise it is deleted. Both happen while
/// still holding the orphan's lock, so no other window or instance can claim it in between.
#[tauri::command]
async fn recovery_resolve(app: tauri::AppHandle, window: tauri::Window, keep: bool) -> Result<(), String> {
    let Some((f, lock)) = CLAIM.lock().map_err(|e| e.to_string())?.remove(window.label()) else { return Ok(()) };
    let json = lock.with_extension("json");
    if keep {
        let (_held, dest) = own(&app, window.label())?;
        std::fs::rename(&json, dest).map_err(|e| e.to_string())?;
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
    use tauri::Emitter;
    let _ = app.emit("libraries-changed", ()); // (every window's parts from them)
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
        git::show(std::path::Path::new(&path), &hash).map(|b| schematic::load(&path, &b).0) // (a symbol's as one)
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

/// The menu bar: the commands of the toolbar, hotkeys and context menus, each sent to the focused
/// window as a "menu" event (its id; New and Quit are handled here). A key shown in a menu is the
/// page's first (its hotkeys); one the page leaves (a dialog open, typing) reaches the menu, which
/// then decides as the page would. macOS: Quit asks every window about unsaved changes first (the
/// system's Quit cannot be vetoed); Undo, Redo and Select All are the page's (the schematic's, or
/// a text field's), Cut, Copy and Paste the system's (a text field's; the schematic's by key).
/// Windows: the system's edit items press their keys (Ctrl+C …) into the page.
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem as P, Submenu, HELP_SUBMENU_ID};
    let it = |id: &str, text: &str, key: Option<&str>| MenuItem::with_id(app, id, text, true, key);
    // a single key (R: resistor) as the menu shows it; not on Linux, where GTK takes it before the page
    let key = |k: &'static str| if cfg!(target_os = "linux") { None } else { Some(k) };
    let sep = || P::separator(app);
    #[cfg(target_os = "macos")]
    let app_menu = Submenu::with_items(app, "nomospice", true, &[
        &it("about", "About nomospice", None)?,
        &it("check-updates", "Check for Updates…", None)?,
        &sep()?,
        &it("libraries", "Model Libraries…", Some("CmdOrCtrl+,"))?,
        &sep()?,
        &P::services(app, None)?,
        &sep()?,
        &P::hide(app, None)?,
        &P::hide_others(app, None)?,
        &P::show_all(app, None)?,
        &sep()?,
        &it("quit", "Quit nomospice", Some("CmdOrCtrl+Q"))?,
    ])?;
    let file = Submenu::with_items(app, "File", true, &[
        &it("new", "New Window", Some("CmdOrCtrl+N"))?,
        &it("new-symbol", "New Symbol", None)?,
        &it("open", "Open…", Some("CmdOrCtrl+O"))?,
        &sep()?,
        &it("close", "Close", Some("CmdOrCtrl+W"))?,
        &it("save", "Save", Some("CmdOrCtrl+S"))?,
        &it("save-as", "Save As…", Some("CmdOrCtrl+Shift+S"))?,
        &sep()?,
        &it("block-symbol", "Create Block Symbol…", None)?,
        &sep()?,
        &it("export-svg", "Export Schematic as SVG…", None)?,
        &it("export-plot", "Export Plot as SVG…", None)?,
        &it("export-raw", "Export Results as .raw…", None)?,
        #[cfg(not(target_os = "macos"))]
        &sep()?,
        #[cfg(not(target_os = "macos"))]
        &it("libraries", "Model Libraries…", Some("CmdOrCtrl+,"))?,
        #[cfg(not(target_os = "macos"))]
        &sep()?,
        #[cfg(not(target_os = "macos"))]
        &it("quit", "Exit", None)?,
    ])?;
    let edit = Submenu::with_items(app, "Edit", true, &[
        #[cfg(target_os = "macos")]
        &it("undo", "Undo", Some("CmdOrCtrl+Z"))?,
        #[cfg(target_os = "macos")]
        &it("redo", "Redo", Some("CmdOrCtrl+Shift+Z"))?,
        #[cfg(not(target_os = "macos"))]
        &P::undo(app, None)?,
        #[cfg(not(target_os = "macos"))]
        &P::redo(app, None)?,
        &sep()?,
        &P::cut(app, None)?,
        &P::copy(app, None)?,
        &P::paste(app, None)?,
        &it("duplicate", "Duplicate", Some("CmdOrCtrl+D"))?,
        &it("delete", "Delete", key(if cfg!(target_os = "macos") { "Backspace" } else { "Delete" }))?,
        #[cfg(target_os = "macos")]
        &it("select-all", "Select All", Some("CmdOrCtrl+A"))?,
        #[cfg(not(target_os = "macos"))]
        &P::select_all(app, None)?,
        &sep()?,
        &it("rotate", "Rotate", Some("Ctrl+R"))?,
        &it("mirror", "Mirror", Some("Ctrl+E"))?,
        &sep()?,
        &it("find", "Find…", Some("CmdOrCtrl+F"))?,
    ])?;
    let place = Submenu::with_items(app, "Place", true, &[
        &it("wire", "Wire", key("W"))?,
        &it("ground", "Ground", key("G"))?,
        &it("label", "Net Label…", key("N"))?,
        &sep()?,
        &it("res", "Resistor", key("R"))?,
        &it("cap", "Capacitor", key("C"))?,
        &it("ind", "Inductor", key("L"))?,
        &it("diode", "Diode", key("D"))?,
        &it("voltage", "Voltage Source", key("V"))?,
        &it("current", "Current Source", key("I"))?,
        &it("component", "Component…", Some("CmdOrCtrl+K"))?,
        &sep()?,
        &it("comment", "Comment…", key("T"))?,
        &it("directive", "SPICE Directive…", key("S"))?,
        &sep()?,
        &it("select", "Select", None)?,
        &it("move", "Move", key("M"))?,
        &it("delete-tool", "Delete Mode", None)?,
    ])?;
    let simulate = Submenu::with_items(app, "Simulate", true, &[
        &it("run", "Run", Some("CmdOrCtrl+Enter"))?,
        &it("halt", "Halt", None)?,
        &sep()?,
        &it("sim-command", "Edit Simulation Command…", None)?,
        &it("live", "Live Mode: Re-run on Every Edit", None)?,
        &sep()?,
        &it("add-trace", "Add Trace…", None)?,
    ])?;
    let view = Submenu::with_items(app, "View", true, &[
        &it("fit", "Fit to Window", None)?,
        &it("zoom-in", "Zoom In", Some("CmdOrCtrl+="))?,
        &it("zoom-out", "Zoom Out", Some("CmdOrCtrl+-"))?,
        &sep()?,
        &it("panel", "Log & Netlist", Some("CmdOrCtrl+L"))?,
        &it("history", "Version History", None)?,
        &sep()?,
        &it("theme", "Light or Dark Theme", None)?,
        &it("up", "Back to the Schematic Above", None)?,
        #[cfg(target_os = "macos")]
        &sep()?,
        #[cfg(target_os = "macos")]
        &P::fullscreen(app, None)?,
    ])?;
    #[cfg(target_os = "macos")]
    let window = Submenu::with_id_and_items(app, tauri::menu::WINDOW_SUBMENU_ID, "Window", true, &[&P::minimize(app, None)?, &P::maximize(app, None)?])?;
    let help = Submenu::with_id_and_items(app, HELP_SUBMENU_ID, "Help", true, &[
        &it("help", "Quick Reference", None)?,
        #[cfg(not(target_os = "macos"))]
        &sep()?,
        #[cfg(not(target_os = "macos"))]
        &it("check-updates", "Check for Updates…", None)?,
        #[cfg(not(target_os = "macos"))]
        &it("about", "About nomospice", None)?,
    ])?;
    let menu = Menu::with_items(app, &[
        #[cfg(target_os = "macos")]
        &app_menu,
        &file,
        &edit,
        &place,
        &simulate,
        &view,
        #[cfg(target_os = "macos")]
        &window,
        &help,
    ])?;
    #[cfg(target_os = "macos")]
    {
        window.set_as_windows_menu_for_nsapp()?; // (the open windows listed, Merge All Windows)
        help.set_as_help_menu_for_nsapp()?;
    }
    Ok(menu)
}

/// A menu item chosen: New and Quit here, the rest for the focused window.
fn on_menu(app: &tauri::AppHandle, id: &str) {
    use tauri::Emitter;
    match id {
        // every window asks about its unsaved changes and closes; the last one ends the app
        "quit" => drop(app.emit("quit-requested", ())),
        "new" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { open_window(&app, vec![]) });
        }
        id => {
            if let Some(label) = focused(app) {
                let _ = app.emit_to(tauri::EventTarget::webview_window(label), "menu", id);
            }
        }
    }
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
        .invoke_handler(tauri::generate_handler![symbols, examples, path_exists, open_asc, save_asc, block_symbol, export_svg, export_raw, netlist, simulate, root_locus, cancel_locus, folder_symbols, trace, run_id, halt, cli_args, recovery_save, recovery_take, recovery_count, recovery_resolve, recovery_clear,
            new_window, window_count, doc_path, focus_doc,
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
            app.set_menu(build_menu(app.handle())?)?;
            Ok(())
        })
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Focused(true) => drop(LAST_FOCUSED.lock().map(|mut l| *l = window.label().to_string())),
            tauri::WindowEvent::Destroyed => forget_window(window.label()),
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running nomospice");
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn orphans_newest_first_skipping_open_windows() {
        let dir = std::env::temp_dir().join(format!("nomospice-orphans-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t0 = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        let doc = |name: &str, json: &str, age: u64| {
            std::fs::File::create(dir.join(format!("{name}.lock"))).unwrap();
            let f = std::fs::File::create(dir.join(format!("{name}.json"))).unwrap();
            std::io::Write::write_all(&mut &f, json.as_bytes()).unwrap();
            f.set_modified(t0 + std::time::Duration::from_secs(age)).unwrap();
        };
        doc("recovery-1-1-main", r#"{"path":"old"}"#, 1);
        doc("recovery-1-2-doc-1", r#"{"path":"new"}"#, 2);
        doc("recovery-1-3-doc-2", r#"{"path":"open"}"#, 3); // a window still open: locked
        let open = std::fs::File::open(dir.join("recovery-1-3-doc-2.lock")).unwrap();
        open.try_lock().unwrap();
        doc("recovery-1-4-doc-3", "{", 4); // corrupt: removed
        let found = super::orphans(&dir, usize::MAX);
        let paths: Vec<_> = found.iter().map(|(_, _, d)| d["path"].as_str().unwrap()).collect();
        assert_eq!(paths, ["new", "old"]);
        assert!(!dir.join("recovery-1-4-doc-3.json").exists());
        // held while found: another look skips them; let go: there again (one at a time)
        assert!(super::orphans(&dir, usize::MAX).is_empty());
        drop(found);
        assert_eq!(super::orphans(&dir, 1).len(), 1);
        drop(open);
        std::fs::remove_dir_all(dir).ok();
    }

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
