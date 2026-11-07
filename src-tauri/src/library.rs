//! Model libraries: an index of every `.model` / `.subckt` in the configured library folders
//! (LTspice's lib/cmp + lib/sub when installed, a Micro-Cap 12 LIBRARY folder, user folders),
//! searchable from the UI and used by the netlister to pull in what a schematic references.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Source {
    pub name: String,
    pub path: PathBuf,
    pub enabled: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Entry {
    /// the symbol to place it with (`symbol_for`)
    pub symbol: String,
    pub name: String,
    /// model type (D, NPN, NMOS, VDMOS ...) or "SUBCKT"
    pub kind: String,
    /// subckt pin names in order (empty for models)
    pub pins: Vec<String>,
    pub source: String,
    pub file: PathBuf,
    #[serde(skip)]
    start: usize,
    #[serde(skip)]
    end: usize,
}

#[derive(Default)]
pub struct Index {
    pub entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
    /// modification time of each indexed file: offsets are only valid while it is unchanged
    stamps: HashMap<PathBuf, Option<std::time::SystemTime>>,
    /// decoded text of recently read files (netlisting reads the same few files again and again)
    cache: Mutex<Vec<(PathBuf, Arc<String>)>>,
}

/// The `file` of the built-in models' entries: none (an empty path; the UI shows no file line).
const BUILTIN: &str = "";

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Text library files (LTspice: .lib/.sub/.mod/.bjt/.dio/.jft/.mos; Micro-Cap: .lib).
/// Micro-Cap's binary .lbr files are not readable.
fn is_library_file(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    ["lib", "sub", "mod", "bjt", "dio", "jft", "mos", "cir", "inc", "txt", "sp", "spi"].contains(&ext.as_str())
}

fn files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() && depth < 4 {
            files(&p, out, depth + 1);
        } else if is_library_file(&p) {
            out.push(p);
        }
    }
}

impl Index {
    pub fn build(sources: &[Source]) -> Index {
        let mut ix = Index::default();
        for s in sources.iter().filter(|s| s.enabled) {
            let mut list = vec![];
            files(&s.path, &mut list, 0);
            list.sort();
            for f in list {
                let Ok(bytes) = std::fs::read(&f) else { continue };
                ix.stamps.insert(f.clone(), mtime(&f));
                ix.scan(&crate::schematic::decode(&bytes), &s.name, &f);
            }
        }
        // nomospice's own defaults (what the netlister falls back to), after the libraries: a
        // library's part of the same name wins, and the list is never empty
        ix.scan(crate::schematic::builtin_cards(), "nomospice (built-in)", Path::new(BUILTIN));
        ix
    }

    /// One file: top-level `.model` cards and `.subckt` ... `.ends` blocks.
    fn scan(&mut self, text: &str, source: &str, file: &Path) {
        let lines: Vec<(usize, &str)> = text.split_inclusive('\n').scan(0, |pos, l| {
            let at = *pos;
            *pos += l.len();
            Some((at, l))
        }).collect();
        let mut i = 0;
        while i < lines.len() {
            let (start, line) = lines[i];
            let t = line.trim();
            let lower = t.to_lowercase();
            let mut end = i + 1;
            while end < lines.len() && lines[end].1.trim_start().starts_with('+') {
                end += 1; // continuation lines
            }
            if lower.starts_with(".model ") {
                let toks: Vec<&str> = t.split(|c: char| c.is_whitespace() || c == '(').filter(|s| !s.is_empty()).collect();
                if let (Some(name), Some(kind)) = (toks.get(1), toks.get(2)) {
                    let kind = kind.strip_prefix("ako:").map_or(kind.to_uppercase(), |_| toks.get(3).unwrap_or(kind).to_uppercase());
                    self.add(name, &kind, vec![], source, file, start, lines.get(end).map_or(text.len(), |l| l.0));
                }
                i = end;
            } else if lower.starts_with(".subckt ") {
                let header: String = lines[i..end].iter().map(|l| l.1.trim().trim_start_matches('+')).collect::<Vec<_>>().join(" ");
                let toks: Vec<&str> = header.split_whitespace().collect();
                let pins: Vec<String> = toks.iter().skip(2).take_while(|t| !t.contains('=') && !t.eq_ignore_ascii_case("params:") && !t.eq_ignore_ascii_case("optional:")).map(|s| s.to_string()).collect();
                // the block runs to its .ends (nested .subckt definitions are rare in vendor files)
                let mut j = end;
                while j < lines.len() && !lines[j].1.trim_start().to_lowercase().starts_with(".ends") {
                    j += 1;
                }
                let stop = lines.get(j + 1).map_or(text.len(), |l| l.0);
                if let Some(name) = toks.get(1) {
                    self.add(name, "SUBCKT", pins, source, file, start, stop);
                }
                i = j + 1;
            } else {
                i = end;
            }
        }
    }

    fn add(&mut self, name: &str, kind: &str, pins: Vec<String>, source: &str, file: &Path, start: usize, end: usize) {
        let key = name.to_lowercase();
        if self.by_name.contains_key(&key) {
            return; // first source wins (sources are ordered by priority)
        }
        self.by_name.insert(key, self.entries.len());
        let mut e = Entry { symbol: String::new(), name: name.to_string(), kind: kind.to_string(), pins, source: source.into(), file: file.into(), start, end };
        e.symbol = symbol_for(&e);
        self.entries.push(e);
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.by_name.get(&name.to_lowercase()).map(|&i| &self.entries[i])
    }

    /// The definition's text (the index keeps offsets; files are read on demand, a few cached).
    /// A file edited since indexing invalidates the index (rebuilt on next use) instead of
    /// returning text from stale offsets.
    pub fn text(&self, e: &Entry) -> Option<String> {
        if e.file == Path::new(BUILTIN) {
            return crate::schematic::builtin_cards().get(e.start..e.end).map(str::to_string);
        }
        if self.stamps.get(&e.file).copied().flatten() != mtime(&e.file) {
            GENERATION.fetch_add(1, Ordering::SeqCst);
            return None;
        }
        let mut cache = self.cache.lock().ok()?;
        let text = match cache.iter().position(|(p, _)| p == &e.file) {
            Some(i) => cache.remove(i).1,
            None => Arc::new(crate::schematic::decode(&std::fs::read(&e.file).ok()?)),
        };
        cache.insert(0, (e.file.clone(), text.clone()));
        cache.truncate(8);
        text.get(e.start..e.end).map(str::to_string)
    }

    /// `name` plus everything it references that is defined in the libraries (models used
    /// inside a subckt, nested subckts), each once, dependencies first.
    /// `seen` grows by everything emitted, so shared dependencies are written once per netlist.
    pub fn resolve(&self, name: &str, seen: &mut HashSet<String>) -> Vec<String> {
        let mut out = vec![];
        self.collect(name, seen, &mut out, 0);
        out
    }

    fn collect(&self, name: &str, seen: &mut HashSet<String>, out: &mut Vec<String>, depth: usize) {
        if depth > 8 || !seen.insert(name.to_lowercase()) {
            return;
        }
        let Some(e) = self.get(name) else { return };
        let Some(text) = self.text(e) else { return };
        if e.kind == "SUBCKT" {
            // names defined inside the block stay local
            let local: HashSet<String> = text.lines().filter_map(|l| {
                let t: Vec<&str> = l.split(|c: char| c.is_whitespace() || c == '(').filter(|s| !s.is_empty()).collect();
                matches!(t.first().map(|s| s.to_lowercase()).as_deref(), Some(".model" | ".subckt")).then(|| t.get(1).map(|s| s.to_lowercase()))?
            }).collect();
            for line in text.lines().skip(1).map(str::trim).filter(|l| !l.starts_with(['*', '.', '+'])) {
                for r in model_refs(line) {
                    let t = r.to_lowercase();
                    if !local.contains(&t) && self.by_name.contains_key(&t) {
                        self.collect(r, seen, out, depth + 1);
                    }
                }
            }
        }
        out.push(if e.kind == "SUBCKT" { subckt_header(&text) } else { self.ako(&text, depth).unwrap_or(text) });
    }

    /// LTspice `ako:` ("a kind of"): `.model ZTX849 ako:FZT849 NPN(overrides)`; ngspice has no
    /// such syntax, so write the base card's parameters followed by the overrides (merged by key:
    /// ngspice dies on a repeated string parameter like mfg=).
    fn ako(&self, card: &str, depth: usize) -> Option<String> {
        // one line, without `; comments` (a `(C) 1993` in a comment must not end the parameter list)
        let flatten = |s: &str| s.lines().map(|l| l.split(';').next().unwrap_or("").trim().trim_start_matches('+')).collect::<Vec<_>>().join(" ");
        let flat = flatten(card);
        let at = flat.to_lowercase().find("ako:")?;
        let name = flat.split_whitespace().nth(1)?.to_string();
        let base_name: String = flat[at + 4..].chars().take_while(|c| !c.is_whitespace() && *c != '(').collect();
        let base_entry = self.get(&base_name)?;
        let base_text = self.text(base_entry)?;
        let base = flatten(&if depth < 8 { self.ako(&base_text, depth + 1).unwrap_or(base_text) } else { base_text });
        let inner = |s: &str| s.find('(').zip(s.rfind(')')).map(|(a, b)| s[a + 1..b].to_string()).unwrap_or_default();
        let ty = flat[at + 4 + base_name.len()..].split(|c: char| c.is_whitespace() || c == '(').find(|s| !s.is_empty())?.to_string();
        let (base_p, over_p) = (inner(&base), inner(&flat));
        let key = |t: &str| t.split('=').next().unwrap_or(t).trim().to_lowercase();
        let over = crate::schematic::spice_tokens(&over_p);
        let mut params: Vec<&str> = crate::schematic::spice_tokens(&base_p).into_iter().filter(|b| !over.iter().any(|o| key(o) == key(b))).collect();
        params.extend(over);
        Some(format!(".model {name} {ty}({})", params.join(" ")))
    }

    /// Case-insensitive substring search over names; exact and prefix matches first.
    /// `kind`: one type or several separated by `|` (`NPN|PNP`).
    pub fn search(&self, query: &str, kind: Option<&str>, limit: usize) -> Vec<&Entry> {
        let q = query.trim().to_lowercase();
        let kinds: Option<Vec<&str>> = kind.map(|k| k.split('|').collect());
        let mut hits: Vec<(u8, &Entry)> = self.entries.iter()
            .filter(|e| kinds.as_ref().is_none_or(|ks| ks.iter().any(|k| e.kind.eq_ignore_ascii_case(k))))
            .filter_map(|e| {
                let n = e.name.to_lowercase();
                let rank = if n == q { 0 } else if n.starts_with(&q) { 1 } else if n.contains(&q) { 2 } else { return None };
                Some((rank, e))
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.len().cmp(&b.1.name.len())).then(a.1.name.cmp(&b.1.name)));
        hits.into_iter().take(limit).map(|h| h.1).collect()
    }
}

/// One header line with at most one `params:` (Micro-Cap writes a `+PARAMS:` line per parameter,
/// which ngspice reads as extra pins: "too few parameters for subcircuit").
fn subckt_header(text: &str) -> String {
    let mut lines = text.lines().peekable();
    let mut header = lines.next().unwrap_or_default().trim().to_string();
    while let Some(l) = lines.next_if(|l| l.trim_start().starts_with('+')) {
        header.push(' ');
        header.push_str(l.trim_start().trim_start_matches('+').trim());
    }
    let mut seen = false;
    let toks: Vec<&str> = header.split_whitespace().filter(|t| !t.eq_ignore_ascii_case("params:") || !std::mem::replace(&mut seen, true)).collect();
    std::iter::once(toks.join(" ")).chain(lines.map(str::to_string)).collect::<Vec<_>>().join("\n") + "\n"
}

/// Model or subckt names an element line refers to, by SPICE position only (element and node
/// names must not be mistaken for library parts: `q1` in a UA741 is not the vendor subckt `Q1`).
fn model_refs(line: &str) -> Vec<&str> {
    let t: Vec<&str> = line.split_whitespace().take_while(|t| !t.contains('=') && !t.eq_ignore_ascii_case("params:")).collect();
    let at = |i: usize| t.get(i).copied();
    match line.chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('d') => at(3).into_iter().collect(),
        Some('j') => at(4).into_iter().collect(),
        Some('w') => at(4).into_iter().collect(),
        Some('q') => [at(4), at(5)].into_iter().flatten().collect(), // optional substrate node
        Some('m') => [at(4), at(5)].into_iter().flatten().collect(), // 3-terminal VDMOS or 4
        Some('s') => at(5).into_iter().collect(),
        Some('x') => t.last().copied().filter(|_| t.len() > 2).into_iter().collect(),
        _ => vec![],
    }
}

/// Built-in symbol for a library part: by model type, or a generated box for a subckt;
/// "" for model types without a symbol (switches, cores, digital ...): listed, not placeable.
pub fn symbol_for(e: &Entry) -> String {
    match e.kind.as_str() {
        "D" => "diode",
        "NPN" => "npn",
        "PNP" => "pnp",
        "NMOS" => "nmos",
        "PMOS" => "pmos",
        "VDMOS" => "nmos", // LTspice VDMOS: pchan cards are rare; the user can swap
        "NJF" => "njf",
        "PJF" => "pjf",
        "SW" => "sw",
        "CSW" => "csw",
        _ if e.kind == "SUBCKT" => return format!("subckt:{}", e.name),
        _ => "",
    }.to_string()
}

// ---------------------------------------------------------------- the global index

/// None: never configured (use the detected libraries); Some([]) is a deliberate empty list.
static SOURCES: RwLock<Option<Vec<Source>>> = RwLock::new(None);
static INDEX: RwLock<Option<Index>> = RwLock::new(None);
/// Bumped whenever the index must be rebuilt (sources changed, a library file was edited);
/// generated symbols and builds started before a bump are thrown away.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static BUILD: Mutex<()> = Mutex::new(());

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

fn home() -> String {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default()
}

/// Libraries found on this machine: LTspice's lib/cmp + lib/sub, a Micro-Cap 12 install.
pub fn detected() -> Vec<Source> {
    let h = home();
    let lt = crate::schematic::ltspice_lib_dirs().into_iter().flat_map(|l| [("LTspice models", l.join("cmp")), ("LTspice subcircuits", l.join("sub"))]);
    let mc = [
        format!("{h}/MC12/library"), format!("{h}/Documents/MC12/library"), format!("{h}/.wine/drive_c/MC12/library"),
        "C:/MC12/library".into(), "C:/Program Files (x86)/Micro-Cap 12/library".into(), "C:/Program Files/Micro-Cap 12/library".into(),
    ].map(|p| ("Micro-Cap 12", PathBuf::from(p)));
    lt.chain(mc).filter(|(_, p)| p.is_dir()).map(|(n, p)| Source { name: n.into(), path: p, enabled: true }).collect()
}

pub fn sources() -> Vec<Source> {
    SOURCES.read().ok().and_then(|s| s.clone()).unwrap_or_else(detected)
}

/// New source list: the index is rebuilt on its next use.
pub fn set_sources(list: Vec<Source>) {
    if let Ok(mut s) = SOURCES.write() {
        *s = Some(list);
    }
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// The index for the current sources, (re)built on demand (some 100 ms for LTspice + Micro-Cap).
/// One build at a time; a build overtaken by a source change is not installed.
pub fn index() -> RwLockReadGuard<'static, Option<Index>> {
    let current = |ix: &Option<Index>, g: u64| ix.is_some() && BUILT.load(Ordering::SeqCst) == g;
    let g = generation();
    if !INDEX.read().map(|ix| current(&ix, g)).unwrap_or(false) {
        let _one = BUILD.lock().unwrap_or_else(|e| e.into_inner());
        let g = generation();
        if !INDEX.read().map(|ix| current(&ix, g)).unwrap_or(false) {
            let built = Index::build(&sources());
            if generation() == g {
                if let Ok(mut ix) = INDEX.write() {
                    *ix = Some(built);
                    BUILT.store(g, Ordering::SeqCst);
                }
            }
        }
    }
    INDEX.read().unwrap_or_else(|e| e.into_inner())
}

/// The generation the installed index was built for.
static BUILT: AtomicU64 = AtomicU64::new(u64::MAX);

/// Library definitions a netlist needs for `name` (with its dependencies), or None.
pub fn definitions(name: &str, seen: &mut HashSet<String>) -> Option<Vec<String>> {
    let ix = index();
    let ix = ix.as_ref()?;
    ix.get(name)?;
    let defs = ix.resolve(name, seen);
    if defs.is_empty() {
        seen.remove(&name.to_lowercase()); // unreadable (file edited since indexing): let the caller fall back / warn
        return None;
    }
    Some(defs)
}

/// A box symbol for a library subckt: pins left and right, in .subckt order, with labels.
pub fn subckt_symbol(name: &str) -> Option<crate::schematic::SymbolDef> {
    let ix = index();
    let e = ix.as_ref()?.get(name)?;
    let n = e.pins.len();
    let left = n.div_ceil(2);
    let rows = left.max(n - left).max(1) as i32;
    let (w, h) = (64, 32 * rows); // pins on the 16-unit grid: wires must be able to reach them
    let mut d = crate::schematic::SymbolDef {
        name: format!("subckt:{}", e.name.to_lowercase()),
        prefix: "X".into(),
        value: e.name.clone(),
        desc: format!("{} subcircuit ({})", e.name, e.source),
        pin_labels: true,
        ..Default::default()
    };
    d.shapes.push(("rect".into(), vec![0, 0, w, h]));
    for (k, pin) in e.pins.iter().enumerate() {
        let (side, row) = if k < left { (0, k) } else { (1, k - left) };
        let y = 32 * row as i32 + 16;
        let (px, edge) = if side == 0 { (-32, 0) } else { (w + 32, w) };
        d.shapes.push(("line".into(), vec![px, y, edge, y]));
        d.pins.push(crate::schematic::Pin { x: px, y, name: pin.clone() });
    }
    d.windows.insert(0, (0, -12));
    d.windows.insert(3, (0, h + 12));
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_models_listed_after_libraries() {
        let ix = Index::build(&[]);
        let d = ix.get("1n4148").expect("built-in 1N4148");
        assert_eq!((d.kind.as_str(), d.symbol.as_str(), d.source.as_str()), ("D", "diode", "nomospice (built-in)"));
        assert!(ix.text(d).unwrap().starts_with(".model 1N4148 D(IS=2n"), "{:?}", ix.text(d));
        assert_eq!(ix.resolve("2N3904", &mut HashSet::new()).len(), 1);
        assert!(ix.get("d").is_none() && ix.get("opamp").is_none(), "type defaults and the opamp are not parts");
        let dir = std::env::temp_dir().join(format!("nomospice-builtin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("v.lib"), ".model 1N4148 D(IS=4n)\n").unwrap();
        let ix = Index::build(&[Source { name: "vendor".into(), path: dir.clone(), enabled: true }]);
        assert_eq!(ix.get("1N4148").unwrap().source, "vendor", "a library wins");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn indexes_models_and_subckts_with_dependencies() {
        let dir = std::env::temp_dir().join(format!("nomospice-lib-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.lib"), "* vendor\n.model QX NPN(BF=100)\n.subckt AMP in out vcc\nQ1 out in 0 QX\n.model QLOCAL PNP\n.ends\n.model BASE NPN(BF=50 mfg=X)\n.model KIND ako:BASE NPN(mfg=Y)\n").unwrap();
        let ix = Index::build(&[Source { name: "test".into(), path: dir.clone(), enabled: true }]);
        let amp = ix.get("amp").unwrap();
        assert_eq!((amp.kind.as_str(), amp.pins.clone(), amp.symbol.as_str()), ("SUBCKT", vec!["in".to_string(), "out".into(), "vcc".into()], "subckt:AMP"));
        let defs = ix.resolve("AMP", &mut HashSet::new());
        assert_eq!(defs.len(), 2, "{defs:?}"); // QX first, then AMP; QLOCAL stays inside
        assert!(defs[0].starts_with(".model QX") && defs[1].starts_with(".subckt AMP"));
        let kind = ix.resolve("KIND", &mut HashSet::new());
        assert_eq!(kind[0], ".model KIND NPN(BF=50 mfg=Y)", "{kind:?}");
        assert_eq!(ix.search("a", Some("SUBCKT"), 10).len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }
}

/// `NOMOSPICE_MC12=<Micro-Cap LIBRARY folder> cargo test microcap -- --ignored --nocapture`
#[test]
#[ignore]
fn microcap_parts_simulate() {
    use crate::schematic::*;
    let _s = crate::ngspice::TEST_SIM.lock();
    let dir = std::env::var("NOMOSPICE_MC12").expect("NOMOSPICE_MC12");
    set_sources(vec![Source { name: "Micro-Cap 12".into(), path: dir.into(), enabled: true }]);
    let mut lib = Library::builtin();
    let ua = lib.get("subckt:UA741").cloned().expect("generated symbol");
    println!("UA741 pins: {:?}", ua.pins.iter().map(|p| (&p.name, p.x, p.y)).collect::<Vec<_>>());
    // inverting amplifier around the Micro-Cap UA741 (pins 1 2 3 4 5 = in+ in- v+ v- out) + a 2N2222 stage
    let at = |k: usize| (ua.pins[k].x, ua.pins[k].y);
    let (p, n, vp, vn, o) = (at(0), at(1), at(2), at(3), at(4));
    let src = format!(
        "SYMBOL subckt:UA741 0 0 R0\nSYMATTR InstName U1\nSYMATTR Value UA741\n\
         FLAG {} {} 0\nFLAG {} {} inm\nFLAG {} {} vcc\nFLAG {} {} vee\nFLAG {} {} out\n\
         SYMBOL res 400 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\nFLAG 416 16 in\nFLAG 416 96 inm\n\
         SYMBOL res 500 0 R0\nSYMATTR InstName R2\nSYMATTR Value 10k\nFLAG 516 16 inm\nFLAG 516 96 out\n\
         SYMBOL voltage 600 0 R0\nSYMATTR InstName V1\nSYMATTR Value 15\nFLAG 600 16 vcc\nFLAG 600 96 0\n\
         SYMBOL voltage 700 0 R0\nSYMATTR InstName V2\nSYMATTR Value -15\nFLAG 700 16 vee\nFLAG 700 96 0\n\
         SYMBOL voltage 800 0 R0\nSYMATTR InstName V3\nSYMATTR Value SINE(0 0.1 1k)\nFLAG 800 16 in\nFLAG 800 96 0\n\
         SYMBOL npn 900 0 R0\nSYMATTR InstName Q1\nSYMATTR Value 2N2222\nFLAG 964 0 vcc\nFLAG 900 48 out\nFLAG 964 96 e\n\
         SYMBOL res 948 96 R0\nSYMATTR InstName R3\nSYMATTR Value 1k\nFLAG 964 112 e\nFLAG 964 192 0\n\
         TEXT 0 300 Left 2 !.tran 3m\n",
        p.0, p.1, n.0, n.1, vp.0, vp.1, vn.0, vn.1, o.0, o.1
    );
    let nl = netlist_for_sim(&parse_asc(&src), &mut lib);
    assert!(nl.warnings.is_empty(), "{:?}\n{}", nl.warnings, nl.text);
    let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).unwrap_or_else(|l| panic!("{}\n{}", nl.text, l.join("\n")));
    let out = crate::ngspice::trace("v(\"out\")", &run.scale, run.id).unwrap();
    let late: Vec<f64> = out.x.iter().zip(&out.re).filter(|(t, _)| **t > 1e-3).map(|(_, v)| *v).collect();
    let (lo, hi) = late.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    println!("V(out) after 1 ms: min {lo:.3} max {hi:.3}");
    let peak = (hi - lo) / 2.0;
    println!("|V(out)| peak = {peak:.3} V (expect ~1 V: gain -10 x 0.1 V)");
    assert!((peak - 1.0).abs() < 0.1, "{peak}");
}
