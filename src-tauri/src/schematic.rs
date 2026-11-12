//! Schematic model, LTspice .asc/.asy I/O and SPICE netlisting.
//! Coordinates are LTspice sheet units (grid = 16), y grows downward.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Item {
    Wire { x1: i32, y1: i32, x2: i32, y2: i32 },
    /// Net label; name "0" is ground. `extra`: IOPIN lines, kept for round-trips.
    Flag {
        x: i32,
        y: i32,
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extra: Vec<String>,
    },
    /// `extra`: WINDOW lines, kept for round-trips.
    Sym {
        sym: String,
        x: i32,
        y: i32,
        rot: String,
        attrs: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extra: Vec<String>,
    },
    /// `align`: LTspice "Left 2" etc.
    Text {
        x: i32,
        y: i32,
        text: String,
        directive: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        align: String,
    },
    /// The schematic's own drawing: `kind` LINE, RECTANGLE, CIRCLE or ARC as written, its pen
    /// (`Normal`, `Wide`), its points (x y pairs: two; an ARC four: box, start, end) and, if the
    /// file gives one, its line style (1 dash … 4 dash-dot-dot).
    Draw {
        kind: String,
        width: String,
        p: Vec<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<i32>,
    },
    /// Any .asc line we do not interpret (SHEET, BUSTAP, ...).
    Raw { line: String },
}

// ---------------------------------------------------------------- symbols

#[derive(Serialize, Clone, Debug, Default)]
pub struct Pin {
    pub x: i32,
    pub y: i32,
    pub name: String,
    /// where its name is drawn (LTspice: LEFT, RIGHT, TOP, BOTTOM, V…; NONE or "": not) and how far
    #[serde(skip_serializing_if = "String::is_empty")]
    pub just: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub off: i32,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct SymbolDef {
    pub name: String,
    pub prefix: String,
    pub value: String,
    pub spice_line: String,
    pub desc: String,
    /// Sorted by SpiceOrder.
    pub pins: Vec<Pin>,
    /// `line|rect|circle|arc` + LTspice coordinates (4 or 8 numbers).
    pub shapes: Vec<(String, Vec<i32>)>,
    /// WINDOW 0 (InstName) and 3 (Value) anchors.
    pub windows: BTreeMap<u8, (i32, i32)>,
    /// `TEXT x y Left 2 &`: x, y, justification (Left, Right, Center …), size (LTspice's 0–7), text
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<(i32, i32, String, i32, String)>,
    /// draw pin names (generated subckt boxes)
    pub pin_labels: bool,
    /// a hierarchical block: the schematic it stands for (`<name>.asc` next to it), netlisted as a
    /// subcircuit of that name with the symbol's pins as ports (labels of their names there)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// `SymbolType BLOCK` (LTspice's hierarchical block symbol)
    #[serde(skip)]
    pub block: bool,
    /// the file of a symbol of the schematic's folder (editable there)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// a `ModelFile` or `SpiceModel` attribute: a subcircuit symbol of a library, never a block
    #[serde(skip)]
    pub model_file: bool,
}

impl SymbolDef {
    /// Whether a schematic of its name would be its subcircuit: a BLOCK, or a symbol calling a
    /// subcircuit (Prefix X) without naming another one in its Value or a library file.
    pub fn calls_sheet(&self) -> bool {
        !self.model_file && (self.block || (matches!(self.prefix.trim(), "" | "X" | "x") && block_params(&self.value, &self.name).is_some()))
    }

    /// Whether `items` (a schematic of its name) can be its subcircuit: it does not place this
    /// symbol itself (a library part's test bench), and has a label for every pin.
    pub fn sheet_fits(&self, items: &[Item]) -> Result<(), String> {
        if items.iter().any(|it| matches!(it, Item::Sym { sym, .. } if sym_key(sym) == self.name)) {
            return Err("places its own block".into());
        }
        match self.pins.iter().find(|p| sheet_label(items, &p.name).is_none()) {
            Some(p) => Err(format!("has no label {} for the pin of that name", p.name)),
            None => Ok(()),
        }
    }
}

/// The label of a block's pin in its schematic (its index in the items).
fn sheet_label(items: &[Item], pin: &str) -> Option<usize> {
    items.iter().position(|it| matches!(it, Item::Flag { name, .. } if name.trim().eq_ignore_ascii_case(pin.trim())))
}


/// A block instance's parameters (`R=1k C=1n`) from its Value: None if the Value names a
/// subcircuit other than the block's own (a model call, not the sheet).
fn block_params<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let v = value.trim();
    let mut t = v.split_whitespace();
    match (t.next(), t.next()) {
        (None, _) => Some(""),
        (Some(w), _) if w.eq_ignore_ascii_case(name) => Some(v[w.len()..].trim_start()),
        // (`R=3k`, `R= 3k`, `R =3k`, `R = 3k`, `params: …`)
        (Some(w), next) if w.contains('=') || next.is_some_and(|n| n.starts_with('=')) || w.eq_ignore_ascii_case("params:") => Some(v),
        _ => None,
    }
}

macro_rules! builtin {
    ($($n:literal),*) => { &[$(($n, include_str!(concat!("../symbols/", $n, ".asy")))),*] };
}
const BUILTIN_SYMBOLS: &[(&str, &str)] = builtin!(
    "res", "cap", "polcap", "ind", "voltage", "current", "diode", "schottky", "zener", "led", "npn", "pnp",
    "nmos", "pmos", "njf", "pjf", "opamp", "e", "g", "jumper", "sw", "csw", "bv", "bi",
    "tline", "ltline", "opamp2", "and2", "nand2", "or2", "nor2", "xor2", "not", "dff"
);

pub fn parse_asy(name: &str, src: &str) -> SymbolDef {
    let mut d = SymbolDef { name: name.into(), ..Default::default() };
    let mut pins: Vec<(u32, Pin)> = vec![];
    for line in src.lines() {
        let tok: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| tok.get(i).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let rest = |n: usize| after_words(line, n).trim().to_string();
        match tok.first().copied() {
            Some(k @ ("LINE" | "RECTANGLE" | "CIRCLE" | "ARC")) => {
                let n = if k == "ARC" { 8 } else { 4 };
                let kind = match k { "LINE" => "line", "RECTANGLE" => "rect", "CIRCLE" => "circle", _ => "arc" };
                // coordinates, then LTspice's line style (0 solid, 1 dash, 2 dot, 3 dash-dot, 4 dash-dot-dot)
                d.shapes.push((kind.into(), (2..2 + n + 1).map(num).collect()));
            }
            Some("WINDOW") => {
                d.windows.insert(num(1) as u8, (num(2), num(3)));
            }
            Some("TEXT") if tok.len() > 5 => d.texts.push((num(1), num(2), tok[3].to_string(), num(4), unescape(&rest(5)))),
            Some("SymbolType") => d.block = tok.get(1).is_some_and(|t| t.eq_ignore_ascii_case("block")),
            Some("PIN") => {
                let just = tok.get(3).filter(|j| !j.eq_ignore_ascii_case("none")).map_or(String::new(), |j| j.to_ascii_uppercase());
                pins.push((pins.len() as u32 + 1, Pin { x: num(1), y: num(2), name: String::new(), just, off: num(4) }));
            }
            Some("PINATTR") => {
                if let Some(p) = pins.last_mut() {
                    match tok.get(1).copied() {
                        Some("PinName") => p.1.name = rest(2),
                        Some("SpiceOrder") => p.0 = num(2) as u32,
                        _ => {}
                    }
                }
            }
            Some("SYMATTR") => {
                let v = rest(2);
                match tok.get(1).copied() {
                    Some("Prefix") => d.prefix = v,
                    Some("Value") => d.value = v,
                    Some("SpiceLine") => d.spice_line = v,
                    Some("Description") => d.desc = v,
                    Some("ModelFile" | "SpiceModel") => d.model_file |= !v.trim().is_empty(),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    pins.sort_by_key(|p| p.0);
    d.pins = pins.into_iter().map(|p| p.1).collect();
    d
}

/// What follows the first `n` words of a line, however they are spaced (tabs, two spaces).
fn after_words(line: &str, n: usize) -> &str {
    let mut s = line.trim_start();
    for _ in 0..n {
        s = s.trim_start_matches(|c: char| !c.is_whitespace()).trim_start();
    }
    s
}

/// `Opamps\opamp` -> `opamp`
pub fn sym_key(name: &str) -> String {
    if name.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("subckt:")) { // not name[..7]: µ, Ω … are 2 bytes
        return name.to_lowercase(); // library subckt names may contain `/` (BS/MMBF170)
    }
    name.rsplit(['\\', '/']).next().unwrap_or(name).to_lowercase()
}

/// The symbols netlisting and drawing use.
pub struct Library {
    /// definitions by key
    pub defs: HashMap<String, SymbolDef>,
    /// keys looked for and not found (no disk search for them again)
    misses: HashSet<String>,
    /// the library generation the `subckt:` boxes and lent symbols belong to
    generation: u64,
    /// where symbols came from (the schematic's folder, library folders)
    local: Local,
    /// blocks' schematics as last read, by path: (modified, size, items), so netlisting on every
    /// edit reads only those that changed
    sheets: HashMap<String, (Option<std::time::SystemTime>, u64, std::sync::Arc<Vec<Item>>)>,
}

type Stamp = Option<((usize, u64), std::collections::BTreeSet<std::path::PathBuf>)>;

/// Where symbols came from, so they can go again.
#[derive(Default)]
pub struct Local {
    /// the schematic's folder
    dir: Option<std::path::PathBuf>,
    /// keys of the symbols loaded from it
    own: HashSet<String>,
    /// a fingerprint of its .asy files when they were (an added, removed, edited, renamed or
    /// replaced symbol changes it), and its schematics (blocks' sheets; a renamed one counts)
    stamp: Stamp,
    /// what each window was last told of each folder (`news_for`): windows take turns with the one
    /// library, and another window's turn is no news to this one
    seen: HashMap<(String, Option<std::path::PathBuf>), Stamp>,
    /// keys of symbols lent by library folders (they go when the libraries change)
    lent: HashSet<String>,
}

/// A folder's files.
fn listing(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect()
}

/// (key, path) of the files of that extension (any case), sorted: of Foo.asy and foo.asy on a
/// case-sensitive disk always the same one wins, the last.
fn keyed(files: &[std::path::PathBuf], ext: &str) -> Vec<(String, std::path::PathBuf)> {
    let mut list: Vec<_> = files.iter().filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
        .filter_map(|p| Some((sym_key(p.file_stem()?.to_str()?), p.clone())))
        .collect();
    list.sort();
    list
}

/// A folder's own symbol files by key (each .asy, not those of a built-in symbol: nomospice's
/// win) and its schematics by key (the sheets of its blocks), from one listing. Library::set_dir,
/// Save As (carry them along) and block symbols share it.
pub fn folder_contents(dir: &std::path::Path) -> (HashMap<String, std::path::PathBuf>, HashMap<String, std::path::PathBuf>) {
    let files = listing(dir);
    (keyed(&files, "asy").into_iter().filter(|(k, _)| !builtin_symbol(k)).collect(), keyed(&files, "asc").into_iter().collect())
}

/// (count, a sum over names, modification times and sizes) of a folder's symbol files (as
/// `folder_contents` lists them).
fn asy_stamp(list: &[(String, std::path::PathBuf)]) -> (usize, u64) {
    use std::hash::{Hash, Hasher};
    let mut files: Vec<_> = list.iter().filter_map(|(_, p)| Some((p.file_name()?.to_os_string(), p.metadata().ok()?))).map(|(name, m)| (name, m.modified().ok(), m.len())).collect();
    files.sort(); // (the listing's order is not fixed)
    let mut h = std::collections::hash_map::DefaultHasher::new();
    files.hash(&mut h); // names (a rename counts), mtimes, sizes
    (files.len(), h.finish())
}

impl Library {
    pub fn builtin() -> Self {
        Library {
            defs: BUILTIN_SYMBOLS.iter().map(|(n, s)| (n.to_string(), parse_asy(n, s))).collect(),
            misses: HashSet::new(),
            generation: crate::library::generation(),
            local: Local::default(),
            sheets: HashMap::new(),
        }
    }

    /// The schematic's folder: its symbols (LTspice looks there first) are all loaded, so the
    /// component list shows them; they replace one cached from LTspice or a library folder (it
    /// comes back with another folder), not a built-in symbol. Another folder (or
    /// none) drops them again; `rescan` reloads the same folder (a symbol saved meanwhile).
    /// (Whether that changed anything for a window: `news_for`.)
    pub fn set_dir(&mut self, dir: Option<&std::path::Path>, rescan: bool) {
        let moved = self.local.dir.as_deref() != dir;
        if !moved && !rescan {
            return; // netlisting: no disk access
        }
        let (list, sheets) = dir.map(folder_contents).unwrap_or_default(); // one listing for the stamp and the load
        let mut list: Vec<_> = list.into_iter().collect();
        list.sort();
        // (None: no such folder; a schematic added or removed beside a symbol makes it a block or not)
        let stamp = dir.and_then(|d| std::fs::metadata(d).ok()).map(|_| (asy_stamp(&list), sheets.values().cloned().collect::<std::collections::BTreeSet<_>>()));
        if !moved && stamp == self.local.stamp {
            return;
        }
        self.sheets.clear();
        self.local.stamp = stamp;
        for k in self.local.own.drain() {
            self.defs.remove(&k);
        }
        // (misses stay: they were looked up in LTspice's and the library folders, which did not
        // change; a name the new folder has is taken off them below)
        self.local.dir = dir.map(std::path::Path::to_path_buf);
        for (key, p) in list {
            match std::fs::read(&p) {
                Ok(bytes) => {
                    let mut def = parse_asy(&key, &decode(&bytes));
                    def.sheet = sheets.get(&key).filter(|_| def.calls_sheet()).map(|p| p.to_string_lossy().into_owned());
                    def.file = Some(p.to_string_lossy().into_owned());
                    self.defs.insert(key.clone(), def);
                    self.misses.remove(&key);
                    self.local.lent.remove(&key); // the folder's own now, no longer lent
                    self.local.own.insert(key);
                }
                // unreadable: left out; a write that finishes changes the stamp, and the next rescan reads it
                Err(_) => {}
            }
        }
    }

    /// A window closed: what it was told goes.
    pub fn forget(&mut self, who: &str) {
        self.local.seen.retain(|(w, _), _| w != who);
    }

    /// Whether the current folder's symbols changed since window `who` was last told (told now).
    pub fn news_for(&mut self, who: &str) -> bool {
        let key = (who.to_string(), self.local.dir.clone());
        self.local.seen.insert(key, self.local.stamp.clone()).as_ref() != Some(&self.local.stamp)
    }

    /// Resolve a symbol; unknown names fall back to a local LTspice install and the library folders
    /// (graphics + pins only).
    /// Misses are cached so netlisting never rescans the disk for the same name.
    pub fn get(&mut self, name: &str) -> Option<&SymbolDef> {
        let key = sym_key(name);
        self.sync();
        if self.misses.contains(&key) {
            return None;
        }
        self.load(name, key)
    }

    /// A run reads the blocks' schematics again: netlisting trusts their size and time, which an
    /// edit within the clock's granularity that keeps the size can fool.
    pub fn reread_sheets(&mut self) {
        self.sheets.clear();
    }

    /// A block's schematic: its items, read again only once the file changed.
    fn sheet(&mut self, path: &str) -> Result<std::sync::Arc<Vec<Item>>, String> {
        let m = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let stamp = (m.modified().ok(), m.len());
        if let Some((_, _, items)) = self.sheets.get(path).filter(|(t, n, _)| (*t, *n) == stamp) {
            return Ok(items.clone());
        }
        let items = std::sync::Arc::new(parse_asc(&decode(&std::fs::read(path).map_err(|e| e.to_string())?)));
        self.sheets.insert(path.to_string(), (stamp.0, stamp.1, items.clone()));
        Ok(items)
    }

    /// The library sources changed (generation): forget what they lent or lacked.
    pub fn sync(&mut self) {
        let g = crate::library::generation();
        if self.generation != g {
            // other libraries now: regenerate subckt boxes, drop symbols a library folder lent (it
            // may be off now: they come back from where they are), retry what was missing
            for k in self.local.lent.drain() {
                self.defs.remove(&k);
            }
            self.defs.retain(|k, _| !k.starts_with("subckt:"));
            self.misses.clear();
            self.generation = g;
        }
    }

    fn load(&mut self, name: &str, key: String) -> Option<&SymbolDef> {
        if let (false, Some(sub)) = (self.defs.contains_key(&key), key.strip_prefix("subckt:")) {
            // a library subcircuit placed from the library browser: generated box symbol
            match crate::library::subckt_symbol(sub) {
                Some(def) => self.defs.insert(key.clone(), def),
                None => return self.misses.insert(key).then_some(None).flatten(),
            };
        }
        if !self.defs.contains_key(&key) {
            let file = format!("{key}.asy");
            // (the library folders are asked for only when LTspice's do not have it)
            let folders = std::iter::once_with(|| crate::library::sources().into_iter().filter(|s| s.enabled).map(|s| (s.path, true))).flatten();
            let found = ltspice_sym_dirs().into_iter().map(|d| (d, false)).chain(folders).find_map(|(dir, lent)| {
                // folders only (the entry type comes with the listing: no stat per model file)
                let folder = |e: &std::fs::DirEntry| e.file_type().is_ok_and(|t| t.is_dir() || (t.is_symlink() && e.path().is_dir()));
                let subdirs = std::fs::read_dir(&dir).into_iter().flatten().flatten().filter(folder).map(|e| e.path());
                let path = std::iter::once(dir.join(name.replace('\\', "/") + ".asy"))
                    .chain(std::iter::once(dir.clone()).chain(subdirs).map(|d| d.join(&file)))
                    .find(|p| p.is_file())?;
                Some((parse_asy(&key, &decode(&std::fs::read(path).ok()?)), lent))
            });
            let Some((def, lent)) = found else {
                self.misses.insert(key);
                return None;
            };
            if lent {
                self.local.lent.insert(key.clone());
            }
            self.defs.insert(key.clone(), def);
        }
        self.defs.get(&key)
    }
}

/// LTspice's `lib` folders (with sym, cmp, sub) on this machine.
pub fn ltspice_lib_dirs() -> Vec<std::path::PathBuf> {
    ltspice_sym_dirs().into_iter().filter_map(|d| d.parent().map(|p| p.to_path_buf())).collect()
}

fn ltspice_sym_dirs() -> Vec<std::path::PathBuf> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    [
        format!("{home}/Library/Application Support/LTspice/lib/sym"),
        format!("{home}/AppData/Local/LTspice/lib/sym"),
        format!("{home}/Documents/LTspiceXVII/lib/sym"),
    ]
    .into_iter()
    .map(Into::into)
    .filter(|p: &std::path::PathBuf| p.is_dir())
    .collect()
}

/// LTspice orientation: rotate by n*90° clockwise (screen), `M` additionally negates x afterwards.
pub fn xform(rot: &str, x: i32, y: i32) -> (i32, i32) {
    let (x, y) = match rot.get(1..).unwrap_or("0") {
        "90" => (-y, x),
        "180" => (-x, -y),
        "270" => (y, -x),
        _ => (x, y),
    };
    if rot.starts_with('M') { (-x, y) } else { (x, y) }
}

// ---------------------------------------------------------------- .asc I/O

macro_rules! examples {
    ($($n:literal),*) => { &[$(($n, include_str!(concat!("../../examples/", $n, ".asc")))),*] };
}
pub const EXAMPLES: &[(&str, &str)] =
    examples!("rc_transient", "rc_bode", "opamp_inverting", "ce_amplifier", "diode_sweep", "bjt_curves", "noise_rc", "phasor_rlc", "pz_sallen_key",
        "meas_rc_timing", "buck_converter", "rectifier_rms", "analysis_tabs", "zener_prefix", "step_rc", "laplace_lowpass", "monte_carlo");

/// LTspice writes ASCII, Latin-1 or UTF-16LE.
pub fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes); // UTF-8 BOM
    let b = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
    if b.len() > 1 && b[1] == 0 {
        let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&u);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

/// LTspice TEXT escapes: `\n` is a line break, `\\` a backslash (`.lib C:\\Users\\...`).
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match (c, it.clone().next()) {
            ('\\', Some('n')) => (it.next(), out.push('\n')).1,
            ('\\', Some('\\')) => (it.next(), out.push('\\')).1,
            _ => out.push(c),
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

/// Bytes as LTspice writes them: Latin-1 when every character fits, else UTF-16LE.
pub fn encode(s: &str) -> Vec<u8> {
    if s.chars().all(|c| (c as u32) < 256) {
        s.chars().map(|c| c as u8).collect()
    } else {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }
}

pub fn parse_asc(src: &str) -> Vec<Item> {
    let mut items = vec![];
    for line in src.lines() {
        let line = line.trim_end_matches('\r');
        let tok: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| tok.get(i).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let rest = |n: usize| line.trim_start().splitn(n + 1, ' ').nth(n).unwrap_or("").to_string();
        match tok.first().copied() {
            Some("WIRE") => items.push(Item::Wire { x1: num(1), y1: num(2), x2: num(3), y2: num(4) }),
            Some("FLAG") => items.push(Item::Flag { x: num(1), y: num(2), name: rest(3), extra: vec![] }),
            Some("SYMBOL") => items.push(Item::Sym {
                sym: tok.get(1).unwrap_or(&"").to_string(),
                x: num(2),
                y: num(3),
                rot: tok.get(4).unwrap_or(&"R0").to_string(),
                attrs: BTreeMap::new(),
                extra: vec![],
            }),
            Some("WINDOW") if matches!(items.last(), Some(Item::Sym { .. })) => {
                if let Some(Item::Sym { extra, .. }) = items.last_mut() {
                    extra.push(line.to_string());
                }
            }
            Some("IOPIN") if matches!(items.last(), Some(Item::Flag { .. })) => {
                if let Some(Item::Flag { extra, .. }) = items.last_mut() {
                    extra.push(line.to_string());
                }
            }
            Some("SYMATTR") => {
                if let Some(Item::Sym { attrs, .. }) = items.last_mut() {
                    attrs.insert(tok.get(1).unwrap_or(&"").to_string(), rest(2));
                }
            }
            Some("TEXT") => {
                let t = rest(5);
                let directive = t.starts_with('!');
                let text = unescape(t.strip_prefix(['!', ';']).unwrap_or(&t));
                let align = format!("{} {}", tok.get(3).unwrap_or(&"Left"), tok.get(4).unwrap_or(&"2"));
                items.push(Item::Text { x: num(1), y: num(2), text, directive, align });
            }
            // (all its numbers, or kept as written)
            Some("LINE" | "RECTANGLE" | "CIRCLE" | "ARC") => items.push(drawing(&tok).unwrap_or_else(|| Item::Raw { line: line.to_string() })),
            Some("Version") | None => {}
            _ => items.push(Item::Raw { line: line.to_string() }),
        }
    }
    items
}

/// A LINE/RECTANGLE/CIRCLE/ARC line as a drawing: its pen, all its points, an optional style.
fn drawing(tok: &[&str]) -> Option<Item> {
    let n = if tok[0] == "ARC" { 8 } else { 4 };
    tok.get(1).filter(|w| w.parse::<i32>().is_err())?; // (its pen first: without one the numbers would shift)
    let nums: Vec<i32> = tok.get(2..)?.iter().map(|t| t.parse().ok()).collect::<Option<_>>()?;
    (nums.len() == n || nums.len() == n + 1).then(|| Item::Draw { kind: tok[0].to_string(), width: tok[1].to_string(), p: nums[..n].to_vec(), style: nums.get(n).copied() })
}

/// A file's items: a schematic's, or a netlist's as a schematic of it (`netlist_items`), and
/// whether it was a netlist (the app saves that elsewhere, never over it). One place decides, for
/// the app and the MCP server alike.
pub fn load(path: &str, bytes: &[u8]) -> (Vec<Item>, bool) {
    let text = decode(bytes);
    let netlist = is_netlist(path);
    (if netlist { netlist_items(&text) } else if is_symbol(path) { asy_items(&text) } else { parse_asc(&text) }, netlist)
}

/// Whether `path` names a symbol (.asy): edited on the canvas as `asy_items`, saved by `write_asy`.
pub fn is_symbol(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".asy")
}

/// A symbol as items to edit: its shapes as drawings, its pins as labels (named as the pin; `extra`: the PIN line, its other
/// PINATTRs), its texts as comments; the rest (SymbolType, WINDOW, SYMATTR …) kept as it is.
pub fn asy_items(src: &str) -> Vec<Item> {
    let mut items = vec![];
    for line in src.lines() {
        let line = line.trim_end_matches('\r');
        let tok: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| tok.get(i).and_then(|s| s.parse::<i32>().ok());
        let rest = |n: usize| after_words(line, n).to_string();
        match tok.first().copied() {
            Some("LINE" | "RECTANGLE" | "CIRCLE" | "ARC") => items.push(drawing(&tok).unwrap_or_else(|| Item::Raw { line: line.to_string() })),
            // (its lines as words with single spaces: written back as read, found by their heads; a
            // pin of no position kept as written)
            Some("PIN") => match (num(1), num(2)) {
                (Some(x), Some(y)) => items.push(Item::Flag { x, y, name: String::new(), extra: vec![tok.join(" ")] }),
                _ => items.push(Item::Raw { line: line.to_string() }),
            },
            Some("PINATTR") if matches!(items.last(), Some(Item::Flag { .. })) => {
                if let Some(Item::Flag { name, extra, .. }) = items.last_mut() {
                    match tok.get(1) {
                        Some(&"PinName") => *name = rest(2).trim().to_string(), // (as parse_asy reads it)
                        _ => extra.push(tok.join(" ")),
                    }
                }
            }
            // (its text as written after the one space following its size: an indented note stays so)
            Some("TEXT") => items.push(Item::Text { x: num(1).unwrap_or(0), y: num(2).unwrap_or(0), text: unescape({
                let s = after_words(line, 4).trim_start_matches(|c: char| !c.is_whitespace());
                s.strip_prefix(|c: char| c.is_whitespace()).unwrap_or(s)
            }), directive: false, align: format!("{} {}", tok.get(3).unwrap_or(&"Left"), tok.get(4).unwrap_or(&"2")) }),
            Some("Version") | None => {}
            // (found by its head: SymbolType, SYMATTR its name and value one space apart)
            Some("SYMATTR") if tok.len() >= 2 => items.push(Item::Raw { line: format!("SYMATTR {} {}", tok[1], rest(2)).trim_end().to_string() }),
            Some("SymbolType") if tok.len() >= 2 => items.push(Item::Raw { line: tok.join(" ") }),
            Some("SymbolType") => {} // (of no type: written as CELL)
            _ => items.push(Item::Raw { line: line.trim().to_string() }),
        }
    }
    items
}

/// A drawing's line (LINE, RECTANGLE, CIRCLE, ARC: its pen, its points, its style if any).
fn draw_line(kind: &str, width: &str, p: &[i32], style: &Option<i32>) -> String {
    let ps: Vec<String> = p.iter().map(|v| v.to_string()).collect();
    format!("{kind} {width} {}{}\n", ps.join(" "), style.map_or(String::new(), |s| format!(" {s}")))
}

/// A symbol file of `asy_items`' items: its pins where they are now, numbered on (SpiceOrder)
/// where new; parts and directives, which a symbol has none of, left out.
pub fn write_asy(items: &[Item]) -> String {
    let kind = items.iter().find_map(|it| match it {
        Item::Raw { line } if line.starts_with("SymbolType ") => Some(line.as_str()),
        _ => None,
    });
    let mut out = format!("Version 4\n{}\n", kind.unwrap_or("SymbolType CELL"));
    for it in items {
        match it {
            Item::Wire { x1, y1, x2, y2 } => out += &format!("LINE Normal {x1} {y1} {x2} {y2}\n"),
            Item::Draw { kind, width, p, style } => out += &draw_line(kind, width, p, style),
            Item::Text { x, y, text, directive: false, align } => out += &format!("TEXT {x} {y} {} {}\n", if align.is_empty() { "Left 2" } else { align }, escape(text)),
            _ => {}
        }
    }
    for it in items {
        if let Item::Raw { line } = it {
            if !line.starts_with("SymbolType ") {
                out += &format!("{line}\n");
            }
        }
    }
    let order = |extra: &[String]| extra.iter().find_map(|l| l.strip_prefix("PINATTR SpiceOrder ")?.trim().parse::<u32>().ok());
    let mut next = items.iter().filter_map(|it| match it { Item::Flag { extra, .. } => order(extra), _ => None }).max().unwrap_or(0);
    let mut taken = HashSet::new();
    for it in items {
        let Item::Flag { x, y, name, extra } = it else { continue };
        // (where its name shows, as read; a new pin's not)
        let pin = extra.iter().find(|l| l.starts_with("PIN ")).map(|l| l.split_whitespace().skip(3).collect::<Vec<_>>().join(" ")).filter(|j| !j.is_empty());
        out += &format!("PIN {x} {y} {}\n", pin.as_deref().unwrap_or("NONE 8"));
        if !name.is_empty() {
            out += &format!("PINATTR PinName {name}\n");
        }
        extra.iter().filter(|l| l.starts_with("PINATTR ") && !l.starts_with("PINATTR SpiceOrder ")).for_each(|l| out += &format!("{l}\n"));
        // (its number as read; a new pin's, or a copy's of another's, the next free one)
        let n = order(extra).filter(|n| taken.insert(*n)).unwrap_or_else(|| {
            next += 1;
            taken.insert(next);
            next
        });
        out += &format!("PINATTR SpiceOrder {n}\n");
    }
    out
}

/// Whether `path` names a SPICE netlist (opened as a schematic of it, see `netlist_items`).
pub fn is_netlist(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [".cir", ".net", ".sp", ".spi", ".ckt"].iter().any(|e| p.ends_with(e))
}

/// A SPICE netlist as a schematic: its title as a comment, the rest (up to `.end`) as one SPICE
/// directive, which a run takes as it is (parts, models, analyses alike), so it simulates and its
/// nodes can be plotted by name; nothing is drawn of it.
pub fn netlist_items(text: &str) -> Vec<Item> {
    let lines: Vec<&str> = text.lines().collect();
    // the first line a title, as SPICE reads a deck; a library's first line (`.subckt …`) not
    let titled = lines.first().is_some_and(|l| !l.trim_start().starts_with('.'));
    let title = if titled { lines[0].trim().trim_start_matches('*').trim().to_string() } else { String::new() };
    // up to `.end`; a `.control` block (not run: commands can do anything) left out, its analyses
    // (`tran 1n 1u`) taken as the dot commands they stand for
    // (ngspice's commands that are analyses, and what dot command each is: fourier is .four)
    const ANALYSES: [(&str, &str); 12] = [("tran", ".tran"), ("ac", ".ac"), ("dc", ".dc"), ("op", ".op"), ("noise", ".noise"), ("tf", ".tf"), ("pz", ".pz"), ("sens", ".sens"), ("fourier", ".four"), ("disto", ".disto"), ("pss", ".pss"), ("sp", ".sp")];
    let (mut body, mut control, mut taken, mut dropped) = (vec![], false, vec![], 0);
    let first = |l: &str| l.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    for l in lines.iter().skip(usize::from(titled)).take_while(|l| !is_end(&l.trim().to_ascii_lowercase())) {
        let (low, w) = (l.trim().to_ascii_lowercase(), first(l));
        match w.as_str() {
            ".control" => control = true,
            ".endc" => control = false,
            // (an analysis of the control language's variables, `tran 1n $&tstop`: no dot command, left
            // out; a `$ comment` after it is no variable)
            w if control && !uncommented(l).contains(['$', '&']) && ANALYSES.iter().any(|(c, _)| *c == w) => {
                let dot = ANALYSES.iter().find(|(c, _)| *c == w).unwrap().1;
                let line = format!("{dot}{}", &uncommented(l).trim()[w.len()..]);
                if !taken.contains(&line) {
                    taken.push(line); // (an analysis repeated in the block, `alter` between: once)
                }
            }
            // (a comment line in it: no command)
            _ if control => dropped += usize::from(!low.is_empty() && !low.starts_with('*')),
            _ => body.push(l.to_string()),
        }
    }
    // (an analysis the deck has as a dot command already: not twice; one of other values: said)
    let (twice, kept): (Vec<String>, Vec<String>) = taken.into_iter().partition(|t| body.iter().any(|b| first(b) == first(t)));
    let taken = kept;
    let replaced: Vec<String> = twice.into_iter().filter(|t| !body.iter().any(|b| b.trim().eq_ignore_ascii_case(t))).collect();
    body.extend(taken.iter().cloned());
    let body = body.join("\n").trim_matches('\n').to_string();
    let mut items = vec![];
    // (named as a netlist's title: a title `.tran 10m` is no commented-out analysis)
    let mut note = if title.is_empty() { title } else { format!("Netlist: {title}") };
    if !taken.is_empty() || dropped > 0 || !replaced.is_empty() {
        let also = if replaced.is_empty() { String::new() } else { format!(", {} not (the deck has its own)", replaced.join(" ")) };
        note += &format!("{}(its .control block is not run: {}{} other command{} left out{also})", if note.is_empty() { "" } else { "\n" }, if taken.is_empty() { String::new() } else { format!("{} taken as dot commands, ", taken.join(" ")) }, dropped, if dropped == 1 { "" } else { "s" });
    }
    if !note.is_empty() {
        items.push(Item::Text { x: 0, y: -48 - 17 * (note.lines().count() as i32 - 1), text: note, directive: false, align: "Left 2".into() });
    }
    if !body.trim().is_empty() {
        items.push(Item::Text { x: 0, y: 0, text: body, directive: true, align: "Left 2".into() });
    }
    items
}

pub fn write_asc(items: &[Item]) -> String {
    let sheet = items.iter().find_map(|it| match it {
        Item::Raw { line } if line.starts_with("SHEET ") => Some(line.as_str()),
        _ => None,
    });
    let mut out = format!("Version 4\n{}\n", sheet.unwrap_or("SHEET 1 880 680"));
    let mut by_kind = |want: u8| {
        for it in items {
            match (want, it) {
                (0, Item::Wire { x1, y1, x2, y2 }) => out += &format!("WIRE {x1} {y1} {x2} {y2}\n"),
                (1, Item::Flag { x, y, name, extra }) => {
                    out += &format!("FLAG {x} {y} {name}\n");
                    // (a port's IOPIN where its label is now: moved since it was read; a symbol pin's
                    // lines, pasted from one, are no schematic's)
                    for l in extra.iter().filter(|l| !l.starts_with("PIN ") && !l.starts_with("PINATTR ")) {
                        out += &port_kind(l).map_or(format!("{l}\n"), |kind| format!("IOPIN {x} {y} {kind}\n"));
                    }
                }
                (2, Item::Sym { sym, x, y, rot, attrs, extra }) => {
                    out += &format!("SYMBOL {sym} {x} {y} {rot}\n");
                    extra.iter().for_each(|l| out += &format!("{l}\n"));
                    let inst = attrs.get("InstName").into_iter().map(|v| ("InstName", v));
                    for (k, v) in inst.chain(attrs.iter().filter(|(k, _)| *k != "InstName").map(|(k, v)| (k.as_str(), v))) {
                        out += &format!("SYMATTR {k} {v}\n");
                    }
                }
                (3, Item::Text { x, y, text, directive, align }) => {
                    let mark = if *directive { '!' } else { ';' };
                    let align = if align.is_empty() { "Left 2" } else { align };
                    out += &format!("TEXT {x} {y} {align} {mark}{}\n", escape(text));
                }
                // (drawings and the other kept lines in one pass: in their order in the file)
                (4, Item::Draw { kind, width, p, style }) => out += &draw_line(kind, width, p, style),
                (4, Item::Raw { line }) if !line.starts_with("SHEET ") => out += &format!("{line}\n"),
                _ => {}
            }
        }
    };
    (0..5).for_each(&mut by_kind);
    out
}

/// A label's port kind (In, Out, BiDir) from its IOPIN line.
fn port_kind(line: &str) -> Option<&str> {
    let t: Vec<&str> = line.split_whitespace().collect();
    (t.len() >= 4 && t[0] == "IOPIN").then(|| t[3])
}

/// A block symbol for a sheet (LTspice's SymbolType BLOCK): a box with its ports as pins, inputs
/// (and bidirectional ones) on the left, outputs on the right, each once, in the sheet's order.
pub fn block_asy(items: &[Item]) -> Result<String, String> {
    let (mut left, mut right): (Vec<&str>, Vec<&str>) = (vec![], vec![]);
    let mut seen = HashSet::new();
    for it in items {
        let Item::Flag { name, extra, .. } = it else { continue };
        let Some(kind) = extra.iter().find_map(|l| port_kind(l)) else { continue };
        if name == "0" || name.trim().eq_ignore_ascii_case("gnd") || !seen.insert(name.trim().to_lowercase()) {
            continue;
        }
        if kind.eq_ignore_ascii_case("out") { right.push(name.trim()) } else { left.push(name.trim()) }
    }
    if seen.is_empty() {
        return Err("the schematic has no ports: give the labels its block connects through a port type (In, Out, BiDir) in their dialog".into());
    }
    let longest = |side: &[&str]| side.iter().map(|n| n.chars().count() as i32).max().unwrap_or(0);
    // (names inside, 8 apart from their pins; pins on the 16 grid)
    let w = (((longest(&left) + longest(&right)) * 8 + 48 + 31) / 32 * 32).max(96);
    let rows = left.len().max(right.len()) as i32;
    let top = -16 * (rows - 1);
    let mut out = format!("Version 4\nSymbolType BLOCK\nRECTANGLE Normal {} {} {} {}\nWINDOW 0 {} {} Left 2\nWINDOW 3 {} {} Left 2\n",
        -w / 2, top - 32, w / 2, -top + 32, -w / 2, top - 48, -w / 2, -top + 48);
    let mut order = 0;
    for (side, x, just) in [(&left, -w / 2, "LEFT"), (&right, w / 2, "RIGHT")] {
        for (k, name) in side.iter().enumerate() {
            order += 1;
            out += &format!("PIN {x} {} {just} 8\nPINATTR PinName {name}\nPINATTR SpiceOrder {order}\n", top + 32 * k as i32);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- netlisting

#[derive(Serialize, Debug, Default)]
pub struct Netlist {
    pub text: String,
    /// Aligned with items: wire/flag -> [net], sym -> pin nets (SpiceOrder), text -> [].
    pub nets: Vec<Vec<String>>,
    /// Aligned with items: SPICE instance name of each symbol ("" otherwise).
    pub insts: Vec<String>,
    /// Aligned with items: ngspice expression for the current into pin 1 ("" if none), valid for
    /// the simulated netlist (sensors, see `netlist_for_sim`).
    pub probes: Vec<String>,
    /// Aligned with items, per pin (SpiceOrder): the name of the current into it where it has a
    /// sensor (`I(R1)`, `Ib(Q1)`, `Ix(U1:OUT)`), else "". Trace expressions may use these names.
    pub pin_currents: Vec<Vec<String>>,
    /// Aligned with items: a block's schematic (the path of the sheet it runs as), else "".
    pub sheets: Vec<String>,
    /// Sensors of the simulated netlist (names are reserved in both builds, so they match).
    #[serde(skip)]
    pub sensors: Vec<Sensor>,
    /// The `.step` statements (`+` lines joined): ngspice has none, the run steps (see `Prepared::steps`).
    #[serde(skip)]
    pub steps: Vec<String>,
    pub warnings: Vec<String>,
}

/// The hierarchical blocks of one netlisting: the sheets being netlisted (outermost first), and
/// the subcircuits done, each once (the top schematic's netlist writes them all, none nested).
#[derive(Default, Clone)]
struct Hier {
    /// (subcircuit name, the symbol's when its refusal is said: a BLOCK symbol's)
    stack: Vec<(String, Option<String>)>,
    blocks: Vec<Block>,
    /// symbols whose sheet is no subcircuit here (it places itself, it lacks a port), and why:
    /// their parts call a model of that name, as before there was a sheet
    refused: BTreeMap<String, String>,
    /// the sheets' `.global` lines: the top netlist's (their includes stay in their subcircuits,
    /// inlined there: what a file holds is that block's)
    hoisted: Vec<String>,
    /// some sheet has a source with an AC amplitude, a ground
    ac: bool,
    grounded: bool,
    /// the top schematic's .model and .subckt names, whether it includes files: its sheets' too
    top_defs: HashSet<String>,
    top_includes: bool,
    /// names of refused BLOCK symbols' calls: their refusal is said, not a missing model
    quiet: HashSet<String>,
    /// each block symbol's subcircuit name, unique (two symbols whose names read alike as SPICE
    /// names get two)
    names: HashMap<String, String>,
    /// the library definitions the sheets need, each once: the top netlist's too (a sheet's own
    /// includes, in its subcircuit, win there), and the names they define
    models: Vec<String>,
    emitted: HashSet<String>,

}

impl Hier {
    /// A block symbol's subcircuit name: `sheet_<name>` (a sheet wrapping a library part of its
    /// name calls that one, not itself), another for another symbol of the same SPICE name.
    fn name(&mut self, sym: &str) -> String {
        if let Some(n) = self.names.get(sym) {
            return n.clone();
        }
        let base = format!("sheet_{}", net_name(sym));
        // (nor a .subckt of the top schematic's own: a call would find that one. ponytail: a library's
        // of that name is not looked for; `sheet_` names are nomospice's)
        let name = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base}_{k}") }).find(|n| !self.names.values().any(|v| v.eq_ignore_ascii_case(n)) && !self.top_defs.contains(&n.to_lowercase())).unwrap();
        self.names.insert(sym.to_string(), name.clone());
        name
    }
}

/// A hierarchical block's sheet as a subcircuit.
#[derive(Debug, Default, Clone)]
struct Block {
    /// the symbol's name, as SPICE takes it
    name: String,
    /// the sheet's nets of the symbol's pins (SpiceOrder)
    ports: Vec<String>,
    /// its parts and definitions
    body: Vec<String>,
    /// parameters its instances pass (lowercase): declared on the .subckt line, so ngspice takes them
    params: std::collections::BTreeSet<String>,
    /// the sheet's own .params (lowercase): their default where an instance passes none
    own: std::collections::BTreeSet<String>,
}

/// Marks a block's instance line until the top netlist is done: `\x01name\x01passed params`.
const BLOCK_TAG: char = '\x01';

struct Dsu(Vec<usize>);
impl Dsu {
    fn find(&mut self, i: usize) -> usize {
        let p = self.0[i];
        if p == i { i } else { let r = self.find(p); self.0[i] = r; r }
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.0[a] = b;
    }
}

/// p strictly inside segment a-b (T-junction). Crossing wires without a vertex do not connect.
fn inside(p: (i32, i32), a: (i32, i32), b: (i32, i32)) -> bool {
    let (px, py, ax, ay, bx, by) = (p.0 as i64, p.1 as i64, a.0 as i64, a.1 as i64, b.0 as i64, b.1 as i64);
    let cross = (bx - ax) * (py - ay) - (by - ay) * (px - ax);
    let dot = (px - ax) * (bx - ax) + (py - ay) * (by - ay);
    cross == 0 && dot > 0 && dot < (bx - ax).pow(2) + (by - ay).pow(2)
}

/// Whether `name` is one of nomospice's own symbols (they win over an .asy of the same name).
pub fn builtin_symbol(name: &str) -> bool {
    let key = sym_key(name);
    BUILTIN_SYMBOLS.iter().any(|(n, _)| *n == key)
}

/// The default models the component library lists ([`BUILTIN_MODELS`] with a name of their own
/// and parameters: not the bare `.model D D` type defaults or the opamp symbol's subckt).
pub fn builtin_cards() -> &'static str {
    static CARDS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        BUILTIN_MODELS.iter().map(|(_, t)| *t).filter(|t| t.starts_with(".model") && t.contains('(')).collect::<Vec<_>>().join("\n") + "\n"
    });
    &CARDS
}

const BUILTIN_MODELS: &[(&str, &str)] = &[
    ("d", ".model D D"),
    ("npn", ".model NPN NPN"),
    ("pnp", ".model PNP PNP"),
    ("nmos", ".model NMOS NMOS"),
    ("pmos", ".model PMOS PMOS"),
    ("njf", ".model NJF NJF"),
    ("pjf", ".model PJF PJF"),
    // switches: closed above 0.5 V / 1 mA (SPICE's own default threshold 0 leaves 0 V ambiguous)
    ("sw", ".model SW SW(Ron=1 Roff=1Meg Vt=0.5)"),
    ("csw", ".model CSW CSW(Ron=1 Roff=1Meg It=1m)"),
    // generic defaults, own estimates from public datasheet values (V_F at 1/10 mA, h_FE, f_T,
    // C_j, t_rr): no vendor or LTspice library card. Installed libraries take precedence.
    ("1n4148", ".model 1N4148 D(IS=2n N=1.8 RS=0.5 CJO=2p VJ=0.7 M=0.33 TT=3n BV=100 IBV=1u)"),
    ("2n3904", ".model 2N3904 NPN(IS=12f BF=180 VAF=80 IKF=0.15 BR=2 RB=10 RC=0.5 RE=0.3 CJE=6p VJE=0.75 MJE=0.35 CJC=3p VJC=0.75 MJC=0.33 TF=0.45n TR=150n XTB=1.5)"),
    ("2n3906", ".model 2N3906 PNP(IS=12f BF=150 VAF=60 IKF=0.12 BR=2 RB=10 RC=0.6 RE=0.3 CJE=8p VJE=0.75 MJE=0.35 CJC=4p VJC=0.75 MJC=0.33 TF=0.6n TR=200n XTB=1.5)"),
    ("opamp", ".subckt opamp inn inp out params: Aol=100K GBW=10Meg\nG1 0 n inp inn 1m\nR1 n 0 {Aol*1k}\nC1 n 0 {1/(6.2832*1k*GBW)}\nE1 out 0 n 0 1\n.ends opamp"),
    // with supplies (LTspice's opamp2 pins): a transconductance into R‖C around mid-supply (the
    // dominant pole; GBW = gm/2πC), its current limited (slew rate = Imax/C), that node kept within
    // the supplies (no wind-up), the output limited to the supplies less Vsat
    ("opamp2", ".subckt opamp2 inp inn vp vn out params: Aol=100K GBW=10Meg Slew=10Meg Vsat=0.1 Rout=1\n.param Cc=1n gm={6.2832*GBW*Cc} Imax={Slew*Cc}\nBm mid 0 V=(V(vp)+V(vn))/2\nB1 0 x I=max(min({gm}*V(inp,inn), {Imax}), -{Imax})\nR1 x mid {Aol/gm}\nC1 x mid {Cc}\nB2 x 0 I=V(x)>V(vp) ? V(x)-V(vp) : (V(x)<V(vn) ? V(x)-V(vn) : 0)\nB3 o 0 V=max(min(V(x), V(vp)-{Vsat}), V(vn)+{Vsat})\nR2 o out {Rout}\nR3 vp vn 1Meg\n.ends opamp2"),
    // 10 m of 50 Ω coax (RG58-like: 250 nH/m, 100 pF/m, some loss)
    ("ltra", ".model LTRA LTRA(R=0.1 L=250n G=0 C=100p LEN=10)"),
    // logic gates and a D flip-flop of behavioral sources (levels Vlow/Vhigh, threshold halfway;
    // the output through an RC of time constant Td); an open input reads low
    ("and2", ".subckt and2 a b y params: Vhigh=5 Vlow=0 Td=10n\nB1 o 0 V=(V(a)>{(Vhigh+Vlow)/2} && V(b)>{(Vhigh+Vlow)/2}) ? {Vhigh} : {Vlow}\nR1 o y 10\nC1 y 0 {Td/10}\nRa a 0 1G\nRb b 0 1G\n.ends and2"),
    ("nand2", ".subckt nand2 a b y params: Vhigh=5 Vlow=0 Td=10n\nB1 o 0 V=(V(a)>{(Vhigh+Vlow)/2} && V(b)>{(Vhigh+Vlow)/2}) ? {Vlow} : {Vhigh}\nR1 o y 10\nC1 y 0 {Td/10}\nRa a 0 1G\nRb b 0 1G\n.ends nand2"),
    ("or2", ".subckt or2 a b y params: Vhigh=5 Vlow=0 Td=10n\nB1 o 0 V=(V(a)>{(Vhigh+Vlow)/2} || V(b)>{(Vhigh+Vlow)/2}) ? {Vhigh} : {Vlow}\nR1 o y 10\nC1 y 0 {Td/10}\nRa a 0 1G\nRb b 0 1G\n.ends or2"),
    ("nor2", ".subckt nor2 a b y params: Vhigh=5 Vlow=0 Td=10n\nB1 o 0 V=(V(a)>{(Vhigh+Vlow)/2} || V(b)>{(Vhigh+Vlow)/2}) ? {Vlow} : {Vhigh}\nR1 o y 10\nC1 y 0 {Td/10}\nRa a 0 1G\nRb b 0 1G\n.ends nor2"),
    ("xor2", ".subckt xor2 a b y params: Vhigh=5 Vlow=0 Td=10n\nB1 o 0 V=(V(a)>{(Vhigh+Vlow)/2}) ? ((V(b)>{(Vhigh+Vlow)/2}) ? {Vlow} : {Vhigh}) : ((V(b)>{(Vhigh+Vlow)/2}) ? {Vhigh} : {Vlow})\nR1 o y 10\nC1 y 0 {Td/10}\nRa a 0 1G\nRb b 0 1G\n.ends xor2"),
    ("not", ".subckt not a y params: Vhigh=5 Vlow=0 Td=10n\nB1 o 0 V=V(a)>{(Vhigh+Vlow)/2} ? {Vlow} : {Vhigh}\nR1 o y 10\nC1 y 0 {Td/10}\nRa a 0 1G\n.ends not"),
    ("dff", ".subckt dff d c q qn params: Vhigh=5 Vlow=0 Td=10n\nB1 mi 0 V=V(c)<{(Vhigh+Vlow)/2} ? V(d) : V(m)\nR1 mi m 10\nC1 m 0 {Td/10}\nRm m 0 1G\nB2 si 0 V=V(c)>{(Vhigh+Vlow)/2} ? V(m) : V(s)\nR2 si s 10\nC2 s 0 {Td/10}\nRs s 0 1G\nB3 qo 0 V=V(s)>{(Vhigh+Vlow)/2} ? {Vhigh} : {Vlow}\nR3 qo q 10\nC3 q 0 {Td/10}\nB4 qno 0 V=V(s)>{(Vhigh+Vlow)/2} ? {Vlow} : {Vhigh}\nR4 qno qn 10\nC4 qn 0 {Td/10}\nRd d 0 1G\nRc c 0 1G\n.ends dff"),
];

/// Whether `m` is the subcircuit of one of nomospice's own symbols (opamp, opamp2, and2 …): its
/// model when placed by that symbol.
fn own_model(m: &str) -> bool {
    BUILTIN_SYMBOLS.iter().any(|(n, _)| *n == m) && BUILTIN_MODELS.iter().any(|(n, t)| *n == m && t.starts_with(".subckt"))
}

/// Byte spans of `line`'s [`spice_tokens`] (they are slices of it).
fn token_spans(line: &str) -> Vec<(usize, usize)> {
    spice_tokens(line).into_iter().map(|t| { let a = t.as_ptr() as usize - line.as_ptr() as usize; (a, a + t.len()) }).collect()
}

/// One line of a deck as sweeps read it.
struct DeckLine<'a> {
    start: usize,
    line: &'a str,
    /// ASCII lowercase: the same byte offsets as `line`
    low: String,
    /// outside `.subckt … .ends`
    top: bool,
    /// a comment (see [`comment_line`]) or blank line (SPICE skips these, also between a line and its `+` continuations)
    skip: bool,
    /// lowercase first token of the statement this line belongs to (`.param` for its `+` lines too)
    head: String,
}

impl DeckLine<'_> {
    fn param(&self) -> bool {
        !self.skip && self.head == ".param"
    }
}

fn deck_lines(text: &str) -> Vec<DeckLine<'_>> {
    let (mut sub, mut head, mut at) = (0, String::new(), 0);
    text.split_inclusive('\n').map(|piece| {
        let (start, line) = (at, piece.trim_end_matches(['\n', '\r']));
        at += piece.len();
        let low = line.to_ascii_lowercase();
        let first = low.split_whitespace().next().unwrap_or("").to_string();
        match first.as_str() { ".subckt" => sub += 1, ".ends" => sub = (sub - 1).max(0), _ => {} } // a stray .ends must not hide the rest
        let skip = first.is_empty() || comment_line(&first);
        if !skip && !first.starts_with('+') {
            head = first.clone();
        }
        DeckLine { start, line, low, top: sub == 0 && first != ".ends", skip, head: head.clone() }
    }).collect()
}

/// The definitions on a `.param` line: (name as written, value span). Whole top-level tokens only,
/// so `{x==1?2:3}` (one token) and `µa=1` (another name) define neither x nor a, and nothing after
/// an inline `;`/`$` comment counts. Forms: `n=v`, `n= v`, `n =v`, `n = v`, `+n=v`.
fn param_defs<'a>(line: &'a str, low: &str) -> Vec<(&'a str, (usize, usize))> {
    let ident = |n: &str| n.bytes().next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') && n.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    let sp = token_spans(line);
    let mut out = vec![];
    for (i, &(a0, e)) in sp.iter().enumerate() {
        if low[a0..].starts_with(';') || low[a0..].starts_with('$') {
            break;
        }
        let a = if low[a0..e].starts_with('+') && e > a0 + 1 { a0 + 1 } else { a0 }; // `+Rb=10k`
        let tok = &low[a..e];
        let next = |k: usize| sp.get(k).copied();
        let def = match tok.find('=') {
            Some(k) if k > 0 && ident(&tok[..k]) && !tok[k..].starts_with("==") => {
                Some((&line[a..a + k], if tok.len() > k + 1 { Some((a + k + 1, e)) } else { next(i + 1) }))
            }
            None if ident(tok) => next(i + 1).filter(|&(na, ne)| low[na..ne].starts_with('=') && !low[na..ne].starts_with("==")).map(|(na, ne)| {
                (&line[a..e], if ne > na + 1 { Some((na + 1, ne)) } else { next(i + 2) })
            }),
            _ => None,
        };
        if let Some((name, Some(v))) = def {
            out.push((name, v));
        }
    }
    out
}

/// The value of an R, C or L line (the token after its two nodes) if a number can stand in for it:
/// a number or a `{…}`/`'…'` expression, not a model name or an `R=…` form.
fn part_value(line: &str) -> Option<(usize, usize)> {
    if !matches!(line.trim_start().bytes().next()?.to_ascii_lowercase(), b'r' | b'c' | b'l') {
        return None;
    }
    let (a, e) = *token_spans(line).get(3)?;
    let v = &line[a..e];
    let plain = v.trim_start_matches(['+', '-']).starts_with(|c: char| c.is_ascii_digit() || c == '.') && !v.contains('=');
    (plain || (v.starts_with('{') && v.ends_with('}')) || (v.len() > 1 && v.starts_with('\'') && v.ends_with('\''))).then_some((a, e))
}

/// Where `target` is set in `text` (byte spans): every top-level `.param` definition of that name
/// (SPICE uses the last), or the value of the top-level R, C or L instance of that name
/// ([`part_value`]). `.subckt` bodies are left alone. None if neither is in the deck. Both there:
/// the .param if `param_first` (the user's own deck defines it: pz_sallen_key's `.param Rb` with
/// part Rb = {Rb}), else the part (an included library's .param that happens to share its name).
pub fn vary_spans(text: &str, target: &str, param_first: bool) -> Option<Vec<(usize, usize)>> {
    let t = target.trim();
    if t.is_empty() || !t.is_ascii() {
        return None;
    }
    let lines = deck_lines(text);
    let defs = param_spans(text, t);
    let low = t.to_ascii_lowercase();
    let part = lines.iter().filter(|l| l.top).find(|l| l.low.split_whitespace().next() == Some(low.as_str()))
        .and_then(|l| part_value(l.line).map(|(a, e)| vec![(l.start + a, l.start + e)]));
    if !defs.is_empty() && (param_first || part.is_none()) { Some(defs) } else { part }
}

/// What a sweep can vary in `text` (vary_spans' rules), for the locus panel's list: top-level
/// `.param` names and the R, C, L instances among `placed` (the schematic's parts, not the
/// netlister's parasitics such as `RC1_rser`).
pub fn sweepable(text: &str, placed: &[String]) -> Vec<String> {
    let lines = deck_lines(text);
    let params = lines.iter().filter(|l| l.top && l.param()).flat_map(|l| param_defs(l.line, &l.low).into_iter().map(|(n, _)| n.to_string()))
        .filter(|n| !n.eq_ignore_ascii_case("pi")); // the netlister's built-in constant
    let parts = lines.iter().filter(|l| l.top && part_value(l.line).is_some()).filter_map(|l| l.line.split_whitespace().next())
        .filter(|n| placed.iter().any(|p| p.eq_ignore_ascii_case(n))).map(str::to_string);
    let mut out: Vec<String> = vec![];
    for n in params.chain(parts) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
            out.push(n);
        }
    }
    out
}

/// The first `.pz` statement in `text`, lowercase, its `+` continuation lines joined (comment
/// lines between them skipped).
pub fn pz_statement(text: &str) -> Option<String> {
    statements(text, &[".pz"]).into_iter().next().map(|s| s.to_ascii_lowercase()) // (inline comments cut)
}

/// `text` with every span replaced by `value` (the rest byte for byte as it was).
pub fn splice(text: &str, spans: &[(usize, usize)], value: f64) -> String {
    let (v, mut out, mut at) = (format!("{value:e}"), String::with_capacity(text.len()), 0);
    for &(a, e) in spans {
        if a < at {
            continue; // inside the span before (`.param a= a=1`: the first value is `a=1`)
        }
        out.push_str(&text[at..a]);
        out.push_str(&v);
        at = e;
    }
    out.push_str(&text[at..]);
    out
}

/// `text` with `target` set to `value` (see [`vary_spans`]).
#[cfg(test)]
pub fn vary(text: &str, target: &str, value: f64) -> Option<String> {
    Some(splice(text, &vary_spans(text, target, true)?, value))
}

/// SPICE number with suffix (f p n u µ m k meg g t), trailing units ignored.
pub fn spice_num(s: &str) -> Option<f64> {
    let s = s.to_lowercase().replace(['µ', 'μ'], "u"); // micro sign and Greek mu
    let end = s.find(|c: char| !(c.is_ascii_digit() || "+-.e".contains(c))).unwrap_or(s.len());
    // "1e" would swallow a unit; back off until it parses
    let (mut num, mut tail) = (&s[..end], &s[end..]);
    while num.parse::<f64>().is_err() && !num.is_empty() {
        (num, tail) = (&s[..num.len() - 1], &s[num.len() - 1..]);
    }
    let mult = if tail.starts_with("meg") { 1e6 } else {
        match tail.chars().next() {
            Some('f') => 1e-15, Some('p') => 1e-12, Some('n') => 1e-9, Some('u') => 1e-6,
            Some('m') => 1e-3, Some('k') => 1e3, Some('g') => 1e9, Some('t') => 1e12, _ => 1.0,
        }
    };
    Some(num.parse::<f64>().ok()? * mult)
}

/// Node and instance names: characters that would break a SPICE line or an ngspice command
/// (whitespace, `()=,;{}'"`, backquotes, `$`, non-ASCII ...) become `_`. `+ - .` stay, so labels
/// like `Vout+` keep matching the user's own directives (`V(Vout+)`); probes quote names anyway.
pub fn net_name(label: &str) -> String {
    let n: String = label.trim().chars().map(|c| match c {
        c if c.is_ascii_alphanumeric() || "_+-.".contains(c) => c,
        _ => '_',
    }).collect();
    // a node called `time`/`frequency` would shadow the plot's scale vector
    if ["time", "frequency"].contains(&n.to_lowercase().as_str()) { n + "_" } else { n }
}

/// Whitespace tokens, but `{a * b}`, `f(a, b)`, `'a b'` and `"a b"` stay one token.
pub fn spice_tokens(s: &str) -> Vec<&str> {
    let (mut out, mut start, mut depth, mut quote) = (vec![], None, 0, None::<char>);
    for (i, c) in s.char_indices() {
        match c {
            '{' | '(' if quote.is_none() => depth += 1,
            '}' | ')' if quote.is_none() => depth -= 1,
            '\'' | '"' if quote.is_none() => quote = Some(c),
            c if quote == Some(c) => quote = None,
            c if c.is_whitespace() && depth <= 0 && quote.is_none() => {
                if let Some(b) = start.take() {
                    out.push(&s[b..i]);
                }
                continue;
            }
            _ => {}
        }
        start.get_or_insert(i);
    }
    out.extend(start.map(|b| &s[b..]));
    out
}

/// How a `.net` is kept in the netlist (a comment the plot reads; a `* .net` of the user's is none).
const NET_MARK: &str = "; nomospice: two-port";

/// `.tran … startup` leaves this line after its `.tran … uic`: `prepare` then ramps the deck's DC
/// sources up (see [`ramped`]).
const STARTUP: &str = "* startup: the DC sources ramped up from 0 over 20 us, no operating point (LTspice's .tran startup)"; // (ASCII: the deck is)

/// An independent source's line `V1 a b 5` (or `DC 5`, an AC part after it) as one that ramps up
/// from 0 over the first 20 µs, as LTspice's `.tran startup` does; None: not such a source (a
/// function such as SINE keeps its own start).
fn ramped(line: &str) -> Option<String> {
    let code = uncommented(line);
    let t = spice_tokens(code);
    if !t.first().is_some_and(|n| n.starts_with(['V', 'v', 'I', 'i'])) || t.len() < 4 {
        return None;
    }
    // (the DC value right after the nodes, or after a `DC` anywhere: `AC 1 DC 5`, as .step finds it)
    let after = &t[3..];
    let (v, rest): (&str, Vec<&str>) = match after.iter().position(|x| x.eq_ignore_ascii_case("dc")) {
        Some(k) => (after.get(k + 1)?, after[..k].iter().chain(&after[k + 2..]).copied().collect()),
        None => (after[0], after[1..].to_vec()),
    };
    // (0 too: a .step of it steps the ramp's end; a 0 V sensor stays 0)
    let value = spice_num(v).is_some() || (v.starts_with('{') && v.ends_with('}'));
    // (a comment after it kept)
    // (after the value: nothing, AC and its numbers, k=v parameters (Rser=…); a SINE there would be
    // a second waveform)
    let mut ac_only = true;
    let mut i = 0;
    while i < rest.len() {
        if rest[i].eq_ignore_ascii_case("ac") {
            i += 1;
            while i < rest.len() && (spice_num(rest[i]).is_some() || rest[i].starts_with('{')) {
                i += 1;
            }
        } else if rest[i].contains('=') {
            i += 1;
        } else {
            ac_only = false;
            break;
        }
    }
    (value && ac_only).then(|| format!("{} {} {} {RAMP}{v}){}{}", t[0], t[1], t[2], rest.iter().map(|r| format!(" {r}")).collect::<String>(), &line[code.trim_end().len()..]))
}
/// How [`ramped`] writes a source's value, `PWL(0 0 20e-6 5)` (a `.step` of the source steps the
/// 5; written so, not `20u`, a PWL of the user's own is not taken for one).
const RAMP: &str = "PWL(0 0 20e-6 ";
/// The sources `.tran startup` ramped, as the mark names them (lowercase).
fn ramped_sources(text: &str) -> HashSet<String> {
    text.lines().find(|l| l.starts_with(STARTUP)).and_then(|l| l.split_once("; ramped:")).map_or_else(HashSet::new, |(_, n)| n.split_whitespace().map(str::to_string).collect())
}

/// The value a ramp token ends at, `5` of `PWL(0 0 20e-6 5)`: one value, a braced expression or a
/// number (a PWL of the user's own that starts alike, `PWL(0 0 20e-6 5 1m 3)`, is none).
fn ramp_end(token: &str) -> Option<&str> {
    // (one brace group, `{max(a, b)}`; `{a} 1m {b}` is a PWL of the user's own)
    let braced = |v: &str| v.len() > 1 && v.starts_with('{') && v.ends_with('}') && !v[1..v.len() - 1].contains(['{', '}']);
    token.strip_prefix(RAMP).and_then(|r| r.strip_suffix(')')).filter(|v| braced(v) || !v.contains([' ', '(']))
}

/// A comment line of LTspice's (`;`) or ngspice's (`#`, `$`, `//`) as SPICE's `* …`: a + line after it
/// then continues the statement before, as LTspice and ngspice read it (ngspice would join one to a
/// `;` line), and every pass here reads it alike; `* `, not `*`: ngspice makes a `#` a `*`, and
/// `*#` is a command line. None: no such line.
fn starred(line: &str) -> Option<String> {
    comment_rest(line).map(|r| format!("* {}", r.trim()))
}
/// A line as libngspice takes it: it rejects the whole deck on non-ASCII bytes (`10µ`, `4.7μF`,
/// `10kΩ`): Ω (Greek or the Ohm sign) dropped, `µ` a `u`, other blanks a blank, the rest `_`. The
/// expander's checks read this line, so an Ω before a `.control` hides nothing.
fn ascii(line: &str) -> std::borrow::Cow<'_, str> {
    if line.is_ascii() {
        return line.into();
    }
    line.chars().filter(|c| !matches!(c, 'Ω' | '\u{2126}')).map(|c| match c {
        'µ' | 'μ' => 'u',
        c if c.is_ascii() => c,
        c if c.is_whitespace() => ' ',
        _ => '_',
    }).collect::<String>().into()
}

/// A line ngspice would take for commands: `.control`, `*#` (a line's `#` it makes a `*`), and the
/// title `*ng_script` (which `.title` sets), which makes it run the deck's lines as commands.
pub(crate) fn command_line(line: &str) -> bool {
    ["#", "*#", ".control", ".title", "*ng_script"].iter().any(|p| starts_ci(line, p))
}
/// Whether dropping command line `l` is said: not for a plain `.title` (of no use here), only for one
/// that would make the deck a script.
fn said(l: &str) -> bool {
    !starts_ci(l, ".title") || starts_ci(&l.trim_start()[".title".len()..], "*ng_script")
}
/// `.include`, `.inc`, `.lib`: a file ngspice would open itself (the expander inlines them).
pub(crate) fn opens_file(line: &str) -> bool {
    [".inc", ".lib"].iter().any(|p| starts_ci(line, p))
}
/// `.end`, a blank (a tab too), a comment or nothing after it: ngspice ends the circuit there.
fn is_end(low: &str) -> bool {
    uncommented(low).trim_end().strip_prefix(".end").is_some_and(|r| r.is_empty() || r.starts_with(char::is_whitespace))
}
fn starts_ci(line: &str, p: &str) -> bool {
    line.trim_start().get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p))
}

/// A comment line: `*`, LTspice's `;`, ngspice's `#`, `$`, `//` (see [`starred`]).
fn comment_line(line: &str) -> bool {
    line.trim_start().starts_with('*') || comment_rest(line).is_some()
}
/// What follows a comment start other than `*` (see [`COMMENT_LINE`]): None, no such line.
fn comment_rest(line: &str) -> Option<&str> {
    let l = line.trim_start();
    l.strip_prefix("//").or_else(|| l.strip_prefix(COMMENT_LINE))
}
/// What else than `*` (and `//`) starts a comment line (see [`starred`]).
const COMMENT_LINE: [char; 3] = [';', '#', '$'];

/// Translate LTspice-only directive syntax to something ngspice accepts.
fn directive(line: &str, warnings: &mut Vec<String>) -> Option<String> {
    let mut tok = spice_tokens(line).into_iter();
    match tok.next().map(str::to_lowercase).as_deref() {
        Some(".tran") => {
            // LTspice: `.tran <stop>` / `.tran 0 <stop> ...`; ngspice needs a real step.
            // An inline `; comment` must not contribute arguments.
            let tok = spice_tokens(uncommented(line)).into_iter().skip(1);
            const LT_ONLY: [&str; 3] = ["startup", "steady", "nodiscard"];
            let expr = |t: &&str| spice_num(t).is_some() || t.starts_with(['{', '\'']);
            let (rest, lt): (Vec<&str>, Vec<&str>) = tok.partition(|t| !LT_ONLY.contains(&t.to_lowercase().as_str()));
            // startup as LTspice does it: no operating point, the DC sources ramped up (see `startup`)
            let startup = lt.iter().any(|k| k.eq_ignore_ascii_case("startup"));
            for k in lt.iter().filter(|k| !k.eq_ignore_ascii_case("startup")) {
                warnings.push(match k.to_lowercase().as_str() {
                    "steady" => "`.tran steady` is LTspice-only: ngspice runs to the full stop time".into(),
                    _ => "`.tran nodiscard` is LTspice-only: ngspice discards data before Tstart".into(),
                });
            }
            let uic = if startup || rest.iter().any(|t| t.eq_ignore_ascii_case("uic")) { " uic" } else { "" };
            let mut v: Vec<String> = rest.iter().copied().filter(expr).map(str::to_string).collect();
            let step = |stop: &str| match spice_num(stop) {
                Some(n) => {
                    let e = format!("{:.6e}", n / 1000.0); // 7 significant digits: no 9.999999e-9
                    let (m, x) = e.split_once('e').unwrap_or((&e, "0"));
                    format!("{}e{x}", m.trim_end_matches('0').trim_end_matches('.'))
                }
                _ => format!("{{({})/1000}}", stop.trim_matches(['{', '}', '\''])),
            };
            match v.len() {
                1 => v.insert(0, step(&v[0])),
                2.. if spice_num(&v[0]) == Some(0.0) => v[0] = step(&v[1]),
                _ => {}
            }
            Some(format!(".tran {}{uic}{}", v.join(" "), if startup { format!("\n{STARTUP}") } else { String::new() }))
        }
        // LTspice's two-port analysis: kept as a comment, the plot makes S11(V1), S21(V1), Zin(V1)
        // of it (an .ac result's vectors)
        Some(".net") => Some(format!("* {} {NET_MARK}", uncommented(line).trim())),
        Some(".wave" | ".savebias" | ".loadbias") => {
            warnings.push(format!("`{line}` is LTspice-only and was skipped"));
            None
        }
        Some(".backanno") => None,
        _ => Some(line.to_string()),
    }
}

/// A netlist that went through [`prepare`]; the only thing `ngspice::run` accepts.
pub struct Prepared(String, Vec<Sensor>, Vec<String>);

impl Prepared {
    pub fn text(&self) -> &str {
        &self.0
    }

    /// The current sensors in this deck, so results can hide them (see `Sensor`).
    #[cfg(test)]
    pub fn with_sensors(mut self, sensors: Vec<Sensor>) -> Self {
        self.1 = sensors;
        self
    }

    /// This deck with its other analyses commented out (a sweep step needs only `.pz`; a `.tran`
    /// or `.meas` would run every step for nothing).
    pub fn only_pz(&self) -> Prepared {
        const OTHER: [&str; 16] = [".tran", ".ac", ".dc", ".op", ".noise", ".tf", ".sens", ".four", ".meas", ".measure", ".disto", ".pss", ".sp", ".hb", ".print", ".plot"];
        // with their `+` continuation lines (a deck with a .pz is not ramped by .tran startup)
        let text = deck_lines(&self.0).iter().map(|l| {
            if !l.skip && OTHER.contains(&l.head.as_str()) { format!("* {}", l.line) } else { l.line.to_string() }
        }).collect::<Vec<_>>().join("\n") + "\n";
        // (random values, mc(): one sample for all the sweep's steps, not a draw per step)
        let random = crate::laplace::calls(&text, &NG_RANDOM);
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_micros() % 2_000_000_000 + 1);
        let text = if random { after_title(&text, &format!(".options seed={seed}")) } else { text };
        Prepared(text, self.1.clone(), vec![]) // (one .pz per sweep step: no .step)
    }
    /// This deck with the value at `spans` set to `value` (one step of a sweep such as a root locus).
    pub fn at(&self, spans: &[(usize, usize)], value: f64) -> Prepared {
        Prepared(splice(&self.0, spans, value), self.1.clone(), vec![]) // (a sweep step: one run, no .step)
    }


    pub fn sensors(&self) -> &[Sensor] {
        &self.1
    }

    /// Its `.step` statements, parsed: `Steps::at` builds each step's deck when it runs. None: no `.step`.
    pub fn steps(&self) -> Result<Option<Steps<'_>>, String> {
        if self.2.is_empty() {
            return Ok(None);
        }
        let steps = self.2.iter().map(|s| parse_step(s)).collect::<Result<Vec<_>, _>>()?;
        if steps.len() > 3 {
            return Err("at most three .step statements (nested)".into());
        }
        let total = steps.iter().map(|s| s.values.len()).product::<usize>();
        if total > MAX_STEPS {
            return Err(format!("{total} steps: at most {MAX_STEPS}"));
        }
        // each combination's label (`R=1k temp=27`), told apart where values repeat (`list 1k 1k`)
        let mut labels: Vec<String> = (0..total).map(|mut k| steps.iter().map(|s| {
            let v = s.values[k % s.values.len()];
            k /= s.values.len();
            format!("{}={}", s.name(), short_num(v))
        }).collect::<Vec<_>>().join(" ")).collect();
        let mut count: HashMap<&str, usize> = HashMap::new();
        labels.iter().for_each(|l| *count.entry(l).or_default() += 1);
        let shared: Vec<bool> = labels.iter().map(|l| count[l.as_str()] > 1).collect(); // (counted before any is changed)
        for (k, l) in labels.iter_mut().enumerate() {
            if shared[k] {
                l.push_str(&format!(" #{}", k + 1));
            }
        }
        Ok(Some(Steps { deck: self, steps, total, labels }))
    }
}

/// A deck's `.step` statements: `total` combinations, the first `.step` varying fastest.
pub struct Steps<'a> {
    deck: &'a Prepared,
    steps: Vec<Step>,
    pub total: usize,
    labels: Vec<String>,
}

impl Steps<'_> {
    /// Combination `k`: its label and its deck (one at a time: a big deck times a thousand steps
    /// would not fit).
    pub fn at(&self, k: usize) -> Result<(String, Prepared), String> {
        let (mut text, mut i) = (self.deck.0.clone(), k);
        for s in &self.steps {
            text = s.apply(&text, s.values[i % s.values.len()])?;
            i /= s.values.len();
        }
        Ok((self.labels[k].clone(), Prepared(text, self.deck.1.clone(), vec![])))
    }
}

/// More steps are refused (each is a full run, its results kept until the next run).
const MAX_STEPS: usize = 1000;

/// One `.step` statement: what it varies, and its values.
#[derive(Debug, PartialEq)]
pub struct Step {
    what: Stepped,
    pub values: Vec<f64>,
}

#[derive(Debug, PartialEq)]
enum Stepped {
    Param(String),
    Source(String),
    Temp,
}

impl Step {
    fn name(&self) -> &str {
        match &self.what {
            Stepped::Param(n) | Stepped::Source(n) => n,
            Stepped::Temp => "temp",
        }
    }

    /// `text` with this step at `v`: the `.param` set (added if the deck has none), the source's
    /// DC value replaced, or `.options temp=`.
    fn apply(&self, text: &str, v: f64) -> Result<String, String> {
        match &self.what {
            Stepped::Param(n) => {
                let spans = param_spans(text, n);
                // (added first: a .param that uses it, `.param Rtot={R+100}`, is read after it)
                Ok(if spans.is_empty() { after_title(text, &format!(".param {n}={v:e}")) } else { splice(text, &spans, v) })
            }
            Stepped::Temp => {
                // the deck's own `.temp 50` / `.options … temp=50` set, else one added
                // (every one: one left over would set the temperature after the stepped one)
                let lines = deck_lines(text);
                let own: Vec<(usize, usize)> = lines.iter().filter(|l| l.top && !l.skip).filter_map(|l| {
                    let sp = token_spans(l.line);
                    match l.head.as_str() {
                        ".temp" => sp.get(1).copied(),
                        ".options" | ".option" | ".opt" => sp.iter().find(|&&(a, e)| l.low[a..e].starts_with("temp=") && e > a + 5).map(|&(a, e)| (a + 5, e)),
                        _ => None,
                    }.map(|(a, e)| (l.start + a, l.start + e))
                }).collect();
                Ok(if own.is_empty() { before_end(text, &format!(".options temp={v:e}")) } else { splice(text, &own, v) })
            }
            Stepped::Source(n) => {
                let low = n.to_ascii_lowercase();
                let lines = deck_lines(text);
                let at = lines.iter().position(|l| l.top && l.low.split_whitespace().next() == Some(low.as_str()))
                    .ok_or_else(|| format!("`.step {n}`: there is no source {n}"))?;
                // (written over + lines: its DC value may be on one of them, which this would miss)
                if lines[at + 1..].iter().find(|l| !l.skip).is_some_and(|l| l.line.trim_start().starts_with('+')) {
                    return Err(format!("`.step {n}`: {n} is written over several lines: step a .param in its value instead"));
                }
                let line = &lines[at];
                // `V1 a b 5` or `V1 a b DC 5 …`: the DC value (a SINE or PULSE has none to step)
                let sp = token_spans(line.line);
                // (ramped up by .tran startup: the value the ramp ends at)
                // (exactly the ramp, under .tran startup: a PWL of the user's own that starts alike is no ramp)
                // (the mark with its list: prepare ramped; beside other analyses it did not)
                let startup = text.contains(&format!("{STARTUP} ; ramped:"));
                if let Some(&(a, e)) = sp.get(3).filter(|_| ramped_sources(text).contains(&low)).filter(|&&(a, e)| ramp_end(&line.line[a..e]).is_some()) {
                    return Ok(splice(text, &[(line.start + a + RAMP.len(), line.start + e - 1)], v));
                }
                // (the DC value where it is written: right after the nodes, or after a `DC` anywhere)
                let k = sp.iter().skip(3).position(|&(a, e)| line.line[a..e].eq_ignore_ascii_case("dc")).map_or(3, |p| p + 4);
                match sp.get(k).filter(|&&(a, e)| spice_num(&line.line[a..e]).is_some() || line.line[a..e].starts_with('{')) {
                    Some(&(a, e)) => Ok(splice(text, &[(line.start + a, line.start + e)], v)),
                    // `V1 in 0 AC 1`, a SINE: no DC value written (0): one put after the nodes
                    None => {
                        let at = sp.get(2).ok_or_else(|| format!("`.step {n}`: {n} has no nodes"))?.1;
                        let new = format!("{} DC {v:e}{}", &line.line[..at], &line.line[at..]);
                        // (under .tran startup: ramped up like the others)
                        let ramp = startup.then(|| ramped(&new)).flatten();
                        Ok(format!("{}{}{}", &text[..line.start], ramp.as_deref().unwrap_or(&new), &text[line.start + line.line.len()..]))
                    }
                }
            }
        }
    }
}

/// LTspice's `.step`: `[lin|oct|dec] param <name> <start> <stop> <increment|points>`,
/// `… param <name> list <value>…`, a source (`.step V1 0 5 1`) or `temp`.
pub fn parse_step(stmt: &str) -> Result<Step, String> {
    let toks: Vec<&str> = spice_tokens(uncommented(stmt)).into_iter().skip(1).filter(|t| *t != "+").collect();
    let mut t = toks.iter().copied().peekable();
    let mode = match t.peek().map(|m| m.to_ascii_lowercase()) {
        Some(m) if ["lin", "oct", "dec"].contains(&m.as_str()) => (t.next(), m).1,
        _ => "lin".into(),
    };
    let what = match t.next() {
        Some(w) if w.eq_ignore_ascii_case("param") => Stepped::Param(t.next().ok_or(format!("`{stmt}`: no parameter name"))?.to_string()),
        Some(w) if w.eq_ignore_ascii_case("temp") => Stepped::Temp,
        // (a source: its values follow; `.step VDMOS IRF530(Rd) …` is a model parameter)
        Some(w) if w.to_ascii_lowercase().starts_with(['v', 'i']) && !w.contains('(') && !t.peek().is_some_and(|n| n.contains('(')) => Stepped::Source(w.to_string()),
        Some(w) => return Err(format!("`{stmt}`: stepping {w} (a model parameter) is not supported: give the model a {{parameter}} and step that")),
        None => return Err(format!("`{stmt}`: nothing to step")),
    };
    let rest: Vec<&str> = t.collect();
    let num = |s: &str| spice_num(s).ok_or_else(|| format!("`{stmt}`: `{s}` is not a number"));
    let values: Vec<f64> = if rest.first().is_some_and(|r| r.eq_ignore_ascii_case("list")) {
        rest[1..].iter().map(|s| num(s)).collect::<Result<_, _>>()?
    } else {
        let &[a, b, c] = &rest[..] else { return Err(format!("`{stmt}`: needs start, stop and step (or list <values>)")) };
        let (a, b, c) = (num(a)?, num(b)?, num(c)?);
        if mode == "lin" {
            if c == 0.0 || (b - a) * c < 0.0 {
                return Err(format!("`{stmt}`: the step does not lead from start to stop"));
            }
            let n = ((b - a) / c + 1e-9).floor() + 1.0; // (as f64: an absurd count must not overflow)
            (0..n.min(MAX_STEPS as f64 + 1.0) as usize).map(|k| a + k as f64 * c).collect()
        } else {
            if a <= 0.0 || b < a || c < 1.0 {
                return Err(format!("`{stmt}`: oct/dec need 0 < start ≤ stop and at least 1 point"));
            }
            let ratio = (if mode == "dec" { 10f64 } else { 2.0 }).powf(1.0 / c);
            let n = ((b / a).ln() / ratio.ln() + 1e-9).floor() + 1.0;
            (0..n.min(MAX_STEPS as f64 + 1.0) as usize).map(|k| a * ratio.powi(k as i32)).collect()
        }
    };
    if values.is_empty() {
        return Err(format!("`{stmt}`: no values"));
    }
    if values.len() > MAX_STEPS {
        return Err(format!("`{stmt}`: more than {MAX_STEPS} steps"));
    }
    Ok(Step { what, values })
}

/// The parameters `.step` statements set (`.step param C …`): they can be swept like a `.param`.
pub fn step_params(stmts: &[String]) -> Vec<String> {
    stmts.iter().filter_map(|s| match parse_step(s).ok()?.what {
        Stepped::Param(n) => Some(n),
        _ => None,
    }).collect()
}

/// Every top-level `.param` definition of `name` (byte spans of its value).
fn param_spans(text: &str, name: &str) -> Vec<(usize, usize)> {
    deck_lines(text).iter().filter(|l| l.top && l.param())
        .flat_map(|l| param_defs(l.line, &l.low).into_iter().filter(|(n, _)| n.eq_ignore_ascii_case(name)).map(move |(_, (a, e))| (l.start + a, l.start + e)))
        .collect()
}

/// `text` with `line` added after its first (title) line.
fn after_title(text: &str, line: &str) -> String {
    let at = text.find('\n').map_or(text.len(), |k| k + 1);
    format!("{}{}{line}\n{}", &text[..at], if at == text.len() && !text.ends_with('\n') && !text.is_empty() { "\n" } else { "" }, &text[at..])
}

/// `text` with `line` added before its `.end` (or at its end).
fn before_end(text: &str, line: &str) -> String {
    match deck_lines(text).iter().find(|l| l.top && is_end(l.low.trim())) {
        Some(e) => format!("{}{line}\n{}", &text[..e.start], &text[e.start..]),
        None => format!("{}{}{line}\n", text, if text.ends_with('\n') || text.is_empty() { "" } else { "\n" }),
    }
}

/// A step value as people write it: 1k, 2.2u, 100n, 27.
fn short_num(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v}");
    }
    const SI: [(f64, &str); 9] = [(1e12, "T"), (1e9, "G"), (1e6, "Meg"), (1e3, "k"), (1.0, ""), (1e-3, "m"), (1e-6, "u"), (1e-9, "n"), (1e-12, "p")];
    let (f, s) = SI.iter().find(|(f, _)| v.abs() >= *f * 0.9999999).copied().unwrap_or((1e-15, "f"));
    let m = format!("{:.6}", v / f); // (close values must not share a label)
    format!("{}{s}", m.trim_end_matches('0').trim_end_matches('.'))
}

/// A generated 0 V current sensor: source name, its extra node, the device it measures (all
/// lowercase, as ngspice names vectors), the pin (SpiceOrder) and the current's name (`Ib(Q1)`).
#[derive(Clone, Debug, Default)]
pub struct Sensor {
    pub source: String,
    pub node: String,
    pub inst: String,
    pub pin: usize,
    pub label: String,
    /// the part's name as its labels use it (drawn, or the netlist's where that clashed), lowercase
    pub name: String,
    /// its 1 TΩ hold resistor, if it has one (a subcircuit pin that could float), lowercase, else ""
    pub hold: String,
    /// the label by the netlist's name (`Ib(QT1)` for `Ib(T1)`), lowercase
    pub alias: String,
}

impl Netlist {
    /// The deck ngspice runs for this (simulation) netlist: includes inlined and filtered, its
    /// current sensors known. The GUI's simulation and root locus and MCP all run this. It reads the
    /// .include/.lib files (can be slow): call it on a blocking thread, without the library lock.
    pub fn prepared(&self, dir: Option<&std::path::Path>, warnings: &mut Vec<String>) -> Prepared {
        let p = prepare(&self.text, dir, warnings);
        Prepared(ltspice_meas(&p.0, &self.sensors, warnings), self.sensors.clone(), self.steps.clone())
    }
}

/// LTspice `.meas` syntax that ngspice reads differently or not at all, rewritten (a statement with
/// its `+` lines becomes one line). ngspice takes a vector or `par('…')` where LTspice takes any
/// expression, and only a number after VAL= (`VAL=0.5*V(vdd)` silently became 0.5):
/// - an expression measured (AVG … INTEG, FIND, DERIV, TRIG, TARG) becomes `par('…')`;
/// - `TRIG|TARG <x> VAL=<expr>` becomes `par('<x>-(<expr>)') VAL=0`: the same crossing;
/// - `WHEN <a>=<b>`, unless `b` is a number, becomes `WHEN par('<a>-(<b>)')=0`;
/// - `I(R1)` of a part with a current sensor becomes the sensor's current;
/// - no analysis type (LTspice measures the simulation's) gets the deck's.
fn ltspice_meas(text: &str, sensors: &[Sensor], warnings: &mut Vec<String>) -> String {
    if !contains_ci(text, ".meas") {
        return text.to_string(); // nothing to rewrite: no pass over a big deck
    }
    // (only with the circuit's sensors: the warnings pass before a run has none)
    if !sensors.is_empty() {
        for l in statements(text, &[".meas", ".measure"]) {
            if let (_, Some(w)) = sensor_currents(&l, sensors) {
                warnings.push(format!(".meas: {w}"));
            }
        }
    }
    let lines = deck_lines(text);
    // the deck's first analysis ngspice can measure (an .op before a .tran: the .tran)
    let an = lines.iter().find_map(|l| l.head.strip_prefix('.').filter(|h| !l.skip && ["tran", "ac", "dc", "sp"].contains(h))).unwrap_or("");
    let meas = |l: &DeckLine| l.head == ".meas" || l.head == ".measure";
    // ngspice allows 99 par() calls per deck and exits past that: rewrites stop before
    let mut budget = 99usize.saturating_sub(statements(text, &PAR_STATEMENTS).iter().map(|s| par_calls(s)).sum());
    let mut out: Vec<String> = vec![];
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        if l.skip || !meas(l) || l.low.trim_start().starts_with('+') {
            out.push(l.line.to_string());
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < lines.len() && meas(&lines[j]) && (lines[j].skip || lines[j].low.trim_start().starts_with('+')) {
            j += 1;
        }
        let stmt = &lines[i..j];
        // each line without its inline comment before they are joined
        let joined = stmt.iter().filter(|l| !l.skip).map(|l| uncommented(l.line).trim().trim_start_matches('+').trim().to_string()).collect::<Vec<_>>().join(" ");
        match meas_statement(&joined, sensors, an) {
            Ok(new) if spice_tokens(&new) == spice_tokens(&joined) => out.extend(stmt.iter().map(|l| l.line.to_string())), // as written
            // as written it would be misread (the reason for the rewrite): left out, and said
            Ok(new) if par_calls(&new).saturating_sub(par_calls(&joined)) > budget => {
                warnings.push(format!("{}: past ngspice's 99 par() expressions per deck: left out", joined.split_whitespace().take(3).collect::<Vec<_>>().join(" ")));
                out.extend(stmt.iter().map(|l| format!("* {}", l.line)));
            }
            Ok(new) => {
                budget -= par_calls(&new).saturating_sub(par_calls(&joined));
                out.push(new);
                out.extend(stmt.iter().filter(|l| l.skip).map(|l| l.line.to_string())); // comments stay
            }
            // ngspice would measure something else without saying so: left out, and said
            Err(why) => {
                warnings.push(why);
                out.extend(stmt.iter().map(|l| format!("* {}", l.line)));
            }
        }
        i = j;
    }
    out.join("\n") + "\n"
}

/// The deck's statements of the dot commands `heads` (lowercase, e.g. ".meas"), each with its `+`
/// lines joined: comment and blank lines between them skipped, inline comments cut (deck_lines).
pub fn statements(text: &str, heads: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in deck_lines(text).iter().filter(|l| !l.skip && heads.contains(&l.head.as_str())) {
        let t = uncommented(l.line).trim();
        match t.strip_prefix('+') {
            Some(more) if !out.is_empty() => {
                let last = out.last_mut().unwrap();
                last.push(' ');
                last.push_str(more.trim());
            }
            _ => out.push(t.to_string()),
        }
    }
    out
}

/// Whether `hay` contains `needle` (ASCII, any case), without a lowercase copy of a big deck.
pub fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.as_bytes().windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// How many par() calls `s` makes (ngspice: at most 99 per deck; one helper vector pair each):
/// `par(` as a word (not the end of a name like `rpar(`).
pub fn par_calls(s: &str) -> usize {
    let l = s.to_ascii_lowercase();
    l.match_indices("par(").filter(|(k, _)| !l[..*k].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_')).count()
}

/// The output statements whose par() calls ngspice counts (the same ones run_until counts).
pub const PAR_STATEMENTS: [&str; 6] = [".meas", ".measure", ".print", ".plot", ".four", ".save"];

/// A line without its inline comment (`;` LTspice, ` $` ngspice).
pub fn uncommented(l: &str) -> &str {
    let l = l.split(';').next().unwrap_or("");
    // as ngspice strips them: `$` after a blank or a comma (or at the start), and `//`
    let at = l.char_indices().find(|&(k, c)| (c == '$' && (k == 0 || l[..k].ends_with(|p: char| p.is_whitespace() || p == ','))) || l[k..].starts_with("//"));
    at.map_or(l, |(k, _)| &l[..k])
}

/// One `.meas` statement (see [`ltspice_meas`]); Err: what ngspice would silently get wrong.
fn meas_statement(s: &str, sensors: &[Sensor], an: &str) -> Result<String, String> {
    // `FROM = 1m` is `FROM=1m` (the tokens below split at blanks): blanks next to `=` go
    let s = uncommented(s);
    let (mut flat, mut blanks) = (String::with_capacity(s.len()), String::new());
    for c in s.chars() {
        if c.is_whitespace() {
            blanks.push(c);
            continue;
        }
        if c != '=' && !flat.ends_with('=') {
            flat.push_str(&blanks);
        }
        blanks.clear();
        flat.push(c);
    }
    let s = sensor_currents(flat.trim_end(), sensors).0;
    let mut toks: Vec<String> = spice_tokens(&s).into_iter().map(str::to_string).collect();
    let low = |t: &str| t.to_ascii_lowercase();
    // a vector, V(..)/I(..) and ngspice's vdb(..), vm(..) … or a number: ngspice reads it as it is
    // (par() would turn it into a B-source, which knows none of the vdb/vm/… forms)
    const VECTOR: [&str; 14] = ["v", "i", "vdb", "vm", "vp", "vr", "vi", "vg", "idb", "im", "ip", "ir", "ii", "ig"];
    // V(a), vdb(out) …: a vector ngspice evaluates itself
    let call = |x: &str| {
        let l = low(x.trim());
        l.find('(').is_some_and(|o| VECTOR.contains(&&l[..o]) && l.ends_with(')') && o + 1 < l.len() && !l[o + 1..l.len() - 1].contains(['(', ')']))
    };
    let simple = |x: &str| {
        let (x, l) = (x.trim(), low(x.trim()));
        let par = l.starts_with("par(") && l.ends_with(')'); // already ngspice's form
        call(x) || par || (!x.is_empty() && x.chars().all(|c| c.is_ascii_alphanumeric() || "_#.:".contains(c)))
    };
    // a quantity already in par('…') form, unwrapped (no par() in a par())
    let unpar = |x: &str| -> String {
        let x = x.trim();
        if low(x).starts_with("par(") && x.ends_with(')') { x[4..x.len() - 1].trim().trim_matches('\'').to_string() } else { x.to_string() }
    };
    let number = |v: &str| {
        let v = v.trim();
        (v.starts_with('{') && v.ends_with('}')) // numparam evaluates it
            || (v.chars().all(|c| c.is_ascii_alphanumeric() || ".+-".contains(c)) && v.starts_with(|c: char| c.is_ascii_digit() || ".+-".contains(c)) && spice_num(v).is_some())
    };
    let par = |x: &str| format!("par('{}')", x.trim().trim_matches('\''));
    let typed = meas_name_at(&toks) == 2;
    let typ = if typed { low(&toks[1]) } else { an.to_string() };
    // .ac and .sp are small-signal: a par() B-source there is linearized (silently wrong)
    let linear = typ == "ac" || typ == "sp";
    // AC: par() is a B-source, linearized there: an expression would come out silently wrong.
    // LTspice's mag()/db()/ph()/re()/im() of a voltage are ngspice's vm/vdb/vp/vr/vi; anything else
    // stays as written (ngspice then reports it failed rather than a wrong number)
    let ac_vector = |x: &str| -> Option<String> {
        let (x, l) = (x.trim(), low(x.trim()));
        let o = l.find('(')?;
        // (not ph(): LTspice's is in degrees, ngspice's vp() in radians)
        let f = [("mag", "vm"), ("db", "vdb"), ("re", "vr"), ("real", "vr"), ("im", "vi"), ("imag", "vi")].iter().find(|(n, _)| *n == &l[..o])?.1;
        let inner = x.get(o + 1..x.len().checked_sub(1)?)?.trim();
        (l.ends_with(')') && low(inner).starts_with("v(") && inner.ends_with(')') && !inner[2..inner.len() - 1].contains(['(', ')'])).then(|| format!("{f}{}", &inner[1..]))
    };
    let measured = |x: &str| if simple(x) { x.to_string() } else if linear { ac_vector(x).unwrap_or(x.to_string()) } else { par(x) };
    let keyword = |t: &str| t.contains('=') || ["trig", "targ", "when", "at", "last"].contains(&low(t).as_str());
    let when_end = |t: &str| ["td=", "rise=", "fall=", "cross=", "from=", "to=", "last", "trig", "targ"].iter().any(|k| low(t).starts_with(k));
    const OPTION: [&str; 8] = ["val", "td", "rise", "fall", "cross", "at", "from", "to"];
    let mut out: Vec<String> = toks.iter().take(1).cloned().collect();
    let mut i = 1;
    if typed {
        out.push(toks[1].clone());
        i = 2;
    } else if ["tran", "ac", "dc", "sp"].contains(&an) {
        out.push(an.to_string()); // (ngspice measures only these: an .op/.noise deck keeps it untyped)
    }
    if let Some(name) = toks.get(i) {
        out.push(name.clone());
        i += 1;
    }
    while i < toks.len() {
        let (t, l) = (toks[i].clone(), low(&toks[i]));
        i += 1;
        match l.as_str() {
            "avg" | "rms" | "min" | "max" | "pp" | "integ" | "integral" | "find" | "deriv" | "derivative" | "trig" | "targ" => {
                out.push(t);
                let start = i;
                while i < toks.len() && !keyword(&toks[i]) {
                    i += 1;
                }
                let mut x = toks[start..i].join(" ");
                let edge = l == "trig" || l == "targ";
                // the level: LTspice's `TRIG V(in)=0.5*V(vdd)` ([VAL]=), else a VAL= among this
                // TRIG/TARG's options (TD=, RISE= may come first)
                let (mut level, mut val_at) = (None, None);
                // a level written with blanks (`VAL=0.5 * V(vdd)`) runs on to the next keyword: its
                // other tokens are taken in (and dropped from the output below)
                let rest_of = |toks: &mut Vec<String>, from: usize, mut v: String| {
                    let mut k = from;
                    while k < toks.len() && !keyword(&toks[k]) {
                        v.push_str(&std::mem::take(&mut toks[k]));
                        k += 1;
                    }
                    (v, k)
                };
                if edge && x.is_empty() {
                    if let Some((a, b)) = toks.get(i).and_then(|tk| tk.split_once('=')).filter(|(a, _)| !a.is_empty() && !OPTION.contains(&low(a).as_str())).map(|(a, b)| (a.to_string(), b.to_string())) {
                        let (b, k) = rest_of(&mut toks, i + 1, b);
                        (x, level, i) = (a, Some(b), k);
                    }
                }
                if edge && level.is_none() {
                    val_at = (i..toks.len()).take_while(|&j| toks[j].contains('=')).find(|&j| low(&toks[j]).starts_with("val="));
                    if let Some(j) = val_at {
                        let v = toks[j][4..].to_string();
                        level = Some(rest_of(&mut toks, j + 1, v).0);
                    }
                }
                match level {
                    Some(v) if !number(&v) && linear => {
                        return Err(format!("{}: in AC/SP, ngspice takes only a number as the level ({v}), and would measure at its first number: left out", out.get(2).map_or(".meas", |n| n.as_str())));
                    }
                    Some(v) if !x.is_empty() && !number(&v) => {
                        out.push(par(&format!("{}-({})", unpar(&x), v.trim_matches('\''))));
                        match val_at {
                            Some(j) => toks[j] = "VAL=0".into(),
                            None => out.push("VAL=0".into()),
                        }
                    }
                    Some(v) if val_at.is_none() => {
                        out.push(measured(&x));
                        out.push(format!("VAL={v}")); // `TRIG V(in)=1`: ngspice wants VAL=
                    }
                    _ if x.is_empty() => {}
                    _ => out.push(measured(&x)),
                }
            }
            // LTspice `PARAM <expr>` (from other results): ngspice writes param='<expr>'
            "param" => {
                let start = i;
                while i < toks.len() && !keyword(&toks[i]) {
                    i += 1;
                }
                out.push(format!("param='{}'", toks[start..i].join("").trim_matches('\'')));
            }
            "when" => {
                let start = i;
                while i < toks.len() && !when_end(&toks[i]) {
                    i += 1;
                }
                let w = toks[start..i].join("");
                match w.split_once('=') {
                    Some((_, b)) if linear && !(number(b) || call(b) || ac_vector(b).is_some()) => {
                        return Err(format!("{}: in AC/SP, ngspice takes only a number or a vector as the level ({b}), and would measure at its first number: left out", out.get(2).map_or(".meas", |n| n.as_str())));
                    }
                    Some((a, b)) => {
                        let a = if linear { ac_vector(a).unwrap_or(a.to_string()) } else { a.to_string() };
                        let b = if linear { ac_vector(b).unwrap_or(b.to_string()) } else { b.to_string() };
                        let b = b.as_str();
                        // ngspice reads `vector=number` and `vector=vector` itself (and AC stays as written);
                        // a bare name on the right is a .param in LTspice: par() resolves it
                        if simple(&a) && (number(b) || call(b)) || linear {
                            out.push(format!("WHEN {a}={b}"));
                        } else {
                            out.push(format!("WHEN {}=0", par(&format!("{}-({})", unpar(&a), b.trim_matches('\'')))));
                        }
                    }
                    None => out.push(format!("WHEN {w}")),
                }
            }
            _ if t.is_empty() => {} // taken into a level above
            _ => out.push(t),
        }
    }
    Ok(out.join(" "))
}

pub const TYPES_MEAS: [&str; 7] = ["tran", "ac", "dc", "op", "noise", "tf", "sp"];

/// Where a `.meas` statement's name is among its tokens: after the analysis type, if it has one.
pub fn meas_name_at<S: AsRef<str>>(tokens: &[S]) -> usize {
    if tokens.get(1).is_some_and(|t| TYPES_MEAS.contains(&t.as_ref().to_ascii_lowercase().as_str())) { 2 } else { 1 }
}

/// Part currents by their names, `I(R1)`, `Ic(Q1)`, `Ib(Q1)`, `Ix(U1:OUT)` (or by the netlist's
/// names, `Ib(QT1)`), → the sensor's current `i(vsense_…)` (ngspice knows only branch currents);
/// `I(Q1)` is the pin-1 current (Ic) as before. With what it could not map: a part current's name
/// the circuit has not (`Ib(Q9)`: ngspice has no such functions; not `ig(`, its group delay of a
/// source current) or a subcircuit's `I(U1)` (its pins have the currents).
pub fn sensor_currents(s: &str, sensors: &[Sensor]) -> (String, Option<String>) {
    let mut out = String::with_capacity(s.len());
    let mut problem = None;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        // the word right before "(" (whole: `II(R1)` is not `I(R1)`)
        let head = &rest[..open];
        let start = head.char_indices().rev().take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_').last().map_or(open, |(k, _)| k);
        let name = &head[start..];
        let close = rest[open + 1..].find(')').map(|e| open + 1 + e);
        let arg = close.map_or("", |e| rest[open + 1..e].trim());
        let key = format!("{name}({arg})").to_ascii_lowercase();
        let ic = |x: &Sensor| x.inst.starts_with('x') && x.label.starts_with("Ix(");
        let hit = close.filter(|_| !name.is_empty()).and_then(|_| {
            // an exact name first (a part's own I(T1) before another's alias), then by the netlist's
            // name, then `I(Q1)`: pin 1 of a two-pin part or transistor (a subcircuit's are per pin)
            sensors.iter().find(|x| x.label.eq_ignore_ascii_case(&key)).or_else(|| sensors.iter().find(|x| x.alias == key)).or_else(|| sensors.iter().find(|x| {
                x.pin == 0 && !ic(x) && name.eq_ignore_ascii_case("i") && (x.inst.eq_ignore_ascii_case(arg) || x.name.eq_ignore_ascii_case(arg))
            }))
        });
        match (hit, close) {
            (Some(x), Some(e)) => {
                out.push_str(&head[..start]);
                out.push_str(&format!("i({})", x.source));
                rest = &rest[e + 1..];
            }
            _ => {
                let n = name.to_ascii_lowercase();
                let whole = (n == "i").then(|| sensors.iter().find(|x| ic(x) && (x.name.eq_ignore_ascii_case(arg) || x.inst.eq_ignore_ascii_case(arg)))).flatten();
                if problem.is_some() || close.is_none() {
                } else if let Some(x) = whole {
                    problem = Some(format!("I({0}) is no single current: {0} has one per pin, e.g. {1}", arg.to_uppercase(), x.label));
                } else if n.len() == 2 && n.starts_with('i') && "xcbeds".contains(&n[1..]) {
                    problem = Some(format!("{name}({arg}): no such part current here; a part's right-click menu lists its currents while probing (Ib(Q1), Ix(U1:OUT))"));
                }
                out.push_str(&rest[..open + 1]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    (out, problem)
}

/// The pins' currents by their usual names: `I(R1)`; `Ic(Q1)` `Ib(Q1)` `Ie(Q1)`; `Id(M1)` `Ig(M1)`
/// `Is(M1)` `Ib(M1)`; `Id(J1)` …; `Ix(U1:OUT)` for a subcircuit's pins: by their names, or all by
/// number where names are empty, repeat (two GND), would not read as one (`a)b`) or clash.
fn pin_labels(prefix: char, inst: &str, pins: &[Pin]) -> Vec<String> {
    let roles = match prefix { 'Q' => "cbes", 'M' => "dgsb", 'J' => "dgs", _ => "" };
    if prefix == 'X' && pins.len() > 2 {
        let names: Vec<String> = pins.iter().map(|p| p.name.trim().to_string()).collect();
        let readable = names.iter().all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || "_+-.".contains(c)));
        let distinct = names.iter().map(|n| n.to_lowercase()).collect::<HashSet<_>>().len() == names.len();
        return (0..pins.len()).map(|k| format!("Ix({inst}:{})", if readable && distinct { names[k].clone() } else { (k + 1).to_string() })).collect();
    }
    // (a pin beyond the usual roles, a 5th of a Q: by number, never a second `I(Q1)`)
    (0..pins.len()).map(|k| match roles.chars().nth(k) {
        Some(r) => format!("I{r}({inst})"),
        None if k == 0 => format!("I({inst})"),
        None => format!("Ix({inst}:{})", k + 1),
    }).collect()
}

/// Make a netlist safe and self-contained for ngspice. This is the single security filter:
/// - `.include`/`.inc`/`.lib` are inlined here (relative to the schematic's folder, like LTspice),
///   so ngspice never opens a file itself;
/// - `.control` blocks and every other line [`command_line`] names (`*#`, `.title`, `*ng_script`:
///   ngspice executes them on load, including `shell`) are dropped everywhere, in the schematic and
///   in included files alike; the title line is made a comment; the checks read each line as it goes
///   out ([`ascii`], comment lines starred). `ngspice::run` refuses a deck with such a line left.
pub fn prepare(text: &str, dir: Option<&std::path::Path>, warnings: &mut Vec<String>) -> Prepared {
    let mut x = Expander { warnings, out: String::new(), seen: HashSet::new(), scope: (None, 0), read: true, diode: None, behavioural: vec![] };
    x.expand(text, dir, None, 0);
    x.warn_behavioural();
    // (also in included files; Laplace first: a random call there is said by its own name)
    let out = crate::laplace::rewrite(&x.out, x.warnings);
    let translated = match lt_random(&out, x.warnings) {
        std::borrow::Cow::Owned(t) => Some(t),
        std::borrow::Cow::Borrowed(_) => None, // (nothing to translate: not copied)
    };
    let out = translated.unwrap_or(out);
    let out = lt_white(&out, x.warnings).unwrap_or(out);
    // .tran … startup: every DC source of the deck ramped up (drawn, written in a directive, in a
    // sheet or an included file)
    // (with other analyses in the deck, as `nomospice --mcp` runs a schematic: their operating point
    // would see the ramps at 0; not ramped then, said)
    const OTHERS: [&str; 10] = [".op", ".ac", ".dc", ".noise", ".tf", ".pz", ".sens", ".disto", ".pss", ".sp"];
    let startup = out.contains(STARTUP);
    let ls = if startup { deck_lines(&out) } else { vec![] };
    let others = ls.iter().any(|l| !l.skip && OTHERS.contains(&l.head.as_str()));
    if others {
        x.warnings.push("`.tran … startup` beside other analyses: the sources are not ramped (their operating point would see them at 0); run the .tran on its own".into());
    }
    let out = if startup && !others {
        // (not the title line, nor comments)
        let mut s = String::with_capacity(out.len() + 64);
        let mut names: Vec<String> = vec![];
        let piece = |i: usize| &out[ls[i].start..ls.get(i + 1).map_or(out.len(), |n| n.start)];
        let mut k = 0;
        while k < ls.len() {
            let l = &ls[k];
            // (a source's statement only: the rest as it is, without looking ahead)
            if k == 0 || l.skip || !l.low.trim_start().starts_with(['v', 'i']) {
                s.push_str(piece(k));
                k += 1;
                continue;
            }
            // (a source over + lines, comments between them allowed: ramped as one line, its
            // continuations then gone, the comments kept; each line's own comment off before)
            let mut n = k + 1;
            while n < ls.len() && (ls[n].skip || ls[n].line.trim_start().starts_with('+')) {
                n += 1;
            }
            while n > k + 1 && ls[n - 1].skip {
                n -= 1;
            }
            let joined = (k..n).filter(|&i| !ls[i].skip).map(|i| uncommented(ls[i].line).trim().trim_start_matches('+').trim()).collect::<Vec<_>>().join(" ");
            match ramped(&joined) {
                Some(r) => {
                    names.push(joined.split_whitespace().next().unwrap_or("").to_ascii_lowercase());
                    s.push_str(&r);
                    s.push_str(&l.line[uncommented(l.line).trim_end().len()..]); // (the first line's comment)
                    s.push('\n');
                    for i in k + 1..n {
                        if ls[i].skip {
                            s.push_str(piece(i));
                        }
                    }
                    k = n;
                }
                None => { s.push_str(piece(k)); k += 1; }
            }
        }
        // (the sources ramped, named on the mark: only these are taken for ramps by a .step, a PWL of
        // the user's own that looks alike is none)
        s.replacen(STARTUP, &format!("{STARTUP} ; ramped: {}", names.join(" ")), 1)
    } else { out };
    Prepared(out, vec![], vec![])
}

/// LTspice's noise function `white(x)` (between -0.5 and 0.5, a new value at each integer x, in
/// between interpolated) in a B source: ngspice has none, so `white(c*time)` becomes the voltage of
/// a noise source of its own, ngspice's TRNOISE, its new values of the same rms (1/√12: σ 0.3235, clipped) and a new value every
/// 1/c seconds, kept within ±0.5 as LTspice's is. Other arguments, and white() in a subcircuit (its
/// node would be local there), are said and left; a subcircuit with a `.func white` of its own is
/// left alone. None: no white() in the deck.
fn lt_white(text: &str, warnings: &mut Vec<String>) -> Option<String> {
    // (`.func white(x) …` of the deck's top level: its own; a subcircuit's is that subcircuit's)
    if !crate::laplace::calls(text, &["white"]) || deck_funcs(text, true).iter().any(|f| f == "white") {
        return None;
    }
    let ls = deck_lines(text);
    // (the subcircuits that define white() themselves: a .subckt's name up to its .ends)
    let name_of = |l: &DeckLine| l.low.trim_start().strip_prefix(".subckt").filter(|r| r.starts_with(char::is_whitespace)).and_then(|r| r.split_whitespace().next()).map(str::to_string);
    let ends = |l: &DeckLine| l.low.trim_start().starts_with(".ends");
    let (mut own_in, mut cur) = (HashSet::<String>::new(), None::<String>);
    for l in &ls {
        if let Some(n) = name_of(l) {
            cur = Some(n);
        } else if ends(l) {
            cur = None;
        } else if let (Some(s), true) = (&cur, !l.skip && func_head(&l.low) == Some("white")) {
            own_in.insert(s.clone());
        }
    }
    // (one noise source per rate: white(1e5*time) twice is the same noise, as in LTspice; its names
    // none the deck has already)
    let low = text.to_ascii_lowercase();
    let mut next = 0;
    let mut fresh = || loop {
        next += 1;
        if !low.contains(&format!("nmsp_white{next}")) {
            break next;
        }
    };
    let (mut out, mut sources, mut cur, mut left) = (String::with_capacity(text.len() + 128), Vec::<(f64, String, u32)>::new(), None::<String>, false);
    for (k, l) in ls.iter().enumerate() {
        let piece = &text[l.start..ls.get(k + 1).map_or(text.len(), |n| n.start)];
        if let Some(n) = name_of(l) {
            cur = Some(n);
        } else if ends(l) {
            cur = None;
        }
        let code = uncommented(l.line); // (not in a `;` comment)
        let theirs = cur.as_ref().is_some_and(|s| own_in.contains(s));
        let b = l.low.trim_start().starts_with('b');
        if l.skip || !b || theirs || !crate::laplace::calls(code, &["white"]) {
            // (a white() on a `+` line or in a .func body is not translated: said)
            left |= !l.skip && !theirs && !b && crate::laplace::calls(code, &["white"]);
            out.push_str(piece);
            continue;
        }
        let mut rest = code;
        while let Some(k) = crate::laplace::call_at(rest, &["white"]) {
            let open = k + rest[k..].find('(').unwrap_or(0);
            let close = matching(rest, open);
            let Some(close) = close else { break };
            let arg = rest[open + 1..close].replace(' ', "");
            // c*time, time*c, time
            let c = if arg.eq_ignore_ascii_case("time") { Some(1.0) } else {
                arg.split_once('*').and_then(|(a, b)| if b.eq_ignore_ascii_case("time") { spice_num(a) } else if a.eq_ignore_ascii_case("time") { spice_num(b) } else { None })
            }.filter(|c| *c > 0.0);
            match c {
                Some(c) if l.top => {
                    let n = match sources.iter().find(|s| s.0 == c) {
                        Some(s) => s.2,
                        None => {
                            let n = fresh();
                            sources.push((c, format!("Vnmsp_white{n} nmsp_white{n} 0 dc 0 trnoise(0.323524 {:e} 0 0)", 1.0 / c), n));
                            n
                        }
                    };
                    out.push_str(&rest[..k]);
                    out.push_str(&format!("max(min(v(nmsp_white{n}),0.5),-0.5)"));
                }
                _ => {
                    warnings.push(format!("`white({arg})`: nomospice turns white(c*time) into noise (ngspice has no white()); {}", match (l.top, c) {
                        (false, Some(_)) => "not inside a subcircuit",
                        (false, None) => "not inside a subcircuit, and the argument as a number times time",
                        _ => "write the argument as a number times time",
                    }));
                    out.push_str(&rest[..=close]);
                }
            }
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        out.push_str(&piece[code.len()..]);
    }
    // (the noise sources before .end, at the top level)
    for s in &sources {
        out = before_end(&out, &s.1);
    }
    if left {
        warnings.push("`white()` only becomes noise in a behavioral source's own line (not on a + line or in a .func): ngspice has no white()".into());
    }
    Some(out)
}

/// The name a lowercase line `.func name(…` (`.function` too) defines.
fn func_head(low: &str) -> Option<&str> {
    let t = low.trim_start();
    let head = [".function", ".func"].into_iter().find(|h| t.len() > h.len() && t.starts_with(h) && t.as_bytes()[h.len()].is_ascii_whitespace())?;
    t[head.len()..].trim_start().split(|c: char| c == '(' || c.is_whitespace()).next()
}


/// The names of the deck's `.func`s (`.function` too), lowercase; `top`: its top level's only (a
/// subcircuit's are local).
fn deck_funcs(text: &str, top: bool) -> Vec<String> {
    deck_lines(text).iter().filter_map(|l| func_head(&l.low).filter(|_| !top || l.top).map(str::to_string)).collect()
}

/// The `)` matching the `(` at `open`.
pub(crate) fn matching(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    s[open..].char_indices().find_map(|(i, c)| {
        depth += match c { '(' => 1, ')' => -1, _ => 0 };
        (depth == 0).then_some(open + i)
    })
}

/// LTspice's random functions as ngspice's, which draw anew each time a deck is read (so each run
/// of `.step param run 1 100 1` is another sample): `mc(x,tol)`, x·(1 ± tol) uniform, is
/// `unif(x,tol)`; `flat(x)`, uniform in ±x, is `aunif(0,x)`; `gauss(x)`, normal with σ = x, is
/// `agauss(0,x,1)` (ngspice's own three-argument gauss stays). Only where ngspice's parameter
/// expressions are: in `{…}` and on `.param` lines (not in comments, a B source's expression, a
/// file name), and not where the deck has a `.func` of that name.
/// A `.param` holding one draws anew wherever it is used (ngspice makes it a `.func`), as LTspice
/// does: no value shared by several parts.
fn lt_random<'a>(text: &'a str, warnings: &mut Vec<String>) -> std::borrow::Cow<'a, str> {
    let calls = |t: &str| crate::laplace::calls(t, &RANDOM); // (a name with `(` after it, blanks between too)
    if !calls(text) {
        return std::borrow::Cow::Borrowed(text);
    }
    // (`.func gauss(x) …`: the deck's own; LTspice also spells it .function)
    let own = deck_funcs(text, false);
    let (mut out, mut param) = (String::with_capacity(text.len() + 64), false);
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with('*') || t.trim().is_empty() {
            out.push_str(line);
            continue;
        }
        // (a `.param` statement's `+` lines are of it too; a `.func`'s body is an expression too.
        // ponytail: line by line, so a brace or a call's parentheses open across a `+` line are
        // not seen; join the lines if that occurs)
        if !t.starts_with('+') {
            param = [".param", ".func", ".function"].iter().any(|h| t.get(..h.len()).is_some_and(|x| x.eq_ignore_ascii_case(h)) && t[h.len()..].starts_with(char::is_whitespace));
        }
        let code = uncommented(line);
        if calls(code) {
            out.push_str(&random_calls(code, param, &own, warnings));
            out.push_str(&line[code.len()..]);
        } else {
            out.push_str(line);
        }
    }
    std::borrow::Cow::Owned(out)
}

pub const RANDOM: [&str; 3] = ["mc", "flat", "gauss"];
/// ngspice's random functions (what `RANDOM` becomes, and its own).
pub const NG_RANDOM: [&str; 5] = ["agauss", "gauss", "aunif", "unif", "limit"];

/// `s` with LTspice's random calls translated (see `lt_random`): everywhere if `all`, else in braces.
fn random_calls(s: &str, all: bool, own: &[String], warnings: &mut Vec<String>) -> String {
    let (b, mut out, mut k, mut braces, mut quoted, mut ticked) = (s.as_bytes(), String::with_capacity(s.len()), 0, 0i32, false, false);
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while k < b.len() {
        // (an expression: in braces, between ' ', a whole .param/.func line; not a "file name")
        let call = (!quoted && (all || braces > 0 || ticked))
            .then(|| RANDOM.into_iter().find(|f| {
                b.len() >= k + f.len() && b[k..k + f.len()].eq_ignore_ascii_case(f.as_bytes()) && (k == 0 || !word(b[k - 1])) && s[k + f.len()..].trim_start().starts_with('(') && !own.iter().any(|o| o == f)
            }))
            .flatten();
        // its arguments: up to the matching parenthesis (an unclosed one: left as it is)
        let open = call.map(|f| s.len() - s[k + f.len()..].trim_start().len());
        let close = open.and_then(|open| matching(s, open));
        let (Some(f), Some(open), Some(close)) = (call, open, close) else {
            let c = s[k..].chars().next().unwrap_or(' ');
            if !quoted {
                braces = (braces + match c { '{' => 1, '}' => -1, _ => 0 }).max(0); // (a stray } counts for nothing)
            }
            quoted ^= c == '"'; // ("a file{name}.csv")
            ticked ^= c == '\'' && !quoted;
            out.push(c);
            k += c.len_utf8();
            continue;
        };
        let inner = random_calls(&s[open + 1..close], true, own, warnings);
        let args = if inner.trim().is_empty() { 0 } else {
            let mut d = 0;
            inner.chars().filter(|&c| {
                d += match c { '(' | '{' => 1, ')' | '}' => -1, _ => 0 };
                c == ',' && d == 0
            }).count() + 1
        };
        match (f, args) {
            ("mc", 2) => out.push_str(&format!("unif({inner})")),
            ("flat", 1) => out.push_str(&format!("aunif(0,{inner})")),
            ("gauss", 1) => out.push_str(&format!("agauss(0,{inner},1)")),
            // (ngspice's own gauss(nom, rvar, sigma) has three)
            ("gauss", 3) => out.push_str(&format!("{}({inner})", &s[k..k + f.len()])),
            _ => {
                let text = format!("{}({inner})", &s[k..k + f.len()]);
                warnings.push(format!("`{text}`: {}", match f { "mc" => "mc takes two arguments, mc(value, tolerance)", "flat" => "flat takes one argument, flat(x): uniform in ±x", _ => "gauss takes one argument, gauss(sigma)" }));
                out.push_str(&text);
            }
        }
        k = close + 1;
    }
    out
}

/// The warnings `prepare` would give for the schematic's own lines, without reading any
/// included file (cheap enough for every edit).
pub fn prepare_warnings(text: &str, dir: Option<&std::path::Path>, warnings: &mut Vec<String>) {
    let mut x = Expander { warnings, out: String::new(), seen: HashSet::new(), scope: (None, 0), read: false, diode: None, behavioural: vec![] };
    x.expand(text, dir, None, 0);
    x.warn_behavioural();
    let expanded = crate::laplace::rewrite(&std::mem::take(&mut x.out), x.warnings);
    lt_random(&expanded, x.warnings); // (mc(1k): said while editing too)
    lt_white(&expanded, x.warnings); // (white(V(x)) too)
    ltspice_meas(&expanded, &[], warnings); // a .meas left out is said before a run too (the schematic's own: included files are read only by a run)
}

/// `.lib "path with spaces.lib" section` -> (path, section)
fn include_args(t: &str) -> (String, Option<&str>) {
    let rest = t.split_once(char::is_whitespace).map_or("", |r| r.1).trim();
    let (file, tail) = match rest.chars().next() {
        Some(q @ ('"' | '\'')) => rest[1..].split_once(q).unwrap_or((&rest[1..], "")),
        // (a `;` comment touching the name, `models.lib;vendor`: nothing after it)
        _ => match rest.split_once(char::is_whitespace).unwrap_or((rest, "")) {
            (f, tail) => f.split_once(';').map_or((f, tail), |(f, _)| (f, "")),
        },
    };
    // (Windows-authored paths; a comment after them, `; vendor`, `TT;typical`, `$ x`, `// corner`, no
    // section; `//` in the name a path's)
    let sec = tail.split_whitespace().next().map(|s| s.split(';').next().unwrap_or("")).filter(|s| !s.is_empty() && !s.starts_with('$') && !s.starts_with("//"));
    (file.replace('\\', "/"), sec)
}

fn resolve(file: &str, dir: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(file);
    if p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    let lt = ltspice_sym_dirs().into_iter().filter_map(|d| d.parent().map(|l| [l.join("sub"), l.join("cmp")])).flatten();
    dir.map(|d| d.to_path_buf()).into_iter().chain(lt).map(|d| d.join(file)).find(|p| p.is_file())
}

struct Expander<'a> {
    warnings: &'a mut Vec<String>,
    out: String,
    /// (subcircuit, file, section) already inlined: repeated includes and include cycles are
    /// skipped; a block's subcircuit has its own (a file its sheet includes is inlined there)
    seen: HashSet<(Option<String>, std::path::PathBuf, Option<String>)>,
    /// the deck's own .subckt the lines are in (the outermost), and how deep
    scope: (Option<String>, usize),
    /// false: only check that included files exist.
    read: bool,
    /// name of the D-type `.model` card being read (its `+` lines continue it)
    diode: Option<String>,
    /// behavioural diode models seen; warned about only if a diode uses them
    behavioural: Vec<String>,
}

impl Expander<'_> {
    /// Diode element lines (`D<name> a k MODEL ...`) in the expanded deck decide which of the
    /// behavioural models matter.
    fn warn_behavioural(&mut self) {
        // logical lines: trimmed (subckt bodies are often indented), `+` continuations joined (over comments)
        let mut logical: Vec<String> = vec![];
        for l in self.out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('*')) {
            match (l.strip_prefix('+'), logical.last_mut()) {
                (Some(cont), Some(last)) => (last.push(' '), last.push_str(cont)).1,
                _ => logical.push(l.to_string()),
            }
        }
        let used: HashSet<String> = logical.iter().filter(|l| l.starts_with(['D', 'd'])).filter_map(|l| l.split_whitespace().nth(3)).map(str::to_lowercase).collect();
        for m in self.behavioural.iter().filter(|m| used.contains(*m)) {
            self.warnings.push(format!("model {m}: LTspice behavioural diode (Ron/Roff/Vfwd/Vrev) needs ngspice's XSPICE code models; without them the circuit does not parse"));
        }
    }

    fn expand(&mut self, text: &str, dir: Option<&std::path::Path>, section: Option<&str>, depth: usize) {
        let (mut in_section, mut control, mut found) = (section.is_none(), false, section.is_none());
        let sec_ascii = section.map(ascii);
        for (n, raw) in text.lines().enumerate() {
            // the line as it goes out, and as every check below reads it (one of the line before
            // would let `Ω.control` through), a comment line starred
            let asc = ascii(raw);
            let line = starred(&asc).map_or(asc, std::borrow::Cow::Owned);
            // (the deck's first line, its title: a comment; ngspice reads an .include there too, and
            // a `*ng_script` there makes it run the deck's lines as commands)
            let cmd = command_line(&line);
            let title = depth == 0 && n == 0 && (cmd || !line.trim_start().starts_with('*'));
            if title && (cmd && said(&line) || opens_file(&line)) {
                self.warnings.push(format!("`{}`: the deck's first line is its title, not read", line.trim()));
            }
            let line = if title { std::borrow::Cow::Owned(format!("* {}", line.trim())) } else { line };
            let cmd = cmd && !title; // (made a comment)
            let t = line.trim();
            let l = t.to_ascii_lowercase();
            let tok: Vec<&str> = t.split_whitespace().collect();
            if let Some(sec) = &sec_ascii {
                if !in_section {
                    // (a marker's words without a comment, `.lib TT ; typical`: it holds no path)
                    in_section = l.starts_with(".lib") && matches!(uncommented(t).split_whitespace().collect::<Vec<_>>()[..], [_, m] if m.eq_ignore_ascii_case(sec));
                    found |= in_section;
                    continue;
                }
                if l.starts_with(".endl") {
                    in_section = false;
                    continue;
                }
            }
            if l.starts_with(".control") {
                control = true;
                self.warnings.push(".control blocks are not executed (security)".into());
                continue;
            }
            if control || l.starts_with(".endc") || l.starts_with(".endl") || (depth > 0 && is_end(&l)) {
                control &= !l.starts_with(".endc");
                continue;
            }
            if cmd {
                if said(t) {
                    self.warnings.push(format!("`{t}`: not passed to ngspice, which would take it for commands (security)"));
                }
                continue;
            }
            if depth == 0 && tok.first().is_some_and(|w| w.eq_ignore_ascii_case(".subckt")) {
                self.scope = (self.scope.0.take().or_else(|| tok.get(1).map(|s| s.to_lowercase())), self.scope.1 + 1);
            }
            if depth == 0 && tok.first().is_some_and(|w| w.eq_ignore_ascii_case(".ends")) {
                self.scope.1 = self.scope.1.saturating_sub(1);
                if self.scope.1 == 0 {
                    self.scope.0 = None;
                }
            }
            if opens_file(&l) {
                self.include(raw.trim(), &l, dir, depth); // (the file's name as written: non-ASCII letters too)
                continue;
            }
            // LTspice's behavioural diode (Ron/Roff/Vfwd/Vrev) becomes an XSPICE device in ngspice:
            // without its code models loaded the whole deck fails to parse, so say why up front
            // (comment and blank lines: neither a model nor its + lines; an inline comment no parameter)
            let code = uncommented(&l);
            if !code.trim().is_empty() && !l.starts_with('*') {
                if !l.starts_with('+') {
                    let t: Vec<&str> = l.split(|c: char| c.is_whitespace() || c == '(').filter(|s| !s.is_empty()).collect();
                    self.diode = (t.first() == Some(&".model") && t.get(2) == Some(&"d")).then(|| t[1].to_string());
                }
                let flat = || code.split_whitespace().collect::<String>();
                if let Some(m) = self.diode.as_ref().filter(|_| { let f = flat(); ["ron=", "roff=", "vfwd=", "vrev="].iter().any(|p| f.contains(p)) }) {
                    self.behavioural.push(m.clone());
                    self.diode = None; // once per model
                }
            }
            self.out.push_str(&line);
            self.out.push('\n');
        }
        if !found {
            self.warnings.push(format!("library section `{}` not found", section.unwrap_or_default()));
        }
    }

    fn include(&mut self, t: &str, l: &str, dir: Option<&std::path::Path>, depth: usize) {
        let (file, sec) = include_args(t);
        let sec = sec.filter(|_| l.starts_with(".lib"));
        // LTspice's own opamp.sub needs LTspice semantics; the built-in `opamp` subckt replaces it
        if file.eq_ignore_ascii_case("opamp.sub") && !dir.is_some_and(|d| d.join(&file).is_file()) {
            self.warnings.push("`opamp.sub`: using nomospice's built-in opamp model".into());
            return;
        }
        match resolve(&file, dir) {
            Some(_) if !self.read => {}
            Some(path) => {
                let key = (self.scope.0.clone(), path.canonicalize().unwrap_or(path.clone()), sec.map(str::to_lowercase));
                // (one the deck includes outside any subcircuit: global already, not inlined again)
                if self.seen.contains(&(None, key.1.clone(), key.2.clone())) || !self.seen.insert(key) {
                    return; // already inlined (or an include cycle)
                }
                match std::fs::read(&path) {
                    Ok(bytes) => self.expand(&decode(&bytes), path.parent(), sec, depth + 1),
                    Err(e) => self.warnings.push(format!("{}: {e}", path.display())),
                }
            }
            // inside an included file, `.lib name` without a file is a section marker
            None if depth > 0 && l.starts_with(".lib") && sec.is_none() => {}
            None => self.warnings.push(format!("`{t}`: file not found")),
        }
    }
}

/// LTspice instance parameters ngspice does not know. Rser/Lser/Rpar/Cpar on C, L and V become
/// real elements (see `parasitic_lines`); datasheet metadata is dropped. Returns the rest.
fn lt_instance_params(prefix: char, inst: &str, extra: &str, warnings: &mut Vec<String>) -> (String, Vec<(String, String)>) {
    // datasheet fields LTspice's component database writes on primitives; on an X instance the
    // same names are the user's own subckt parameters and must stay
    const META: [&str; 11] = ["tol", "pwr", "v", "irms", "ipk", "mfg", "pn", "type", "vpk", "iave", "rlshunt"];
    let (mut kept, mut para, mut dropped) = (vec![], vec![], vec![]);
    for t in spice_tokens(extra) {
        let kv = t.split_once('=').map(|(k, v)| (k.trim().to_lowercase(), v.trim()));
        match kv {
            Some((k, v)) if "CLV".contains(prefix) && ["rser", "lser", "rpar", "cpar"].contains(&k.as_str()) => para.push((k, v.to_string())),
            Some((k, _)) if "RCLVID".contains(prefix) && META.contains(&k.as_str()) => dropped.push(k),
            _ => kept.push(t),
        }
    }
    if !dropped.is_empty() {
        warnings.push(format!("{inst}: LTspice-only parameters ignored: {}", dropped.join(", ")));
    }
    // the same key in Value and SpiceLine: the later one wins (one element per kind)
    let para: Vec<(String, String)> = para.iter().enumerate().filter(|(i, (k, _))| !para[i + 1..].iter().any(|(k2, _)| k2 == k)).map(|(_, p)| p.clone()).collect();
    (kept.join(" "), para)
}

/// Series parts go between the first pin and the device (whose first node is rewired), parallel
/// parts across both pins: `pin1 -Rser- -Lser- [device] pin2`, `Rpar`/`Cpar` from pin1 to pin2.
fn parasitic_lines(inst: &str, nodes: &mut [String], para: &[(String, String)]) -> Vec<String> {
    let (a, b) = (nodes[0].clone(), nodes[1].clone());
    let mut out = vec![];
    let mut at = a.clone();
    for (k, v) in para {
        let zero = spice_num(v) == Some(0.0);
        let letter = k[..1].to_uppercase(); // rser/rpar -> R, lser -> L, cpar -> C
        match k.as_str() {
            _ if zero => {}
            "rser" | "lser" => {
                let next = format!("{inst}_{k}");
                out.push(format!("{letter}{inst}_{k} {at} {next} {v}"));
                at = next;
            }
            _ => out.push(format!("{letter}{inst}_{k} {a} {b} {v}")),
        }
    }
    nodes[0] = at;
    out
}

fn jumper_root(parent: &mut HashMap<String, String>, net: &str) -> String {
    let mut n = net.to_lowercase();
    while let Some(p) = parent.get(&n) {
        n = p.clone();
    }
    n
}

/// The netlist as the user writes it (shown in the UI).
pub fn netlist(items: &[Item], lib: &mut Library) -> Netlist {
    build_netlist(items, lib, false, &mut Hier::default())
}

/// The netlist that is simulated: every probe-able device gets 0 V current sensors: two-pin parts
/// (R C L D) `Vsense_<inst>` at pin 1, transistors (Q M J) and subcircuits (X) one at every pin
/// (`Vsense_<inst>_2` …), so `I(R1)`, `Ib(Q1)` … are sensor currents in every analysis: with phase
/// in .ac (where ngspice, also with `.options savecurrents`, leaves @r[i]/@c[i]/@q[ie] empty and
/// reports DC currents for D/Q/M), including ESR/parasitics, and following value changes.
pub fn netlist_for_sim(items: &[Item], lib: &mut Library) -> Netlist {
    build_netlist(items, lib, true, &mut Hier::default())
}

/// Prefixes that get current sensors (see `netlist_for_sim`).
/// (L too: its native branch current misses Rpar/Cpar from the LTspice database.)
pub const SENSED: &str = "RCLDQMJX";

/// Directives a block's sheet keeps (definitions); its analyses, .meas, .options … are the top
/// schematic's business, as in LTspice (in a sheet they are its own test bench).
const SHEET_KEEPS: [&str; 13] = [".param", ".func", ".function", ".model", ".subckt", ".ends", ".include", ".inc", ".lib", ".endl", ".ic", ".nodeset", ".global"];

/// `h.stack`: the blocks being netlisted, outermost first (empty: the top schematic; else a sheet's
/// body, a subcircuit's: no title, analyses or `.end`).
fn build_netlist(items: &[Item], lib: &mut Library, sense: bool, h: &mut Hier) -> Netlist {
    let sub = !h.stack.is_empty();
    let mut nl = Netlist { nets: vec![vec![]; items.len()], insts: vec![String::new(); items.len()], probes: vec![String::new(); items.len()], pin_currents: vec![vec![]; items.len()], sheets: vec![String::new(); items.len()], ..Default::default() };

    // 1. connection points
    let mut ids: HashMap<(i32, i32), usize> = HashMap::new();
    let id = |p: (i32, i32), ids: &mut HashMap<(i32, i32), usize>| {
        let n = ids.len();
        *ids.entry(p).or_insert(n)
    };
    let mut pins: Vec<Vec<usize>> = vec![vec![]; items.len()]; // point ids per item
    let mut segs = vec![];
    for (i, it) in items.iter().enumerate() {
        match it {
            Item::Wire { x1, y1, x2, y2 } => {
                pins[i] = vec![id((*x1, *y1), &mut ids), id((*x2, *y2), &mut ids)];
                segs.push(((*x1, *y1), (*x2, *y2)));
            }
            Item::Flag { x, y, .. } => pins[i] = vec![id((*x, *y), &mut ids)],
            Item::Sym { sym, x, y, rot, .. } => match lib.get(sym) {
                Some(def) => {
                    pins[i] = def.pins.iter().map(|p| {
                        let (dx, dy) = xform(rot, p.x, p.y);
                        id((x + dx, y + dy), &mut ids)
                    }).collect();

                }
                None => {
                    let w = format!("symbol `{sym}` not found: its parts are left out (its .asy is not next to the schematic, in LTspice or a library folder)");
                    if !nl.warnings.contains(&w) {
                        nl.warnings.push(w);
                    }
                }
            },
            Item::Text { .. } | Item::Draw { .. } | Item::Raw { .. } => {}
        }
    }

    // 2. union: wire ends, T-junctions, equal flag names
    let mut dsu = Dsu((0..ids.len()).collect());
    for (i, it) in items.iter().enumerate() {
        if let Item::Wire { .. } = it {
            dsu.union(pins[i][0], pins[i][1]);
        }
    }

    let pts: Vec<((i32, i32), usize)> = ids.iter().map(|(p, i)| (*p, *i)).collect();
    for &(p, pid) in &pts {
        for &(a, b) in &segs {
            if inside(p, a, b) {
                dsu.union(pid, ids[&a]);
            }
        }
    }
    let mut named: HashMap<String, usize> = HashMap::new();
    let mut ground = None;
    for (i, it) in items.iter().enumerate() {
        if let Item::Flag { name, .. } = it {
            if name.trim().is_empty() {
                continue;
            }
            let key = if name == "0" || name.eq_ignore_ascii_case("gnd") { "0".into() } else { name.to_lowercase() };
            if key == "0" {
                ground = Some(pins[i][0]);
            }
            match named.get(&key) {
                Some(&other) => dsu.union(pins[i][0], other),
                None => { named.insert(key, pins[i][0]); }
            }
        }
    }

    // 3. net names: ground, labels, then N001.. in symbol order. Names are unique
    //    case-insensitively (SPICE is), so an auto name can never short a labelled net.
    let mut names: HashMap<usize, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::from(["0".to_string()]);
    let claim = |base: String, used: &mut HashSet<String>| {
        let mut name = base.clone();
        for k in 2.. {
            if used.insert(name.to_lowercase()) {
                break;
            }
            name = format!("{base}_{k}");
        }
        name
    };
    if let Some(g) = ground {
        names.insert(dsu.find(g), "0".into());
    }
    for it in items.iter().zip(&pins) {
        if let (Item::Flag { name, .. }, p) = it {
            let r = dsu.find(p[0]);
            if !name.trim().is_empty() && !names.contains_key(&r) {
                names.insert(r, claim(net_name(name), &mut used));
            }
        }
    }
    let mut counter = 0;
    let order: Vec<usize> = items.iter().enumerate()
        .filter(|(_, it)| matches!(it, Item::Sym { .. }))
        .chain(items.iter().enumerate().filter(|(_, it)| !matches!(it, Item::Sym { .. })))
        .map(|(i, _)| i).collect();
    for &i in &order {
        for &p in &pins[i] {
            let r = dsu.find(p);
            if !names.contains_key(&r) {
                let name = loop {
                    counter += 1;
                    let n = format!("N{counter:03}");
                    if !used.contains(&n.to_lowercase()) {
                        break claim(n, &mut used);
                    }
                };
                names.insert(r, name);
            }
        }
    }
    for (i, p) in pins.iter().enumerate() {
        nl.nets[i] = p.iter().map(|&p| names[&dsu.find(p)].clone()).collect();
    }
    // (a schematic of no parts, an opened netlist: its lines ground it or not, ngspice says; a part
    // line in a directive with a node 0 grounds the drawing too)
    // (not inside a .subckt: its 0 grounds what calls it, not the drawing)
    let grounded_by_lines = || items.iter().any(|it| matches!(it, Item::Text { text, directive: true, .. } if {
        let mut sub = 0;
        text.lines().any(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            match t.first().map(|w| w.to_ascii_lowercase()).as_deref() {
                Some(".subckt") => sub += 1,
                Some(".ends") => sub = (sub - 1).max(0),
                _ => {}
            }
            sub == 0 && t.len() >= 3 && t[0].starts_with(|c: char| c.is_ascii_alphabetic()) && t[1..t.len() - 1].contains(&"0")
        })
    }));
    // (said once the blocks are netlisted: node 0 in a sheet grounds it all)
    let ungrounded = ground.is_none() && items.iter().any(|it| matches!(it, Item::Sym { .. })) && !grounded_by_lines();
    if sub {
        h.grounded |= ground.is_some() || grounded_by_lines();
    }

    // 4. directives (`.control` blocks are removed later by `prepare`, the single security filter)
    let mut directives = vec![];
    for it in items {
        if let Item::Text { text, directive: true, .. } = it {
            let mut skipped = false; // `+` continuation lines of a skipped directive go too
            // (the last .tran: its index among the directives and its text as written, for its + lines)
            let mut tran: Option<(usize, String)> = None;
            let mut control = false; // a sheet's `.control` block: all of it goes
            let mut stepping = false; // ... of a .step: they belong to it
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let star = starred(line);
                let line = star.as_deref().unwrap_or(line);
                let l = line.to_lowercase();
                if sub {
                    // (a sheet: its definitions and parts; its commands with their `+` lines go)
                    let head = l.split_whitespace().next().unwrap_or("");
                    let more = head.starts_with('+');
                    control = (control || head == ".control") && head != ".endc";
                    if head.starts_with('*') {
                        continue; // (a comment: no use in its subcircuit, and a dropped command's `+` lines still go)
                    }
                    // (its analyses, .meas, .options …: the sheet's own test bench, unsaid)
                    if control || if more { skipped } else { head.starts_with('.') && !SHEET_KEEPS.contains(&head) } {
                        skipped = true;
                        continue;
                    }
                }
                // (each line without its inline comment: a `;` must not swallow the next line's values;
                // comment lines between a .step and its + lines skipped, as SPICE does)
                if stepping && l.starts_with('*') {
                    directives.push(line.to_string()); // (kept, as a comment)
                    continue;
                }
                if stepping && l.starts_with('+') {
                    if let Some(s) = nl.steps.last_mut() {
                        s.push(' ');
                        s.push_str(uncommented(line)[1..].trim());
                    }
                    continue;
                }
                stepping = l.starts_with(".step") && (l.len() == 5 || l[5..].starts_with(char::is_whitespace));
                if stepping {
                    nl.steps.push(uncommented(line).trim_end().to_string());
                    continue;
                }
                if skipped && l.starts_with('+') || is_end(&l) {
                    continue;
                }
                // (a statement's + lines, comments between allowed: a .net's onto its comment, before
                // the mark, not onto the line before; a .tran's read with it, `+ startup` too)
                let last = directives.iter().rposition(|d| !d.trim_start().starts_with('*') || d.ends_with(NET_MARK));
                if l.starts_with('+') {
                    if let Some(k) = last.filter(|&k| directives[k].ends_with(NET_MARK)) {
                        let d = &mut directives[k];
                        d.truncate(d.len() - NET_MARK.len());
                        d.push_str(&format!("{} {NET_MARK}", uncommented(line)[1..].trim()));
                        continue;
                    }
                    if let Some((k, raw)) = tran.as_mut().filter(|t| Some(t.0) == last) {
                        raw.push(' ');
                        raw.push_str(uncommented(line)[1..].trim());
                        // (read again whole: its warnings once)
                        let mut w = vec![];
                        if let Some(d) = directive(raw, &mut w) {
                            directives[*k] = d;
                        }
                        for m in w {
                            if !nl.warnings.contains(&m) {
                                nl.warnings.push(m);
                            }
                        }
                        continue;
                    }
                }
                let d = directive(line, &mut nl.warnings);
                // (a comment between leaves the + lines with their statement)
                if !l.starts_with('*') {
                    skipped = d.is_none();
                    tran = (l.starts_with(".tran") && d.is_some()).then(|| (directives.len(), uncommented(line).trim_end().to_string()));
                }
                directives.extend(d);
            }
        }
    }
    // a .step that will not run is said before a run; the netlist shows them (nomospice runs them)
    nl.warnings.extend(nl.steps.iter().filter_map(|s| parse_step(s).err()));
    directives.extend(nl.steps.iter().map(|s| format!("* {s} (nomospice: one run per step)")));
    // (the names the directives define, once: looked up per part, a big netlist's thousands too)
    let mut defs: HashSet<String> = directives.iter().filter_map(|d| {
        let t: Vec<String> = d.split_whitespace().take(2).map(str::to_lowercase).collect();
        matches!(t.first().map(String::as_str), Some(".model" | ".subckt")).then(|| t.get(1).cloned()).flatten()
    }).collect();
    // (the top's models and includes are global: its sheets use them as their own)
    let include = |d: &String| opens_file(d);
    if sub {
        defs.extend(h.top_defs.iter().cloned());
    } else {
        // (not those inside a .subckt of the top's own: local to it)
        let mut depth = 0;
        let global = directives.iter().filter(|d| {
            let w = d.split_whitespace().next().unwrap_or("").to_lowercase();
            depth += i32::from(w == ".subckt");
            let top = depth == 0;
            depth = (depth - i32::from(w == ".ends")).max(0);
            top || (depth == 1 && w == ".subckt") // (a .subckt's own name is global)
        });
        h.top_defs = global.filter_map(|d| {
            let t: Vec<String> = d.split_whitespace().take(2).map(str::to_lowercase).collect();
            matches!(t.first().map(String::as_str), Some(".model" | ".subckt")).then(|| t.get(1).cloned()).flatten()
        }).collect();
        h.top_includes = directives.iter().any(include);
    }
    let defined = |name: &str| defs.contains(name);
    // LTspice runs .noise without an AC value on the input source; ngspice refuses: add `AC 1`
    let noise_src: Vec<String> = directives.iter().filter_map(|d| {
        let t = spice_tokens(d);
        (t.first()?.eq_ignore_ascii_case(".noise")).then(|| t.get(2).map(|s| s.to_uppercase()))?
    }).collect();
    let vdmos: Vec<String> = directives.iter().filter_map(|d| {
        let t: Vec<String> = d.split(|c: char| c.is_whitespace() || c == '(').filter(|s| !s.is_empty()).map(str::to_lowercase).collect();
        (t.first()? == ".model" && t.get(2)? == "vdmos").then(|| t[1].clone())
    }).collect();

    // 5. element lines

    let mut lines = if sub { vec![] } else { vec!["* nomospice netlist".to_string()] };
    let mut used_models = vec![];
    // (models of nomospice's own subcircuit symbols placed here: ours, whatever a library has of
    // that name; a library's AND2 placed by its own box symbol stays the library's)
    let mut own_used: HashSet<String> = HashSet::new();
    let one_line = |s: &str| s.replace(['\n', '\r'], " ");
    let spice_inst = |inst: &str, prefix: char| {
        let inst = net_name(inst);
        if inst.to_uppercase().starts_with(prefix) { inst } else { format!("{prefix}{inst}") }
    };
    // the part's own Prefix (LTspice's attribute editor: X to call a subcircuit) over the symbol's
    let prefix_of = |def: &SymbolDef, attrs: &BTreeMap<String, String>| {
        attrs.get("Prefix").filter(|p| !p.trim().is_empty()).unwrap_or(&def.prefix).trim().chars().next().unwrap_or('X').to_ascii_uppercase()
    };
    let mut taken: HashSet<String> = items.iter().filter_map(|it| match it {
        Item::Sym { sym, attrs, .. } => {
            let p = prefix_of(lib.get(sym)?, attrs);
            attrs.get("InstName").filter(|n| !n.is_empty()).map(|n| spice_inst(n, p).to_uppercase())
        }
        _ => None,
    }).collect();
    // parts without a name get theirs now (R1, R2 … not yet taken), so every name is known before
    // current names are given (a part drawn as V1 next to an unnamed source that becomes V1)
    let auto: HashMap<usize, String> = items.iter().enumerate().filter_map(|(i, it)| match it {
        Item::Sym { sym, attrs, .. } if attrs.get("InstName").is_none_or(|n| n.is_empty()) => {
            let p = prefix_of(lib.get(sym)?, attrs);
            Some((i, (1..).map(|n| format!("{p}{n}")).find(|n| taken.insert(n.to_uppercase())).unwrap()))
        }
        _ => None,
    }).collect();
    let mut seen = HashSet::new();
    let mut named: HashSet<String> = HashSet::new(); // current names given so far (lowercase)
    // nets that reach a part other than a subcircuit: there a subcircuit pin's sensor node is held by
    // the outside; elsewhere (an open NC pin, ports of ICs only) it could float with a port the
    // subcircuit leaves unused (singular matrix) and gets a 1 TΩ hold to ground
    let held: HashSet<&str> = items.iter().zip(&nl.nets).filter(|(it, _)| match it {
        Item::Sym { sym, attrs, .. } => lib.get(sym).is_some_and(|d| prefix_of(d, attrs) != 'X'),
        _ => false,
    }).flat_map(|(_, nets)| nets.iter().map(String::as_str)).collect();
    // drawn names more than one part has (T1 a diode and T1 a resistor): those parts go by their netlist names
    let mut drawn: HashMap<String, usize> = HashMap::new();
    for it in items {
        if let Item::Sym { attrs, .. } = it {
            if let Some(n) = attrs.get("InstName").filter(|n| !n.trim().is_empty()) {
                *drawn.entry(net_name(n).to_uppercase()).or_default() += 1;
            }
        }
    }
    let mut jumped: HashMap<String, String> = HashMap::new(); // nets already tied by jumpers
    let mut any_ac = false; // some V/I source has an AC amplitude (else .ac results are all 0)
    for (i, it) in items.iter().enumerate() {
        let Item::Sym { sym, attrs, .. } = it else { continue };
        let Some(def) = lib.get(sym).cloned() else { continue }; // (said in step 1)

        let prefix = prefix_of(&def, attrs);
        let inst = match attrs.get("InstName").filter(|n| !n.is_empty()) {
            Some(n) => spice_inst(n, prefix),
            None => auto[&i].clone(), // (named up front)
        };
        if !seen.insert(inst.to_uppercase()) {
            nl.warnings.push(format!("duplicate instance name {inst}"));
        }
        if sym_key(sym) == "jumper" && nl.nets[i].len() == 2 {
            // LTspice MISC\JUMPER: one net, two names. A 0 V source keeps both names in the netlist;
            // jumpers that would short a net to itself (or close a loop of jumpers) are left out.
            let (a, b) = (jumper_root(&mut jumped, &nl.nets[i][0]), jumper_root(&mut jumped, &nl.nets[i][1]));
            let mut name = format!("V{inst}");
            while !taken.insert(name.to_uppercase()) {
                name += "_j"; // a user's own source may already be called V<inst>
            }
            if a != b {
                jumped.insert(a, b);
                lines.push(format!("{name} {} {} 0", nl.nets[i][0], nl.nets[i][1]));
            }
            nl.insts[i] = name;
            continue;
        }
        // a hierarchical block: its sheet's subcircuit, named as the symbol, its Value's parameters
        // passed; a sheet that is none here (said, or none at all): a subcircuit of that name
        let written = attrs.get("Value").unwrap_or(&def.value);
        let call = (prefix == 'X' && (def.sheet.is_some() || (def.block && !def.model_file))).then(|| block_params(written, &def.name)).flatten();
        let name = if call.is_some() { h.name(&def.name) } else { String::new() };
        if let (Some(_), Some(path)) = (&call, &def.sheet) {
            if !h.blocks.iter().any(|b| b.name == name) && !h.refused.contains_key(&name) {
                sheet_subckt(&name, path, &def, lib, h, &mut nl.warnings);
            }
        }
        let block = call.filter(|_| h.blocks.iter().any(|b| b.name == name)).map(|params| (name.clone(), params.to_string()));
        nl.sheets[i] = block.as_ref().and(def.sheet.clone()).unwrap_or_default();
        let value = match (&block, call) {
            (Some((name, params)), _) => one_line(&format!("{name} {params}")),
            // (a block symbol that is no subcircuit here, its Value naming none: a subcircuit of its name)
            (None, Some(params)) if params == written.trim() => one_line(&format!("{} {params}", def.name)),
            _ => one_line(written).replace("SINE(", "SIN("),
        };
        let mut nodes = nl.nets[i].clone();
        let npins = nodes.len(); // the symbol's (an M gets a bulk node below)
        // W (current-controlled switch): `Vcontrol model [on|off]`, the model comes last
        let model = if prefix == 'W' {
            let t: Vec<&str> = value.split_whitespace().filter(|t| !t.eq_ignore_ascii_case("on") && !t.eq_ignore_ascii_case("off")).collect();
            if t.len() < 2 {
                nl.warnings.push(format!("{inst}: a current-controlled switch needs its controlling source in Value, e.g. `V1 {}`", t.first().unwrap_or(&"CSW")));
            }
            t.last().unwrap_or(&"").to_lowercase()
        } else {
            value.split(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("").to_lowercase()
        };
        // bulk node only for 4-terminal MOS models; a VDMOS (directive or library) has 3 terminals
        let lib_vdmos = || crate::library::index().as_ref().and_then(|ix| ix.get(&model).map(|e| e.kind == "VDMOS")).unwrap_or(false);
        if prefix == 'M' && nodes.len() == 3 && !vdmos.contains(&model) && !lib_vdmos() {
            nodes.push(nodes[2].clone()); // bulk tied to source
        }
        // (a BLOCK symbol whose sheet is none here, said: a library may have its name, else the
        // refusal is what is said, not a missing model)
        if call.is_some() && def.block && block.is_none() && h.refused.get(&name).is_some_and(|w| !w.is_empty()) {
            h.quiet.insert(model.clone());
        }
        // (nomospice's own subcircuit symbols: their default parameters belong to their own model,
        // opamp2's Slew=… are no TL072's; any other symbol's apply whatever its Value, as in LTspice)
        let defaults = !(builtin_symbol(&def.name) && own_model(&def.value.to_lowercase())) || def.value.eq_ignore_ascii_case(&model);
        if builtin_symbol(&def.name) && own_model(&model) {
            own_used.insert(model.clone());
        }
        if "DQMXJS".contains(prefix) && model.is_empty() {
            nl.warnings.push(format!("{inst}: no model or subcircuit named (its Value is empty)"));
        } else if "DQMXJSWO".contains(prefix) && !model.is_empty() && block.is_none() && !defined(&model) && !used_models.contains(&model) {
            used_models.push(model);
        }
        let extra: Vec<String> = ["Value2", "SpiceLine", "SpiceLine2"].iter()
            .filter_map(|k| attrs.get(*k).cloned().or_else(|| (*k == "SpiceLine" && defaults).then(|| {
                // (a parameter given in Value is not followed by its default: ngspice takes the last)
                let mut written = format!("{value} {}", attrs.get("Value2").map_or("", String::as_str)); // (Value2's too)
                while written.contains(" =") || written.contains("= ") {
                    written = written.replace(" =", "=").replace("= ", "="); // (`Slew = 1Meg` is one)
                }
                let given: Vec<String> = spice_tokens(&written).iter().filter_map(|t| t.split_once('=').map(|(k, _)| k.to_ascii_lowercase())).collect();
                spice_tokens(&def.spice_line).into_iter().filter(|t| !t.split_once('=').is_some_and(|(k, _)| given.contains(&k.to_ascii_lowercase()))).collect::<Vec<_>>().join(" ")
            })))
            .filter(|s| !s.is_empty()).map(|s| one_line(&s)).collect();
        // LTspice accepts Rser=... in Value too: value and extras go through one pass
        let (rest, parasitics) = lt_instance_params(prefix, &inst, &format!("{value} {}", extra.join(" ")), &mut nl.warnings);
        // (a block's: the parameters it passes, declared on the subcircuit; the others it gets from
        // its caller where its sheet has none, filled in once all its instances are known)
        let tag = block.as_ref().and_then(|(name, _)| {
            let low = rest.to_ascii_lowercase();
            let passed: Vec<String> = param_defs(&rest, &low).into_iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
            h.blocks.iter_mut().find(|b| b.name == *name)?.params.extend(passed.iter().cloned());
            Some(format!("{BLOCK_TAG}{name}{BLOCK_TAG}{}", passed.join(" ")))
        });
        nl.probes[i] = match prefix {
            'V' | 'E' | 'H' => format!("i({})", inst.to_lowercase()),
            _ => String::new(),
        };
        // named as drawn (InstName U1, not the netlist's XU1: `Ix(U1:OUT)`), unless another part
        // already has such a name (a diode D1 beside a subcircuit drawn as D1): then the netlist's
        let shown = attrs.get("InstName").map(|n| net_name(n)).filter(|n| !n.is_empty()).unwrap_or_else(|| inst.clone());
        let pins_of = &def.pins[..npins.min(def.pins.len())];
        let aliases = pin_labels(prefix, &inst, pins_of);
        let mut called = shown.clone();
        let mut labels = pin_labels(prefix, &shown, pins_of);
        let other = !shown.eq_ignore_ascii_case(&inst) && (taken.contains(&shown.to_uppercase()) || drawn.get(&shown.to_uppercase()).is_some_and(|&n| n > 1)); // (another part's name)
        if other || labels.iter().any(|l| named.contains(&l.to_lowercase())) {
            // (the netlist's name; still taken, e.g. two parts drawn alike: made unique like the sensors)
            called = inst.clone();
            labels = pin_labels(prefix, &called, pins_of);
            while labels.iter().any(|l| named.contains(&l.to_lowercase())) {
                called.push('_');
                labels = pin_labels(prefix, &called, pins_of);
            }
        }
        // ponytail: a sensor on every device, on every pin of transistors and subcircuits (+1 node,
        // +1 branch each: an imported board of 100 ICs gains ~1500 rows); sense only probed devices
        // or pins if huge imported schematics get slow
        // (two-pin parts: pin 1; transistors and subcircuits: every pin of the symbol, not an M's
        // added bulk, which stays on the source's own net)
        let sensed = if !SENSED.contains(prefix) || npins == 0 { 0..0 } else if npins <= 2 || !"QMJX".contains(prefix) { 0..1 } else { 0..npins };
        nl.pin_currents[i] = vec![String::new(); npins];
        if !nl.probes[i].is_empty() && npins > 0 {
            nl.pin_currents[i][0] = format!("I({inst})"); // a source's own branch current (ngspice: i(v1), by the netlist's name)
        }
        for k in sensed {
            // reserved in both builds (identical names), never clashing with the user's own
            let tail = if k == 0 { String::new() } else { format!("_{}", k + 1) };
            let mut src = format!("Vsense_{inst}{tail}");
            while !taken.insert(src.to_uppercase()) {
                src.push('_');
            }
            let mut node = format!("{inst}_sense{tail}");
            while !used.insert(node.to_lowercase()) {
                node.push('_');
            }
            let label = labels[k].clone();
            if k == 0 && !(prefix == 'X' && npins > 2) {
                nl.probes[i] = format!("i({})", src.to_lowercase()); // (a subcircuit's pins are its currents)
            }
            nl.pin_currents[i][k] = label.clone();
            named.insert(label.to_lowercase());
            // (1 TΩ, ngspice's own gmin: pA at 1 V, only where nothing else holds the net)
            let hold = if prefix == 'X' && nl.nets[i][k] != "0" && !held.contains(nl.nets[i][k].as_str()) {
                let mut r = format!("Rsense_{inst}{tail}");
                while !taken.insert(r.to_uppercase()) {
                    r.push('_');
                }
                r
            } else {
                String::new()
            };
            nl.sensors.push(Sensor { source: src.to_lowercase(), node: node.to_lowercase(), inst: inst.to_lowercase(), pin: k, label, name: called.to_lowercase(), hold: hold.to_lowercase(), alias: aliases[k].to_lowercase() });
            if sense {
                lines.push(format!("{src} {} {node} 0", nodes[k]));
                if !hold.is_empty() {
                    // (the net's side: not in the measured current; noiseless: no .noise contribution)
                    lines.push(format!("{hold} {} 0 1T noisy=0", nodes[k]));
                }
                if prefix == 'M' && k == 2 && npins == 3 && nodes.len() == 4 {
                    nodes[3] = node.clone(); // the bulk added for a 3-pin symbol is part of its source pin
                }
                nodes[k] = node;
            }
        }
        if !parasitics.is_empty() && nodes.len() == 2 {
            lines.extend(parasitic_lines(&inst, &mut nodes, &parasitics));
        }
        // LTspice D ... N=4: four diodes in series (LED strings); ngspice has no such parameter
        let (rest, series) = match prefix {
            'D' => {
                let toks = spice_tokens(&rest);
                let n = toks.iter().find_map(|t| t.to_lowercase().strip_prefix("n=").map(str::to_string));
                let count = n.as_deref().and_then(|v| spice_num(v).filter(|f| *f >= 1.0 && f.fract() == 0.0).map(|f| f as usize));
                if let (Some(v), None) = (&n, count) {
                    nl.warnings.push(format!("{inst}: N={v} needs a whole number (no parameters); using one diode"));
                }
                (toks.iter().filter(|t| !t.to_lowercase().starts_with("n=")).copied().collect::<Vec<_>>().join(" "), count.unwrap_or(1))
            }
            _ => (rest, 1),
        };
        if series > 1 && nodes.len() == 2 {
            let mid = |k: usize| if k == series { nodes[1].clone() } else { format!("{inst}_n{k}") };
            for k in 2..=series {
                lines.push(format!("{inst}_{k} {} {} {rest}", mid(k - 1), mid(k)));
            }
            nodes[1] = mid(1);
        }
        let mut line = format!("{inst} {} {rest}", nodes.join(" ")).trim_end().to_string();
        // look for `AC` only after instance + node names (a net may well be called "ac")
        let has_ac = spice_tokens(&line).iter().skip(1 + nodes.len()).any(|t| t.eq_ignore_ascii_case("ac"));
        any_ac |= has_ac && matches!(prefix, 'V' | 'I');
        if noise_src.contains(&inst.to_uppercase()) && !has_ac {
            line += " AC 1";
            nl.warnings.push(format!("{inst} has no AC value; using AC 1 for .noise"));
        }
        lines.push(match tag { Some(t) => line + &t, None => line });
        nl.insts[i] = inst;
    }
    let is = |d: &str, k: &str| spice_tokens(d).first().is_some_and(|t| t.eq_ignore_ascii_case(k));
    // sources written as SPICE directives count too (`V1 in 0 AC 1`: AC after the two nodes), and
    // those of the blocks' sheets
    any_ac |= directives.iter().any(|d| d.starts_with(['V', 'v', 'I', 'i']) && spice_tokens(d).iter().skip(3).any(|t| t.eq_ignore_ascii_case("ac")));
    h.ac |= any_ac && sub;
    any_ac |= h.ac;
    if !any_ac && directives.iter().any(|d| is(d, ".ac")) {
        nl.warnings.push(".ac: no source has an AC amplitude, so every result is 0; add e.g. `AC 1` to the input source".into());
    }
    // a netlist's own part lines (an opened .cir, parts written in a directive): their models from
    // the libraries too, as a drawn part's; only names the libraries know (a node, a value: none)
    {
        // (each part line with its `+` lines; where SPICE has the model: D n+ n- model; Q c b e [s]
        // model; M d g s [b] model (a VDMOS has 3 nodes); J/Z d g s model; X …nodes subckt
        // [params]: the last word before them)
        let mut logical: Vec<String> = vec![];
        for d in &directives {
            match (d.trim_start().strip_prefix('+'), logical.last_mut()) {
                (Some(more), Some(last)) => (last.push(' '), last.push_str(more)).1,
                _ => logical.push(d.clone()),
            }
        }
        let wanted: Vec<String> = logical.iter().flat_map(|d| {
            let t: Vec<&str> = d.split_whitespace().collect();
            let words = |at: &[usize]| at.iter().filter_map(|&k| t.get(k)).map(|w| w.to_lowercase()).collect::<Vec<_>>();
            match t.first().and_then(|w| w.chars().next()).map(|c| c.to_ascii_uppercase()) {
                Some('D') => words(&[3]),
                Some('Q') => words(&[4, 5]),
                Some('M') => words(&[4, 5]),
                Some('J' | 'Z') => words(&[4]),
                Some('X') => t[1..].iter().take_while(|w| !w.contains('=') && !w.eq_ignore_ascii_case("params:")).last().map(|w| w.to_lowercase()).into_iter().collect(),
                _ => vec![],
            }
        }).filter(|m| !defined(m) && !used_models.contains(m)).collect();
        // (the library's index only when there is something to look up: building it takes a while)
        if !wanted.is_empty() {
            let index = crate::library::index();
            for m in wanted {
                if index.as_ref().is_some_and(|ix| ix.get(&m).is_some()) && !used_models.contains(&m) {
                    used_models.push(m);
                }
            }
        }
    }
    if !sub {
        lines.push(".param pi=3.141592653589793".into()); // LTspice built-in; a user .param pi comes later and wins
    }
    let includes = directives.iter().any(include) || h.top_includes; // (in a sheet, the top's too: global)
    lines.extend(directives);
    // after the user's directives (ngspice keeps the first definition, so .include/.lib models win):
    // the libraries (LTspice's stock models as LTspice loads them implicitly, Micro-Cap 12, user
    // folders; with their dependencies), else ours. `opamp` is always ours: the symbol's model.
    // (one set for the whole hierarchy: a sheet's go to the top, global, each definition once)
    for m in used_models {
        if h.emitted.contains(&m) {
            continue;
        }
        let builtin = BUILTIN_MODELS.iter().find(|(n, _)| *n == m).map(|(_, t)| t.to_string());
        // (`opamp` is always ours, drawn or in a netlist's text: LTspice's opamp.sub needs LTspice)
        let lib = if own_used.contains(&m) || m == "opamp" { None } else { crate::library::definitions(&m, &mut h.emitted) };
        let def = match (lib, builtin) {
            (Some(defs), _) => Some(defs.join("\n")),
            (None, Some(text)) => h.emitted.insert(m.clone()).then_some(text),
            (None, None) if !includes && !h.emitted.contains(&m) && !h.quiet.contains(&m) => {
                nl.warnings.push(format!("`{m}`: no model or subcircuit of that name in the libraries"));
                None
            }
            _ => None,
        };
        match def {
            Some(d) if sub => h.models.push(d),
            Some(d) => lines.push(d),
            None => {}
        }
    }
    if !sub {
        if ungrounded && !h.grounded {
            nl.warnings.insert(0, "no ground (0) in schematic".into());
        }
        nl.warnings.extend(h.refused.values().filter(|w| !w.is_empty()).cloned());
        for g in std::mem::take(&mut h.hoisted) {
            if !lines.contains(&g) {
                lines.push(g);
            }
        }
        lines.append(&mut h.models);
        // a block's instances pass what one of them passes: the sheet's own .param of it where they
        // do not, else their caller's value of it (a .param above it), as LTspice has it
        // (a tagged line: the line, and the parameters it takes from its caller)
        let more = |l: &str| -> Option<(String, Vec<String>)> {
            let (head, tag) = l.split_once(BLOCK_TAG)?;
            let (name, passed) = tag.split_once(BLOCK_TAG).unwrap_or((tag, ""));
            let passed: Vec<&str> = passed.split_whitespace().collect();
            let b = h.blocks.iter().find(|b| b.name == name);
            Some((head.to_string(), b.map_or(vec![], |b| b.params.iter().filter(|p| !b.own.contains(*p) && !passed.contains(&p.as_str())).cloned().collect())))
        };
        let fill = |l: &String| match more(l) {
            None => l.clone(),
            Some((head, ps)) => head + &ps.iter().map(|p| format!(" {p}={{{p}}}")).collect::<String>(),
        };
        if !h.blocks.is_empty() {
            // the top's parts take them from the top's .params (or .step): one it lacks stops ngspice, said
            if !includes {
                let text = lines.join("\n");
                let mut top: HashSet<String> = deck_lines(&text).iter().filter(|l| l.top && l.param())
                    .flat_map(|l| param_defs(l.line, &l.low).into_iter().map(|(n, _)| n.to_ascii_lowercase())).collect();
                top.extend(step_params(&nl.steps).iter().map(|p| p.to_ascii_lowercase()));
                // (a part in a sheet: from that sheet's .params and those it is given, or the top's)
                let scopes = std::iter::once((&lines, None)).chain(h.blocks.iter().map(|b| (&b.body, Some(b))));
                for (body, b) in scopes {
                    for (head, ps) in body.iter().filter_map(|l| more(l)) {
                        let inst = head.split_whitespace().next().unwrap_or("");
                        let has = |p: &String| top.contains(p) || b.is_some_and(|b| b.own.contains(p) || b.params.contains(p));
                        for p in ps.iter().filter(|p| !has(p)) {
                            let in_sheet = b.map_or(String::new(), |b| format!(" (in {}'s schematic)", b.name));
                            nl.warnings.push(format!("{inst}{in_sheet} passes no {p}, which its block's schematic uses without a .param {p} of its own (and none above defines it): give {inst} {p}=… in its Value"));
                        }
                    }
                }
            }
        }
        if lines.iter().any(|l| l.contains(BLOCK_TAG)) {
            lines = lines.iter().map(fill).collect();
        }
        for b in &h.blocks {
            let params: String = b.params.iter().map(|p| format!(" {p}=0")).collect();
            lines.push(format!(".subckt {} {}{}", b.name, b.ports.join(" "), if params.is_empty() { String::new() } else { format!(" params:{params}") }));
            lines.extend(b.body.iter().map(fill));
            lines.push(format!(".ends {}", b.name));
        }
        lines.push(".end".into());
    }
    nl.text = lines.join("\n") + "\n";
    nl
}

/// A block's sheet as a subcircuit, into `h.blocks` after the blocks it has itself: its parts and
/// definitions; its ports the nets of the labels named as the symbol's pins.
// ponytail: every netlisting (the live one on each edit, a run twice) netlists the sheets again
// from their cached items; cache their netlists per file stamp if big hierarchies make editing lag
fn sheet_subckt(name: &str, path: &str, def: &SymbolDef, lib: &mut Library, h: &mut Hier, warnings: &mut Vec<String>) {
    let file = std::path::Path::new(path).file_name().map_or(path.into(), |f| f.to_string_lossy().into_owned());
    let mut warn = |w: String| if !warnings.contains(&w) { warnings.push(w) };
    // (said once, by the top schematic's netlist; of a BLOCK symbol only: beside a library part's
    // symbol its test bench, which places it, is simply no block)
    let said = def.block.then(|| def.name.clone());
    let sym = &def.name;
    let refuse = |why: String, h: &mut Hier| _ = h.refused.insert(name.to_string(), if said.is_some() { format!("{file} {why}: it is no subcircuit here, a {sym} calls a model of that name") } else { String::new() });
    if let Some(at) = h.stack.iter().position(|(n, _)| n == name) {
        // (every block of the loop alike, whichever of them came first)
        for (n, said) in h.stack[at + 1..].to_vec() {
            h.refused.insert(n, said.map_or(String::new(), |s| format!("{s}'s schematic places itself (through its blocks): it is no subcircuit here, a {s} calls a model of that name")));
        }
        return refuse("places its own block (through its blocks)".into(), h);
    }
    let items = match lib.sheet(path) {
        Ok(items) => items,
        Err(e) => return refuse(format!("cannot be read ({e})"), h), // (once: not tried again for every part)
    };
    // (seen before netlisting it: a library part's test bench, placing it, is the part's model, as
    // before it had a sheet; a schematic of another kind that only shares its name lacks a pin)
    if let Err(why) = def.sheet_fits(&items) {
        return refuse(why, h);
    }
    let label = |pin: &str| sheet_label(&items, pin);
    // (netlisted on trial: refused, what it added goes again, but for what it found refused and
    // the names it gave: true anyway, and not to be found again)
    // ponytail: the whole state copied per sheet (blocks so far, models); undo by lengths if big
    // hierarchies make editing lag
    let mut trial = h.clone();
    let keep = |trial: Hier, h: &mut Hier| (h.refused, h.names) = (trial.refused, trial.names);
    trial.stack.push((name.to_string(), said.clone()));
    let child = build_netlist(&items, lib, false, &mut trial);
    trial.stack.pop();
    if trial.refused.contains_key(name) {
        return keep(trial, h); // (it places itself, through its blocks: said)
    }
    // (every pin has its label: refused above otherwise)
    let ports: Vec<String> = def.pins.iter().map(|p| label(&p.name).and_then(|k| child.nets[k].first().cloned()).unwrap_or_default()).collect();
    // (a port on ground, two on one net: no subcircuit SPICE takes)
    let shared = (0..ports.len()).flat_map(|a| (a + 1..ports.len()).map(move |b| (a, b))).find(|&(a, b)| ports[a] == ports[b]);
    let why = match (ports.iter().position(|n| n == "0"), shared) {
        (Some(k), _) => Some(format!("has its port {} on ground", def.pins[k].name)),
        (_, Some((a, b))) => Some(format!("has its ports {} and {} on one net", def.pins[a].name, def.pins[b].name)),
        _ => None,
    };
    if let Some(why) = why {
        keep(trial, h);
        return refuse(why, h);
    }
    *h = trial;
    for w in child.warnings {
        warn(format!("{file}: {w}"));
    }
    let lines = deck_lines(&child.text);
    let own = lines.iter().filter(|l| l.top && l.param())
        .flat_map(|l| param_defs(l.line, &l.low).into_iter().map(|(n, _)| n.to_ascii_lowercase())).collect();
    // (its .global lines: the top netlist's; not those inside a .subckt of its own)
    let mut body = vec![];
    for l in &lines {
        if l.top && l.head == ".global" {
            // (with its `+` lines, as one: alike ones of two sheets are one)
            match h.hoisted.last_mut().filter(|_| l.line.trim_start().starts_with('+')) {
                Some(g) => (g.push('\n'), g.push_str(l.line)).1,
                None => h.hoisted.push(l.line.to_string()),
            }
        } else {
            body.push(l.line.to_string());
        }
    }
    h.blocks.push(Block { name: name.into(), ports, body, params: Default::default(), own });
}

#[cfg(test)]
mod tests {
    use super::*;
    // libngspice is one global engine: sim tests must not interleave run/trace.
    use crate::ngspice::TEST_SIM as SIM;

    const RC: &str = "Version 4\nSHEET 1 880 680\nWIRE 96 64 -32 64\nWIRE 224 64 176 64\nWIRE 224 96 224 64\nWIRE -32 96 -32 64\nFLAG -32 176 0\nFLAG 224 160 0\nFLAG 224 64 out\nSYMBOL voltage -32 80 R0\nSYMATTR InstName V1\nSYMATTR Value SINE(0 1 1k)\nSYMBOL res 192 48 R90\nSYMATTR InstName R1\nSYMATTR Value 1k\nSYMBOL cap 208 96 R0\nSYMATTR InstName C1\nSYMATTR Value 100n\nTEXT -32 216 Left 2 !.tran 3m\n";

    #[test]
    fn text_escapes_and_latin1_like_ltspice() {
        let items = parse_asc("TEXT 0 0 Left 2 !.lib C:\\\\Users\\\\me\\\\npn.lib\\n.tran 1m\nSYMBOL cap 0 0 R0\nSYMATTR Value 100µ\n");
        let Item::Text { text, .. } = &items[0] else { panic!() };
        assert_eq!(text, ".lib C:\\Users\\me\\npn.lib\n.tran 1m");
        let out = write_asc(&items);
        let again = parse_asc(&out);
        assert!(again.contains(&items[0]) && write_asc(&again) == out, "{out}");
        let bytes = encode(&out);
        assert!(bytes.contains(&0xB5) && !bytes.windows(2).any(|w| w == [0xC2, 0xB5]), "µ must be Latin-1");
        assert_eq!(decode(&bytes), out);
        assert_eq!(decode(&encode("Version 4\nΩ")), "Version 4\nΩ"); // UTF-16LE, detected from the ASCII start
    }

    #[test]
    fn rotations_match_ltspice() {
        assert_eq!(xform("R90", 16, 16), (-16, 16));
        assert_eq!(xform("M90", 1, 2), (2, 1));
        assert_eq!(xform("M270", 1, 2), (-2, -1));
        assert_eq!(xform("M180", 1, 2), (1, -2));
    }

    #[test]
    fn tran_translation() {
        let t = |l: &str| directive(l, &mut vec![]).unwrap();
        assert_eq!(t(".tran 1u 1m startup"), format!(".tran 1u 1m uic\n{STARTUP}"));
        assert_eq!(t(".tran 0 1m 0 1u uic"), ".tran 1e-6 1m 0 1u uic");
        assert_eq!(t(".tran 0 {T} startup"), format!(".tran {{(T)/1000}} {{T}} uic\n{STARTUP}"));
        // startup: DC sources ramp up, functions and current sensors stay
        assert_eq!(ramped("V1 vcc 0 12").as_deref(), Some("V1 vcc 0 PWL(0 0 20e-6 12)"));
        assert_eq!(ramped("V2 a 0 DC {Vs} AC 1").as_deref(), Some("V2 a 0 PWL(0 0 20e-6 {Vs}) AC 1"));
        assert_eq!(ramped("I1 0 b 1m").as_deref(), Some("I1 0 b PWL(0 0 20e-6 1m)"));
        assert_eq!(ramped("V3 a 0 SINE(0 1 1k)"), None);
        assert_eq!(ramped("Vsense_r1 a b 0").as_deref(), Some("Vsense_r1 a b PWL(0 0 20e-6 0)")); // (stays 0; steppable)
        assert_eq!(ramped("R1 a b 1k"), None);
        assert_eq!(ramped("V1 vcc 0 12 ; supply").as_deref(), Some("V1 vcc 0 PWL(0 0 20e-6 12) ; supply"));
        assert_eq!(ramped("V1 in 0 DC 0 AC 1 SIN(0 1 1k)"), None); // (a waveform of its own after the AC part)
        assert_eq!(ramped("V1 a 0 AC 1 DC 5").as_deref(), Some("V1 a 0 PWL(0 0 20e-6 5) AC 1")); // (DC after the AC part)
        // the ramp pass: not the title line; a stepped source without a DC value ramps too
        let p = prepare(&format!("Vref test 1 2\nV1 a 0 5\n.tran 1u 1m uic\n{STARTUP}\n.end\n"), None, &mut vec![]);
        assert!(p.0.starts_with("* Vref test 1 2\nV1 a 0 PWL(0 0 20e-6 5)\n"), "{}", p.0); // (the title a comment)
        let p = prepare(&format!("* t\nV1 vcc 0\n+ 12\nR1 vcc 0 1k\n.tran 1u 1m uic\n{STARTUP}\n.end\n"), None, &mut vec![]);
        assert!(p.0.starts_with("* t\nV1 vcc 0 PWL(0 0 20e-6 12)\nR1 vcc 0 1k\n"), "{}", p.0); // (over + lines)
        let p = prepare(&format!("* t\nV1 vcc 0 ; supply\n* note\n+ 12 Rser=0.1\nR1 vcc 0 1k\n.tran 1u 1m uic\n{STARTUP}\n.end\n"), None, &mut vec![]);
        assert!(p.0.starts_with("* t\nV1 vcc 0 PWL(0 0 20e-6 12) Rser=0.1 ; supply\n* note\nR1 vcc 0 1k\n"), "{}", p.0); // (a comment between, parameters after)
        let out = parse_step(".step V1 1 5 1").unwrap().apply(&format!("V1 a 0 PWL(0 0 20e-6 {{max(Va, Vb)}})\n.tran 1u 1m uic\n{STARTUP} ; ramped: v1\n"), 3.0).unwrap();
        assert!(out.starts_with("V1 a 0 PWL(0 0 20e-6 3"), "{out}"); // (a braced end value stepped)
        // a root locus on the deck: its .pz sees the sources as they were
        let p = prepare(&format!("* t\nV1 a 0 5\nR1 a 0 1k\n.tran 1u 1m uic\n{STARTUP}\n.pz a 0 a 0 vol pol\n.end\n"), None, &mut vec![]);
        let pz = p.only_pz().0;
        assert!(pz.contains("\nV1 a 0 5\n") && pz.contains("\n* .tran 1u 1m uic\n") && !pz.contains("* .pz"), "{pz}");
        assert_eq!(ramp_end("PWL(0 0 20e-6 {a} 1m {b})"), None); // (braced points of the user's own)
        // (beside other analyses not ramped: a stepped source without a DC value neither)
        let out = parse_step(".step V1 1 5 1").unwrap().apply(&format!("V1 in 0 AC 1\n.tran 1u 1m uic\n{STARTUP}\n.ac dec 10 1 1k\n"), 2.0).unwrap();
        assert!(out.starts_with("V1 in 0 DC 2e0 AC 1\n"), "{out}");
        // (a `;` comment between a source and its + line)
        let p = prepare(&format!("* t\nV1 vcc 0\n; supply\n+ 12\n.tran 1u 1m uic\n{STARTUP}\n.end\n"), None, &mut vec![]);
        assert!(p.0.starts_with("* t\nV1 vcc 0 PWL(0 0 20e-6 12)\n* supply\n"), "{}", p.0);
        let st = parse_step(".step V1 1 5 1").unwrap();
        let out = st.apply(&format!("V1 in 0 AC 1\n.tran 1u 1m uic\n{STARTUP} ; ramped:\n"), 2.0).unwrap();
        assert!(out.starts_with("V1 in 0 PWL(0 0 20e-6 2e0) AC 1\n"), "{out}");
        // white(c*time): a TRNOISE source of its own, before .end; other arguments are said
        let mut w = vec![];
        let out = lt_white("B1 n 0 V=white(1e6*time)/10 + white( time*2k )\n.tran 1m\n.end ; done\n", &mut w).unwrap();
        let (w1, w2) = ("max(min(v(nmsp_white1),0.5),-0.5)", "max(min(v(nmsp_white2),0.5),-0.5)");
        assert!(out.starts_with(&format!("B1 n 0 V={w1}/10 + {w2}\n.tran 1m\nVnmsp_white1 nmsp_white1 0 dc 0 trnoise(0.323524 1e-6 0 0)\nVnmsp_white2 nmsp_white2 0 dc 0 trnoise(0.323524 5e-4 0 0)\n.end ; done")), "{out}");
        assert!(w.is_empty(), "{w:?}");
        let out = lt_white("B1 n 0 V=white(V(x))\n", &mut w).unwrap();
        assert!(out.contains("white(V(x))") && w.len() == 1, "{out} {w:?}");
        assert_eq!(lt_white("B1 n 0 V=1\n", &mut w), None);
        // not in a comment; a deck's own white() stays
        let out = lt_white("B1 n 0 V=white(time) ; was white(2*time)\n", &mut w).unwrap();
        assert!(out.starts_with(&format!("B1 n 0 V={w1} ; was white(2*time)\n")) && !out.contains("nmsp_white2"), "{out}");
        assert_eq!(lt_white(".func white(x) {sin(x)}\nB1 n 0 V=white(time)\n", &mut w), None);
        let mut w = vec![];
        let out = lt_white("B1 n 0 V=white(1e5*time)-white(1e5*time)\nB2 m 0 V=1\n+ +white(time)\n", &mut w).unwrap();
        assert!(out.starts_with(&format!("B1 n 0 V={w1}-{w1}\n")) && !out.contains("nmsp_white2") && w.len() == 1, "{out} {w:?}");
        // a subcircuit with a white() of its own: left alone, nothing said
        let mut w = vec![];
        let taken = lt_white("B1 n 0 V=white(time)+white(2*time)\nR1 nmsp_white2 0 1k\n", &mut w).unwrap();
        assert!(taken.contains("Vnmsp_white1 nmsp_white1") && taken.contains("Vnmsp_white3 nmsp_white3") && !taken.contains("Vnmsp_white2"), "{taken}"); // (a name the deck has: skipped)
        let mut w = vec![];
        let own = ".subckt n a b\n.func white(x) {sin(x)}\nB1 a b V=white(time)\n.ends\nB2 m 0 V=white(time)\n";
        let out = lt_white(own, &mut w).unwrap();
        assert!(out.contains("B1 a b V=white(time)") && out.contains(&format!("B2 m 0 V={w1}")) && w.is_empty(), "{out} {w:?}");
        // a .step of a source ramped up: the ramp's end
        let st = parse_step(".step V1 1 2 1").unwrap();
        let su = format!("V1 a 0 PWL(0 0 20e-6 5)\n.tran 1m uic\n{STARTUP} ; ramped: v1\n");
        assert!(st.apply(&su, 2.0).unwrap().starts_with("V1 a 0 PWL(0 0 20e-6 2"), "{:?}", st.apply(&su, 2.0));
        // (the same PWL without startup is the user's own: a DC value is put in)
        assert!(st.apply("V1 a 0 PWL(0 0 20e-6 5)\n", 2.0).unwrap().contains("PWL(0 0 20e-6 5)"));
        assert!(st.apply(&format!("V1 a 0 PWL(0 0 20u 5)\n{STARTUP}\n"), 2.0).unwrap().contains("DC 2e0 PWL(0 0 20u 5)")); // (written 20u: the user's own)
        // (a PWL of the user's own: no DC value to step, one put after the nodes)
        assert!(st.apply("V1 a 0 PWL(0 0 20e-6 5 1m 3)\n", 2.0).unwrap().contains("PWL(0 0 20e-6 5 1m 3)"));
        assert_eq!(t(".tran {T}"), ".tran {(T)/1000} {T}");
        assert_eq!(t(".tran 3m"), ".tran 3e-6 3m");
        assert_eq!(t(".tran 1m ; 10 cycles uic"), ".tran 1e-6 1m");
        let p = prepare("* t\n.tran 10µ\nR1 a 0 10kΩ\n", None, &mut vec![]);
        assert!(p.text().is_ascii() && p.text().contains("10u") && p.text().contains("10k\n"), "{}", p.text());
        assert_eq!(net_name("Vout+"), "Vout+");
        assert_eq!(t(".tran 10μ"), ".tran 1e-8 10μ");
        assert_eq!(net_name("a;b(c)"), "a_b_c_");
        assert_eq!(t(".tran {T * 2}"), ".tran {(T * 2)/1000} {T * 2}");
        assert_eq!(t(".tran 0 {Tstop} 0 {Tstop / 1000}"), ".tran {(Tstop)/1000} {Tstop} 0 {Tstop / 1000}");
    }

    #[test]
    fn tran_with_parameters_simulates() {
        let _s = SIM.lock();
        let src = "WIRE 0 0 0 16\nFLAG 0 96 0\nFLAG 0 0 a\nSYMBOL voltage 0 0 R0\nSYMATTR Value 1\nTEXT 0 200 Left 2 !.param T=1m\\n.tran 0 {T}\n";
        let nl = netlist(&parse_asc(src), &mut Library::builtin());
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).unwrap_or_else(|l| panic!("{}\n{}", nl.text, l.join("\n")));
        assert_eq!(run.scale, "time");
    }

    #[test]
    fn diode_strings_jumpers_and_pi() {
        let src = "FLAG 0 0 a\nFLAG 64 0 b\nSYMBOL diode 0 0 R0\nSYMATTR InstName D1\nSYMATTR Value D\nSYMATTR SpiceLine N=3\nSYMBOL Misc\\jumper 100 0 R0\nSYMATTR InstName J1\nFLAG 68 64 x\nFLAG 132 64 y\nTEXT 0 100 Left 2 !.param w=2*pi\n";
        let mut lib = Library::builtin();
        let nl = netlist(&parse_asc(src), &mut lib);
        assert!(nl.text.contains("VJ1 x y 0"), "{}", nl.text);
        // a second jumper between the same nets, and one shorting a net to itself, are dropped
        let src2 = format!("{src}SYMBOL Misc\\jumper 100 0 R0\nSYMATTR InstName J2\nSYMBOL Misc\\jumper 300 0 R0\nSYMATTR InstName J3\nFLAG 268 64 z\nFLAG 332 64 z\n");
        let nl2 = netlist(&parse_asc(&src2), &mut lib);
        assert!(!nl2.text.contains("VJ2") && !nl2.text.contains("VJ3"), "{}", nl2.text);
        assert!(nl.text.contains("D1 N001 D1_n1 D\n") && nl.text.contains("D1_2 D1_n1 D1_n2 D\n") && nl.text.contains("D1_3 D1_n2 N002 D\n"), "{}", nl.text);
        assert!(!nl.text.contains("\nJ1") && nl.text.contains(".param pi="), "{}", nl.text);
    }

    #[test]
    fn ako_models_and_behavioural_diode_warning() {
        let mut w = vec![];
        prepare("* t\n.model TVS D(Ron=.5 Roff=10Meg\n+ Vfwd=5 Vrev=30)\n.end\n", None, &mut w);
        assert!(w.is_empty(), "unused model must not warn: {w:?}");
        prepare("* t\nD1 a 0 TVS\n.model TVS D(Ron=.5 Roff=10Meg\n+ Vfwd=5 Vrev=30)\n.end\n", None, &mut w);
        assert!(w.iter().any(|w| w.contains("model tvs")), "{w:?}");
        let mut w = vec![];
        prepare("* t\n  D1 a 0\n+ TVS\n.model TVS D(Ron=.5)\n.end\n", None, &mut w);
        assert!(w.iter().any(|w| w.contains("model tvs")), "indented/continued diode line: {w:?}");
        if let Some(card) = crate::library::definitions("ztx849", &mut HashSet::new()).and_then(|d| d.last().cloned()) {
            assert!(!card.to_lowercase().contains("ako:") && card.to_lowercase().starts_with(".model ztx849 npn("), "{card}");
            assert!(!card.contains(';'), "comment swallowed the overrides: {card}");
            assert_eq!(card.to_lowercase().matches("mfg=").count(), 1, "{card}");
        }
    }

    #[test]
    fn ltspice_stock_vdmos_is_used() {
        if crate::library::definitions("bsp89", &mut HashSet::new()).is_none() {
            return; // no local LTspice library
        }
        let _s = SIM.lock();
        let src = "FLAG 48 0 d\nFLAG 0 80 g\nFLAG 48 96 0\nSYMBOL nmos 0 0 R0\nSYMATTR InstName M1\nSYMATTR Value BSP89\nSYMBOL voltage -100 0 R0\nSYMATTR InstName V1\nSYMATTR Value 5\nFLAG -100 16 g\nFLAG -100 96 0\nSYMBOL voltage 200 0 R0\nSYMATTR InstName V2\nSYMATTR Value 10\nFLAG 200 16 d\nFLAG 200 96 0\nTEXT 0 200 Left 2 !.op\n";
        let nl = netlist(&parse_asc(src), &mut Library::builtin());
        assert!(nl.text.to_lowercase().contains(".model bsp89") && nl.warnings.is_empty(), "{}\n{:?}", nl.text, nl.warnings);
        assert!(crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).is_ok(), "{}", nl.text);
    }

    #[test]
    fn ltspice_instance_parameters() {
        let _s = SIM.lock();
        let src = EXAMPLES.iter().find(|e| e.0 == "rc_transient").unwrap().1
            .replace("SYMATTR Value 100n", "SYMATTR Value 100n Lser=1n\nSYMATTR SpiceLine V=6.3 Irms=0 Rser=0.1 mfg=\"Taiyo Yuden\" type=\"Al electrolytic\"")
            .replace("SYMATTR Value SINE(0 1 1k)", "SYMATTR Value SINE(0 1 1k)\nSYMATTR SpiceLine Rser=50")
            .replace("!.tran 3m", "!.step param X 1 2\\n+ 3 4\\n.tran 3m");
        let nl = netlist(&parse_asc(&src), &mut Library::builtin());
        assert!(nl.text.contains("LC1_lser out C1_lser 1n") && nl.text.contains("RC1_rser C1_lser C1_rser 0.1"), "{}", nl.text);
        assert!(!nl.text.contains("Yuden") && !nl.text.contains("+ 3 4"), "{}", nl.text);
        assert!(nl.text.contains("C1 C1_rser 0 100n\n") && nl.text.contains("RV1_rser"), "{}", nl.text);
        assert!(nl.warnings.iter().any(|w| w.contains("irms")), "{:?}", nl.warnings);
        assert!(crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).is_ok(), "{}", nl.text);
    }

    #[test]
    fn noise_source_gets_ac_value() {
        let _s = SIM.lock();
        let src = EXAMPLES.iter().find(|e| e.0 == "rc_transient").unwrap().1.replace(".tran 3m", ".noise v(out, 0) V1 dec 10 10 1Meg").replace("FLAG -32 64 in", "FLAG -32 64 ac");
        let nl = netlist(&parse_asc(&src), &mut Library::builtin());
        assert!(nl.text.contains("SIN(0 1 1k) AC 1"), "{}", nl.text);
        assert!(crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).is_ok());
    }

    #[test]
    fn single_frequency_ac_is_a_phasor_table() {
        let _s = SIM.lock();
        // RC low-pass at its corner frequency 1/(2*pi*1k*100n): |V(out)| = 1/sqrt(2), -45 degrees
        let src = EXAMPLES.iter().find(|e| e.0 == "rc_bode").unwrap().1.replace(".ac dec 50 10 1Meg", ".ac lin 1 1591.549 1591.549");
        let nl = netlist_for_sim(&parse_asc(&src), &mut Library::builtin());
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![]).with_sensors(nl.sensors.clone())).unwrap();
        assert!(run.scale.is_empty(), "a table, not a 1-point plot: {run:?}");
        let get = |n: &str| run.op.iter().find(|o| o.0 == n).cloned().unwrap_or_else(|| panic!("{n} missing: {:?}", run.op));
        let out = get("out");
        assert!((out.1.hypot(out.2) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-4 && (out.2.atan2(out.1).to_degrees() + 45.0).abs() < 0.01, "{out:?}");
        assert_eq!((out.3.as_str(), get("I(R1)").3.as_str()), ("V", "A"));
    }

    #[test]
    fn switches_simulate_and_netlist() {
        let _s = SIM.lock();
        // S1 between vdd and out, controlled by a step at 1 ms; the built-in model (Vt=0.5, Ron=1)
        let src = "Version 4\nSHEET 1 880 680\nFLAG 0 16 vdd\nFLAG 0 96 out\nFLAG -48 80 ctl\nFLAG -48 32 0\n\
            FLAG 200 16 vdd\nFLAG 200 96 0\nFLAG 300 16 ctl\nFLAG 300 96 0\nFLAG 416 16 out\nFLAG 416 96 0\n\
            SYMBOL sw 0 0 R0\nSYMATTR InstName S1\nSYMBOL voltage 200 0 R0\nSYMATTR InstName V1\nSYMATTR Value 1\n\
            SYMBOL voltage 300 0 R0\nSYMATTR InstName V2\nSYMATTR Value PULSE(0 1 1m 1n 1n 5m 10m)\n\
            SYMBOL res 400 0 R0\nSYMATTR InstName R1\nSYMATTR Value 1k\nTEXT 0 200 Left 2 !.tran 2m\n";
        let nl = netlist_for_sim(&parse_asc(src), &mut Library::builtin());
        assert!(nl.text.contains("S1 vdd out ctl 0 SW") && nl.text.contains(".model SW SW("), "{}", nl.text);
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).unwrap();
        let out = crate::ngspice::trace("v(out)", &run.scale, run.id).unwrap();
        let at = |t: f64| out.re[out.x.iter().position(|&x| x >= t).unwrap()];
        assert!(at(0.5e-3).abs() < 1e-3 && (at(1.5e-3) - 1.0).abs() < 2e-3, "off {} on {}", at(0.5e-3), at(1.5e-3));
        // W: Value is `Vcontrol model`; the model is the last token
        let w = src.replace("SYMBOL sw 0 0 R0\nSYMATTR InstName S1", "SYMBOL csw 0 16 R0\nSYMATTR InstName W1\nSYMATTR Value V1 CSW");
        let nl = netlist(&parse_asc(&w), &mut Library::builtin());
        assert!(nl.text.contains("W1 vdd out V1 CSW") && nl.text.contains(".model CSW CSW("), "{}", nl.text);
        let bare = netlist(&parse_asc(&w.replace("SYMATTR Value V1 CSW", "")), &mut Library::builtin());
        assert!(bare.warnings.iter().any(|m| m.contains("controlling source")), "{:?}", bare.warnings);
    }

    #[test]
    fn behavioral_startup_and_white_simulate() {
        let _s = SIM.lock();
        // bv as a multiplier: out = 2·V(a)·V(a); .tran startup ramps V1 up from 0 (no operating point)
        let src = "Version 4\nSHEET 1 880 680\nFLAG 0 16 a\nFLAG 0 96 0\nFLAG 200 16 out\nFLAG 200 96 0\nFLAG 400 16 n\nFLAG 400 96 0\n\
            SYMBOL voltage 0 0 R0\nSYMATTR InstName V1\nSYMATTR Value 3\nSYMBOL bv 200 0 R0\nSYMATTR InstName B1\nSYMATTR Value V=2*V(a)*V(a)\n\
            SYMBOL bv 400 0 R0\nSYMATTR InstName B2\nSYMATTR Value V=white(1e5*time)\nTEXT 0 200 Left 2 !.tran 0 2m 0 1u startup\n";
        let nl = netlist_for_sim(&parse_asc(src), &mut Library::builtin());
        let deck = prepare(&nl.text, None, &mut vec![]);
        assert!(nl.text.contains("B1 out 0 V=2*V(a)*V(a)") && deck.0.contains("V1 a 0 PWL(0 0 20e-6 3)") && deck.0.contains(" uic"), "{}", deck.0);
        let run = crate::ngspice::run(&deck).unwrap();
        let tr = |e: &str| crate::ngspice::trace(e, &run.scale, run.id).unwrap();
        let out = tr("v(out)");
        let at = |t: f64| out.re[out.x.iter().position(|&x| x >= t).unwrap()];
        assert!(at(0.0).abs() < 1e-3 && (at(1e-3) - 18.0).abs() < 1e-6, "start {} later {}", at(0.0), at(1e-3));
        // white(): noise, new values of rms 1/√12 every 10 µs (in between interpolated: lower)
        let n = tr("v(n)");
        let rms = (n.re.iter().map(|v| v * v).sum::<f64>() / n.re.len() as f64).sqrt();
        assert!((0.2..0.37).contains(&rms), "rms {rms}");
        // a `;` comment line between a source and its + line: the source's value, as LTspice reads it
        let run = crate::ngspice::run(&prepare("t\nV1 vcc 0\n; supply\n+ 12\nR1 vcc 0 1k\n.tran 1u 10u\n.end\n", None, &mut vec![])).unwrap();
        let v = crate::ngspice::trace("v(vcc)", &run.scale, run.id).unwrap();
        assert!((v.re.last().unwrap() - 12.0).abs() < 1e-9, "{:?}", v.re.last());
    }

    #[test]
    fn two_port_net_simulates() {
        // .net I(R1) V1 stays as a comment the plot reads; its quantities as the plot writes them out
        // (geom's twoPortIn): V1 (AC 1, Rser 50) -> 50 Ω in series -> R1 50 Ω: Zin 100, S11 1/3, S21 2/3
        let _s = SIM.lock();
        let src = "Version 4\nSHEET 1 880 680\nFLAG 0 16 in\nFLAG 0 96 0\nFLAG 112 16 in\nFLAG 112 96 out\nFLAG 208 16 out\nFLAG 208 96 0\n\
            SYMBOL voltage 0 0 R0\nSYMATTR InstName V1\nSYMATTR Value AC 1\nSYMATTR SpiceLine Rser=50\n\
            SYMBOL res 96 0 R0\nSYMATTR InstName R2\nSYMATTR Value 50\nSYMBOL res 192 0 R0\nSYMATTR InstName R1\nSYMATTR Value 50\n\
            TEXT 0 200 Left 2 !.net I(R1) V1\nTEXT 0 232 Left 2 !.ac lin 2 1k 2k\n";
        let nl = netlist_for_sim(&parse_asc(src), &mut Library::builtin());
        assert!(nl.text.contains(&format!("* .net I(R1) V1 {NET_MARK}")) && nl.text.contains("RV1_rser in V1_rser 50") && !nl.warnings.iter().any(|w| w.contains(".net")), "{}\n{:?}", nl.text, nl.warnings);
        // (its + lines onto its comment, not onto the .ac before it)
        let plus = netlist_for_sim(&parse_asc(&src.replace("!.net I(R1) V1", "!.ac dec 10 1 1Meg\\n.net I(R1) V1\\n+ Rout=75")), &mut Library::builtin());
        assert!(plus.text.contains(&format!("* .net I(R1) V1 Rout=75 {NET_MARK}")) && plus.text.contains(".ac dec 10 1 1Meg\n"), "{}", plus.text);
        let between = netlist_for_sim(&parse_asc(&src.replace("!.net I(R1) V1", "!.ac dec 10 1 1Meg\\n.net I(R1) V1\\n* 75 ohm port\\n+ Rout=75")), &mut Library::builtin());
        assert!(between.text.contains(&format!("* .net I(R1) V1 Rout=75 {NET_MARK}")) && !between.text.contains("\n+ Rout"), "{}", between.text); // (a comment between)
        let tr = netlist_for_sim(&parse_asc(&src.replace("!.net I(R1) V1", "!.tran 0 10m\\n+ startup")), &mut Library::builtin());
        assert!(tr.text.contains(&format!(" uic\n{STARTUP}")) && !tr.text.contains("+ startup"), "{}", tr.text); // (startup on a + line)
        let tc = netlist_for_sim(&parse_asc(&src.replace("!.net I(R1) V1", "!.tran 0 10m steady\\n* soft start\\n+ startup")), &mut Library::builtin());
        assert!(tc.text.contains(&format!(" uic\n{STARTUP}")) && !tc.text.contains("+ startup"), "{}", tc.text); // (a comment between)
        let th = netlist_for_sim(&parse_asc(&src.replace("!.net I(R1) V1", "!.tran 0 10m\\n# note\\n+ startup")), &mut Library::builtin());
        assert!(th.text.contains(&format!(" uic\n{STARTUP}")) && th.text.contains("\n* note\n"), "{}", th.text); // (a # comment between, as SPICE's *)
        let ts = netlist_for_sim(&parse_asc(&src.replace("!.net I(R1) V1", "!.tran 0 10m\\n// note\\n+ startup")), &mut Library::builtin());
        assert!(ts.text.contains(&format!(" uic\n{STARTUP}")) && ts.text.contains("\n* note\n"), "{}", ts.text); // (likewise a // one)
        let tl = netlist_for_sim(&parse_asc(&src.replace("!.net I(R1) V1", "!.tran 0 10m\\n; note\\n+ startup")), &mut Library::builtin());
        assert!(tl.text.contains(&format!(" uic\n{STARTUP}")) && tl.text.contains("\n* note\n"), "{}", tl.text); // (and LTspice's ;)
        assert_eq!(tc.warnings.iter().filter(|w| w.contains("steady")).count(), 1, "{:?}", tc.warnings); // (said once)
        let sk = netlist_for_sim(&parse_asc(&src.replace("!.net I(R1) V1", "!.wave out.wav 16 44.1k V(out)\\n* right channel\\n+ V(in)")), &mut Library::builtin());
        assert!(!sk.text.contains("+ V(in)"), "{}", sk.text); // (a skipped directive's + line after a comment)
        let mut w = vec![];
        let both = prepare(&format!("* t\nV1 a 0 5\n.tran 1u 1m uic\n{STARTUP}\n.op\n.end\n"), None, &mut w);
        assert!(both.0.contains("\nV1 a 0 5\n") && w.iter().any(|m| m.contains("beside other analyses")), "{} {w:?}", both.0); // (with .op: not ramped, said)
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).unwrap();
        let mag = |e: &str| { let t = crate::ngspice::trace(e, &run.scale, run.id).unwrap(); t.re[0].hypot(t.im.as_ref().map_or(0.0, |i| i[0])) };
        let zin = "((v(\"in\")-0)/(-i(v1)))";
        assert!((mag(zin) - 100.0).abs() < 1e-6, "Zin {}", mag(zin));
        assert!((mag(&format!("(({zin}-(50))/({zin}+(50)))")) - 1.0 / 3.0).abs() < 1e-9);
        assert!((mag("(2*v(\"out\")/(1)*sqrt((50)/(50)))") - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn new_symbols_parse() {
        let def = |n: &str| parse_asy(n, BUILTIN_SYMBOLS.iter().find(|s| s.0 == n).unwrap().1);
        let pins = |n: &str| def(n).pins.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        // LTspice's pin order (opamp2, tline), the subcircuits' port order (gates, dff)
        assert_eq!(pins("opamp2"), ["In+", "In-", "V+", "V-", "OUT"]);
        assert_eq!(pins("tline"), ["I1", "R1", "I2", "R2"]);
        assert_eq!(pins("dff"), ["D", "C", "Q", "Qn"]);
        assert_eq!(def("and2").texts, vec![(0, 18, "Center".to_string(), 2, "&".to_string())]);
        assert_eq!((def("bv").prefix.as_str(), def("ltline").prefix.as_str(), def("ltline").value.as_str()), ("B", "O", "LTRA"));
        // opamp2's default parameters go with its own model only
        let op = |v: &str| netlist(&parse_asc(&format!("SYMBOL opamp2 0 0 R0\nSYMATTR InstName U1\n{v}TEXT 0 200 Left 2 !.op\n")), &mut Library::builtin()).text;
        assert!(op("").contains("opamp2 Aol=100K GBW=10Meg Slew=10Meg Vsat=0.1"), "{}", op(""));
        let tl = op("SYMATTR Value TL072\n");
        assert!(tl.lines().any(|l| l.starts_with("XU1") && l.trim_end().ends_with("TL072")), "{tl}");
        // (a parameter given in Value: its default not after it)
        let own = op("SYMATTR Value opamp2 Slew=1Meg\n");
        let x = |nl: &str| nl.lines().find(|l| l.starts_with("XU1")).unwrap_or("").to_string();
        assert_eq!(x(&own), "XU1 N001 N002 N003 N004 N005 opamp2 Slew=1Meg Aol=100K GBW=10Meg Vsat=0.1");
        assert_eq!(x(&tl), "XU1 N001 N002 N003 N004 N005 TL072");
        assert_eq!(x(&op("SYMATTR Value opamp2\nSYMATTR Value2 Slew=1Meg\n")), "XU1 N001 N002 N003 N004 N005 opamp2 Slew=1Meg Aol=100K GBW=10Meg Vsat=0.1"); // (Value2's parameter too)
        assert!(!x(&op("SYMATTR Value opamp2 Slew = 1Meg\n")).contains("Slew=10Meg")); // (blanks around =)
    }

    #[test]
    fn opamp2_gates_and_lines_simulate() {
        let _s = SIM.lock();
        let model = |n: &str| BUILTIN_MODELS.iter().find(|m| m.0 == n).unwrap().1;
        let run = |body: &str, models: &[&str]| {
            let deck = format!("* test\n{body}\n{}\n.end\n", models.iter().map(|m| model(m)).collect::<Vec<_>>().join("\n"));
            crate::ngspice::run(&prepare(&deck, None, &mut vec![])).unwrap()
        };
        let tr = |r: &crate::ngspice::Run, e: &str| {
            let t = crate::ngspice::trace(e, &r.scale, r.id).unwrap();
            move |at: f64| t.re[t.x.iter().position(|&x| x >= at).unwrap()]
        };
        // opamp2 as a follower on one 5 V supply: no swing below Vsat above 0 V, 1 V/µs up to 3 V
        let r = run("V1 vcc 0 5\nV2 in 0 PULSE(-1 3 10u 1n 1n 20u 40u)\nX1 in out vcc 0 out opamp2 Aol=100K GBW=10Meg Slew=1Meg Vsat=0.1\n.tran 10n 40u", &["opamp2"]);
        let out = tr(&r, "v(out)");
        assert!((out(5e-6) - 0.1).abs() < 0.02 && (out(30e-6) - 3.0).abs() < 0.02, "clipped {} high {}", out(5e-6), out(30e-6));
        assert!((out(11.5e-6) - 1.6).abs() < 0.3, "slewing {}", out(11.5e-6));
        // gates: a high in [0,1µ) and [2µ,3µ), b in [0,2µ); a D flip-flop takes a at each rising clock edge
        let r = run("Va a 0 PULSE(0 5 0 1n 1n 1u 2u)\nVb b 0 PULSE(0 5 0 1n 1n 2u 4u)\nVc c 0 PULSE(0 5 0.25u 1n 1n 0.5u 1u)\n\
            X1 a b y1 and2\nX2 a b y2 or2\nX3 a b y3 xor2\nX4 a b y4 nand2\nX5 a b y5 nor2\nX6 a y6 not\nX7 a c q qn dff\n.tran 1n 4u",
            &["and2", "or2", "xor2", "nand2", "nor2", "not", "dff"]);
        for (y, want) in [("y1", [5., 0., 0., 0.]), ("y2", [5., 5., 5., 0.]), ("y3", [0., 5., 5., 0.]), ("y4", [0., 5., 5., 5.]), ("y5", [0., 0., 0., 5.]), ("y6", [0., 5., 0., 5.])] {
            let v = tr(&r, &format!("v({y})"));
            let got = [0.5e-6, 1.5e-6, 2.5e-6, 3.5e-6].map(|t| (v(t) * 10.0).round() / 10.0);
            assert_eq!(got, want, "{y}");
        }
        let q = tr(&r, "v(q)");
        let got = [0.75e-6, 1.1e-6, 1.75e-6, 2.75e-6, 3.75e-6].map(|t| (q(t) * 10.0).round() / 10.0);
        assert_eq!(got, [5., 5., 0., 5., 0.], "q at the edges only");
        // a 50 Ω line of 100 ns between 50 Ω: half the step, 100 ns late; the lossy one (10 m of coax) runs
        let r = run("V1 s 0 PULSE(0 2 10n 1n 1n 1u 2u)\nRs s in 50\nT1 in 0 out 0 Z0=50 Td=100n\nRl out 0 50\n\
            V2 s2 0 PULSE(0 2 10n 1n 1n 1u 2u)\nRs2 s2 in2 50\nO1 in2 0 out2 0 LTRA\nRl2 out2 0 50\n.tran 1n 400n", &["ltra"]);
        let (out, out2) = (tr(&r, "v(out)"), tr(&r, "v(out2)"));
        assert!(out(80e-9).abs() < 0.01 && (out(200e-9) - 1.0).abs() < 0.02, "line {} {}", out(80e-9), out(200e-9));
        assert!(out2(30e-9).abs() < 0.05 && out2(300e-9) > 0.8, "lossy {} {}", out2(30e-9), out2(300e-9));
    }

    #[test]
    fn form_directives_simulate() {
        // what the directive forms write: .meas results reach the log, .pz and .tf give result tables
        let _s = SIM.lock();
        let bode = EXAMPLES.iter().find(|e| e.0 == "rc_bode").unwrap().1;
        let with = |d: &str| bode.replace(".ac dec 50 10 1Meg", d);
        let tran = with(".tran 5m\\n.meas tran vmax MAX V(out)").replace("SYMATTR Value AC 1", "SYMATTR Value PULSE(0 1 0 1n 1n 10m 20m)");
        let run = crate::ngspice::run(&prepare(&netlist(&parse_asc(&tran), &mut Library::builtin()).text, None, &mut vec![])).unwrap();
        assert!(run.log.iter().any(|l| l.to_lowercase().contains("vmax")), "{:?}", run.log);
        let pz = crate::ngspice::run(&prepare(&netlist(&parse_asc(&with(".pz in 0 out 0 vol pz")), &mut Library::builtin()).text, None, &mut vec![])).unwrap();
        // RC low-pass: one real pole at -1/(RC) = -10000 rad/s
        let pole = pz.op.iter().find(|o| o.0.starts_with("pole")).expect("a pole");
        assert!((pole.1 + 1e4).abs() < 1.0 && pole.2.abs() < 1e-6, "{:?}", pz.op);
        let tf = crate::ngspice::run(&prepare(&netlist(&parse_asc(&with(".tf V(out) V1")), &mut Library::builtin()).text, None, &mut vec![])).unwrap();
        assert!(tf.op.iter().any(|o| o.0.to_lowercase().contains("transfer_function") && (o.1 - 1.0).abs() < 1e-6), "{:?}", tf.op);
    }

    #[test]
    fn vary_sets_params_and_values() {
        let deck = "* x\n.param R = 1k Cx={2*R}\nR1 a b {R}\nC1 b 0 100n\n.end\n";
        let v = vary(deck, "R", 2000.0).unwrap();
        assert!(v.contains(".param R = 2e3 Cx={2*R}"), "{v}");
        assert!(vary(deck, "Cx", 1.0).unwrap().contains("Cx=1e0"));
        assert!(vary(deck, "c1", 1e-6).unwrap().contains("C1 b 0 1e-6"));
        assert_eq!(vary(deck, "R2", 1.0), None);
        assert!(vary("  .param a=1 µ=2\n", "a", 5.0).unwrap().contains("  .param a=5e0 µ=2"), "indented, non-ASCII");
        assert_eq!(vary(deck, "µ", 1.0), None);
        // a brace expression with spaces is one value; subcircuit bodies are not top-level instances
        assert!(vary("R1 a b {R * 2} tc=1\n", "R1", 5.0).unwrap().contains("R1 a b 5e0 tc=1"));
        assert_eq!(vary(".subckt opamp a b c\nR1 n 0 {Aol*1k}\n.ends opamp\n", "R1", 1.0), None);
        // every definition of a .param, and the .param before a same-named part
        let two = vary(".param X=1\nRx a b 1k\n.param X=2\n", "X", 3.0).unwrap();
        assert!(two.contains(".param X=3e0") && !two.contains("X=1") && !two.contains("X=2"), "{two}");
        assert!(vary(".param Rb=10k\nRb out inv {Rb}\n", "Rb", 2e4).unwrap().contains("Rb out inv {Rb}"));
        // `==` inside an expression and a name after a non-ASCII letter are not definitions
        let v = vary(".param y={x==1?2:3} x=5\n", "x", 7.0).unwrap();
        assert!(v.contains("y={x==1?2:3} x=7e0"), "{v}");
        assert!(vary(".param µa=1 a=2\n", "a", 3.0).unwrap().contains("µa=1 a=3e0"));
        assert!(vary(".param Ra=1k\n+ Rb=10k\nR5 a b 1k\n", "Rb", 2e4).unwrap().contains("+ Rb=2e4"), "continuation line");
        assert!(vary(".param Ra=1k\n* note\n+Rb=10k\n", "Rb", 2e4).unwrap().contains("+Rb=2e4"), "no space after +, comment between");
        assert!(vary(".param Ra=1k\n\n+ Rb=10k\n", "Rb", 2e4).unwrap().contains("+ Rb=2e4"), "blank line between");
        assert!(vary(".ends\nR1 a b 1k\n", "R1", 2.0).unwrap().contains("R1 a b 2e0"), "stray .ends");
        assert_eq!(vary(".param Rb=10k Rb=20k\n", "Rb", 3.0).unwrap(), ".param Rb=3e0 Rb=3e0\n", "every definition on a line");
        let lib = ".param R1=5\nR1 a b 1k\n"; // a library's .param named like the user's part: the part
        assert_eq!(splice(lib, &vary_spans(lib, "R1", false).unwrap(), 2.0), ".param R1=5\nR1 a b 2e0\n");
        assert_eq!(vary_spans(".param x=1\n", "x", false).map(|s| s.len()), Some(1), "no part: the .param still");
        assert!(sweepable(".param pi=3.14159\n.param a=1\n", &[]) == ["a"], "built-in pi not offered");
        // what the root-locus panel offers: the same rules
        let placed = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(sweepable(".param a=1 b = {a*2} ; c=3\n.subckt x 1 2\nR9 1 2 1k\n.ends\nR1 n1 n2 1k\nC1 n2 0 1n\nV1 n1 0 1\n", &placed(&["R9", "R1", "C1", "V1"])), ["a", "b", "R1", "C1"]);
        // comments are not definitions: the part R1 is varied
        let c = vary(".param a=1 ; R1=5\n* R1=old value\nR1 x y 1k\n", "R1", 2e3).unwrap();
        assert!(c.contains("R1 x y 2e3") && c.contains("; R1=5") && c.contains("* R1=old value"), "{c}");
        assert!(vary("R1 a b 1k\n+ tc=1\n", "tc", 2.0).is_none(), "a part's continuation is not a .param");
        for l in [".param x = 5\n", ".param x =5\n", ".param x= 5\n"] {
            assert!(vary(l, "x", 7.0).unwrap().contains("7e0") && !vary(l, "x", 7.0).unwrap().contains('5'), "{l}");
        }
        // a number cannot stand in for a model name or an `R=` form: not a target
        for l in ["R1 a b R=V(c)*1k\n", "R1 a b rmod L=10u\n", "C1 a b Q=1u*V(a)\n"] {
            assert_eq!(vary(l, &l[..2], 1.0), None, "{l}");
            assert!(sweepable(l, &placed(&["R1", "C1"])).is_empty(), "{l}");
        }
        assert!(vary("R1 a b 'x*2'\n", "R1", 1.0).unwrap().contains("R1 a b 1e0"));
        assert_eq!(sweepable("C1 a c 1u\nRC1_rser a b 0.1\n", &placed(&["c1"])), ["C1"], "netlister parasitics");
        assert!(vary("R1 a b -1k\n", "R1", 2.0).unwrap().contains("R1 a b 2e0"), "signed value");
        assert_eq!(pz_statement("R1 a b 1k\n.PZ in 0 out 0 vol\n* c\n+ zer\nR2 b 0 1k\n+ x\n").as_deref(), Some(".pz in 0 out 0 vol zer"));
        assert_eq!(pz_statement(".tran 1m\n.pzx 1\n"), None);
        assert_eq!(vary(".param a= a=1\n", "a", 2.0).unwrap(), ".param a= 2e0\n", "nested spans: no panic");
        // a sweep step runs only .pz: other analyses go, with their continuation lines
        let only = prepare(".param a=1\n.meas tran x AVG V(out)\n* c\n+ FROM=1m TO=2m\n.pz in 0 out 0 vol pz\n+ \n", None, &mut vec![]).only_pz();
        assert!(only.text().contains("* + FROM=1m") && only.text().contains("\n.pz in 0") && !only.text().contains("* .pz"), "{}", only.text());
        // RC low-pass: the pole moves with R (-1/RC)
        let _s = SIM.lock();
        let bode = EXAMPLES.iter().find(|e| e.0 == "rc_bode").unwrap().1.replace(".ac dec 50 10 1Meg", ".pz in 0 out 0 vol pz");
        let nl = netlist_for_sim(&parse_asc(&bode), &mut Library::builtin());
        let p = prepare(&nl.text, None, &mut vec![]).with_sensors(nl.sensors.clone());
        let run = crate::ngspice::run(&p.at(&vary_spans(p.text(), "R1", true).unwrap(), 2000.0)).unwrap();
        let pole = run.op.iter().find(|o| o.0.starts_with("pole")).unwrap();
        assert!((pole.1 + 5000.0).abs() < 1.0, "{:?}", run.op);
    }

    #[test]
    fn ltspice_meas_for_ngspice() {
        let r1 = [Sensor { source: "vsense_r1".into(), node: "x".into(), inst: "r1".into(), pin: 0, label: "I(R1)".into(), name: "r1".into(), hold: String::new(), alias: String::new() }];
        let m = |s: &str| meas_statement(s, &r1, "tran").unwrap_or_else(|e| e);
        assert_eq!(m(".MEAS TRAN P AVG -I(V1)*V(VDD)"), ".MEAS TRAN P AVG par('-I(V1)*V(VDD)')");
        assert_eq!(m(".meas tran t TRIG V(in) VAL=0.5*V(vdd) TD=2n RISE=1 TARG V(out) VAL=1 RISE=1"),
            ".meas tran t TRIG par('V(in)-(0.5*V(vdd))') VAL=0 TD=2n RISE=1 TARG V(out) VAL=1 RISE=1");
        assert_eq!(m(".meas x FIND I(R1) AT=1m"), ".meas tran x FIND i(vsense_r1) AT=1m", "LTspice's default analysis, part current");
        assert_eq!(m(".meas tran w WHEN V(out)=0.5*V(vdd) RISE=1"), ".meas tran w WHEN par('V(out)-(0.5*V(vdd))')=0 RISE=1");
        for same in [".meas tran w WHEN V(out)=1 RISE=1", ".meas tran a AVG V(out) FROM=1m TO=2m", ".meas tran t TRIG AT=1n TARG V(out) VAL={0.5*2} RISE=1", ".meas ac bw WHEN vdb(out)=-3", ".meas ac g MAX vm(out)"] {
            assert_eq!(m(same), same);
        }
        assert_eq!(sensor_currents("I(R1)+I(R10)+II(R1)", &r1).0, "i(vsense_r1)+I(R10)+II(R1)");
        // subcircuit pins: by name, by number where names repeat or do not read as one; I(U1) is none
        let pin = |n: &str| Pin { name: n.into(), ..Default::default() };
        let pins = [pin("IN"), pin("GND"), pin("GND"), pin("a)b")];
        assert_eq!(pin_labels('X', "U1", &pins), ["Ix(U1:1)", "Ix(U1:2)", "Ix(U1:3)", "Ix(U1:4)"]);
        assert_eq!(pin_labels('X', "U1", &[pin("IN+"), pin("IN-"), pin("OUT")]), ["Ix(U1:IN+)", "Ix(U1:IN-)", "Ix(U1:OUT)"]);
        assert_eq!(pin_labels('X', "U1", &[pin("2"), pin("GND"), pin("GND")]), ["Ix(U1:1)", "Ix(U1:2)", "Ix(U1:3)"], "never two alike");
        assert_eq!(pin_labels('Q', "Q1", &[pin("C"), pin("B"), pin("E")]), ["Ic(Q1)", "Ib(Q1)", "Ie(Q1)"]);
        let ic = [Sensor { source: "vsense_xu1".into(), node: "y".into(), inst: "xu1".into(), pin: 0, label: "Ix(U1:IN)".into(), name: "u1".into(), hold: String::new(), alias: String::new() }];
        let (text, why) = sensor_currents("I(U1)+ix(u1:in)", &ic);
        assert_eq!(text, "I(U1)+i(vsense_xu1)");
        assert!(why.is_some_and(|w| w.contains("one per pin")), "a subcircuit's I(U1)");
        assert!(sensor_currents("I(XU1)", &ic).1.is_some(), "by the netlist's name too");
        let problem = |e: &str| sensor_currents(e, &r1).1;
        assert!(problem("v(a)*Ib(Q9)").is_some_and(|w| w.starts_with("Ib(Q9)")));
        assert_eq!(problem("i(v1)+mix(1)+vin(a)+ig(v1)"), None, "a source's i(), other words, ngspice's group delay");
        assert!(problem("1µ*Ω°ib(q9)").is_some(), "after non-ASCII: no panic, still a word of its own");
        assert_eq!(m(".meas tran p AVG 1µ*I(R1)"), ".meas tran p AVG par('1µ*i(vsense_r1)')", "multi-byte text: no panic");
        assert_eq!(m(".meas tran a AVG V(out) FROM = 1m TO =2m"), ".meas tran a AVG V(out) FROM=1m TO=2m", "blanks around =");
        assert_eq!(m(".meas tran w WHEN V(out) = 0.5 RISE = 1"), ".meas tran w WHEN V(out)=0.5 RISE=1");
        assert_eq!(m(".meas tran p AVG -I(V1)*V(in) ; dynamic power"), ".meas tran p AVG par('-I(V1)*V(in)')", "a ; comment is not part of it");
        assert_eq!(m(".meas tran p AVG V(a)*2\t$ avg power"), ".meas tran p AVG par('V(a)*2')", "a $ comment after a tab");
        assert_eq!(m(".meas tran p AVG V(a)*2 // note"), ".meas tran p AVG par('V(a)*2')", "a // comment");
        for same in [".meas tran x AVG par('v(a)*v(b)')", ".meas ac x WHEN vdb(out)=vdb(in)"] {
            assert_eq!(m(same), same, "ngspice's own forms stay");
        }
        assert_eq!(m(".meas tran t TRIG V(in)=0.5*V(vdd) RISE=1 TARG V(out)=1 RISE=1"),
            ".meas tran t TRIG par('V(in)-(0.5*V(vdd))') VAL=0 RISE=1 TARG V(out) VAL=1 RISE=1", "LTspice's [VAL]= form");
        assert_eq!(m(".meas tran t TRIG V(in) TD=2n VAL=0.5*V(vdd) RISE=1 TARG V(out) VAL=1"),
            ".meas tran t TRIG par('V(in)-(0.5*V(vdd))') TD=2n VAL=0 RISE=1 TARG V(out) VAL=1", "VAL after TD=");
        assert_eq!(m(".meas ac g MAX mag(V(out))"), ".meas ac g MAX vm(out)", "AC: no par() (linearized), ngspice's vector form");
        assert_eq!(m(".meas ac bw WHEN db(V(out))=-3"), ".meas ac bw WHEN vdb(out)=-3");
        assert_eq!(m(".meas ac r MAX V(out)/V(in)"), ".meas ac r MAX V(out)/V(in)", "AC: as written rather than wrong");
        assert!(m(".meas ac f1 TRIG vm(out) VAL=0.5*vm(in) TARG vm(out) VAL=1").contains("left out"), "AC: an expression level is not silently cut");
        assert!(m(".meas ac bw WHEN vm(out)=0.707*vm(in)").contains("left out"), "the same for WHEN");
        assert_eq!(m(".meas tran t TRIG V(in) VAL=0.5 * V(vdd) RISE=1 TARG V(out) VAL=1"), ".meas tran t TRIG par('V(in)-(0.5*V(vdd))') VAL=0 RISE=1 TARG V(out) VAL=1", "a level with blanks");
        assert_eq!(m(".meas tran t TRIG V(in)=0.5 * V(vdd) RISE=1"), ".meas tran t TRIG par('V(in)-(0.5*V(vdd))') VAL=0 RISE=1");
        assert_eq!(meas_statement(".meas x AVG V(a)", &[], "op").unwrap(), ".meas x AVG V(a)", "ngspice measures tran/ac/dc/sp only");
        assert_eq!(m(".meas tran t TRIG par('v(a)') VAL=0.5 * V(vdd) RISE=1"), ".meas tran t TRIG par('v(a)-(0.5*V(vdd))') VAL=0 RISE=1", "no par() in par(), nothing cut");
        assert_eq!(m(".meas tran w WHEN par('v(a)*v(b)')=0.5*V(vdd)"), ".meas tran w WHEN par('v(a)*v(b)-(0.5*V(vdd))')=0", "WHEN too");
        assert_eq!(m(".meas tran t WHEN V(out)=vth"), ".meas tran t WHEN par('V(out)-(vth)')=0", "a .param level resolves in par()");
        assert_eq!(m(".meas tran ratio PARAM tplh / tphl"), ".meas tran ratio param='tplh/tphl'", "LTspice's PARAM");
        assert_eq!(m(".meas tran t TRIG V(in) VAL='0.5*V(vdd)' RISE=1"), ".meas tran t TRIG par('V(in)-(0.5*V(vdd))') VAL=0 RISE=1", "a quoted level: no nested quotes");
        assert_eq!(m(".meas sp g MAX V(out)/V(in)"), ".meas sp g MAX V(out)/V(in)", "SP: no par() either (linearized)");
        assert_eq!(m(".meas ac x WHEN mag(V(out))=mag(V(in))"), ".meas ac x WHEN vm(out)=vm(in)", "both sides as ngspice vectors");
        assert_eq!(m(".meas ac p FIND ph(V(out)) AT=1k"), ".meas ac p FIND ph(V(out)) AT=1k", "degrees vs radians: not mapped");
        let mut w = vec![];
        let skipped = ltspice_meas(".ac dec 10 1 1k\n.meas ac f1 TRIG vm(out) VAL=0.5*vm(in) TARG vm(out) VAL=1\n", &[], &mut w);
        assert!(skipped.contains("* .meas ac f1") && w.len() == 1, "{skipped} {w:?}");
        assert_eq!(par_calls(".func rpar(a,b) {a*b}\n.meas x AVG par('v(a)') PAR( 'x')"), 2, "par( as a word only");
        // ngspice exits past 99 par() calls: the rewrite stops there
        let many: String = (0..120).map(|k| format!(".meas tran m{k} AVG V(a)*{k}\n")).collect();
        let mut w = vec![];
        let capped = ltspice_meas(&format!(".tran 1m\n{many}"), &[], &mut w);
        assert!(capped.matches("par(").count() == 99 && w.len() == 21 && capped.contains("* .meas tran m119 AVG V(a)*119"), "{} {w:?}", capped.matches("par(").count()); // 21 left out, each named
        let cont = ltspice_meas(".tran 1m\n.meas tran p AVG -I(V1)*V(in) ; power\n+ FROM=1m TO=2m\n.meas tran q AVG V(a)*2 $ note\n", &[], &mut vec![]);
        assert!(cont.contains(".meas tran p AVG par('-I(V1)*V(in)') FROM=1m TO=2m") && cont.contains(".meas tran q AVG par('V(a)*2')\n"), "comments cut per line: {cont}");
        // the manual's delay and power measurements, end to end
        let deck = "* m\nV1 in 0 PULSE(0 2 0 1n 1n 4n 10n)\nV2 vdd 0 2\nR1 out in 100\nC1 out 0 10p\n.tran 4e-11 40n 0.1n\n.MEAS TRAN TpLH\n* note\n+TRIG V(in) VAL=0.5*V(vdd) TD=22n RISE=1\n+TARG V(out) VAL=0.5*V(vdd) TD=22n RISE=1\n.meas tran pdyn AVG -I(V1)*V(in)\n.meas tran vmax MAX V(out)\n+ FROM=0 TO=10n\n.end\n";
        let text = ltspice_meas(&prepare(deck, None, &mut vec![]).0, &[], &mut vec![]);
        assert!(text.contains(".MEAS TRAN TpLH TRIG par('V(in)-(0.5*V(vdd))') VAL=0 TD=22n RISE=1 TARG par('V(out)-(0.5*V(vdd))') VAL=0") && text.contains("* note"), "{text}");
        assert!(text.contains(".meas tran vmax MAX V(out)\n+ FROM=0 TO=10n"), "unchanged: as written\n{text}");
        let _s = SIM.lock();
        let run = crate::ngspice::run(&Prepared(text, vec![], vec![])).unwrap();
        let get = |n: &str| run.meas.iter().find(|m| m.0 == n).and_then(|m| m.1).unwrap_or_else(|| panic!("{n}: {:?}\n{}", run.meas, run.log.join("\n")));
        assert!((get("tplh") / 7.28e-10 - 1.0).abs() < 0.02, "{:?}", run.meas); // V(in) at 1 V: 30.5 ns
        assert!((get("pdyn") / 2.9e-3 - 1.0).abs() < 0.03, "{:?}", run.meas);
        assert!(get("vmax") > 1.9);
        assert!(!run.vectors.iter().any(|v| v.contains("pa_")), "par() helpers hidden: {:?}", run.vectors);
        let one = crate::ngspice::trace("1", &run.scale, run.id).unwrap();
        assert!(one.re.len() == one.x.len() && one.x.len() > 10 && one.re.iter().all(|&v| v == 1.0), "a constant spans the x axis");
        let half = crate::ngspice::trace("2*0.5/2", &run.scale, run.id).unwrap(); // a constant expression too
        assert!(one.constant && half.constant && half.re[0] == 0.5, "constants are flagged");
        assert!(!crate::ngspice::trace("v(in)", &run.scale, run.id).unwrap().constant);
        assert!(!crate::ngspice::trace("mean(v(in))", &run.scale, run.id).unwrap().constant, "a measured scalar keeps its own pane");
        assert!(crate::ngspice::trace("(1+2)*0.5", &run.scale, run.id).unwrap().constant);
    }

    #[test]
    fn folder_symbols_and_prefix_override() {
        let dir = std::env::temp_dir().join(format!("nomospice-sym-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("nmos_018.asy"), BUILTIN_SYMBOLS.iter().find(|(n, _)| *n == "nmos").unwrap().1).unwrap();
        let mut lib = Library::builtin();
        lib.set_dir(Some(&dir), false);
        assert!(lib.defs.contains_key("nmos_018"), "the folder's symbols are listed");
        // the manual's step: Prefix X calls the subcircuit NM from the model library
        let attrs: BTreeMap<String, String> = [("InstName", "M1"), ("Value", "NM"), ("Value2", "l=0.18u w=1u"), ("Prefix", "X")].map(|(k, v)| (k.to_string(), v.to_string())).into();
        let nl = netlist(&[Item::Sym { sym: "nmos_018".into(), x: 0, y: 0, rot: "R0".into(), attrs, extra: vec![] }], &mut lib);
        let line = nl.text.lines().find(|l| l.starts_with("XM1 ")).unwrap_or_else(|| panic!("{}", nl.text));
        assert!(line.ends_with(" NM l=0.18u w=1u"), "{line}");
        lib.set_dir(None, false);
        assert!(!lib.defs.contains_key("nmos_018"), "another schematic: gone");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn hierarchy() {
        let _s = SIM.lock();
        let dir = std::env::temp_dir().join(format!("nomospice-hier-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let block = |pins: &[&str]| {
            let mut s = "Version 4\nSymbolType BLOCK\nRECTANGLE Normal -32 -32 32 32\n".to_string();
            for (k, p) in pins.iter().enumerate() {
                s += &format!("PIN {} 0 NONE 8\nPINATTR PinName {p}\nPINATTR SpiceOrder {}\n", k as i32 * 64 - 32, k + 1);
            }
            s
        };
        // a divider of R={R} (its own .param 1k) over 1k; its .tran is not run (the top's .op is)
        let div = "Version 4\nSHEET 1 880 680\nFLAG 16 16 in\nIOPIN 0 0 In\nFLAG 16 96 out\nIOPIN 16 96 Out\nFLAG 160 16 OUT\nIOPIN 160 16 Out\nFLAG 160 96 0\n\
SYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMATTR Value {R}\nSYMBOL res 144 0 R0\nSYMATTR InstName R2\nSYMATTR Value 1k\n\
TEXT 0 200 Left 2 !.param R=1k\\n.tran 1m\\n.control\\nrun\\n.endc\\n.global vcc\\n+ vdd\\n.include m.lib\n";
        assert!(write_asc(&parse_asc(div)).contains("FLAG 16 16 in\nIOPIN 16 16 In\n"), "a port's IOPIN where its label is");
        // its symbol from its ports: in left, out right (once), as pins of those names
        let asy = block_asy(&parse_asc(div)).unwrap();
        let d = parse_asy("div", &asy);
        assert!(d.block && d.pins.iter().map(|p| (p.name.as_str(), p.x < 0, p.just.as_str())).eq([("in", true, "LEFT"), ("out", false, "RIGHT")]), "{asy}");
        assert!(d.pins.iter().all(|p| p.x % 16 == 0 && p.y % 16 == 0), "on the grid: {asy}");
        assert!(block_asy(&parse_asc("Version 4\nFLAG 0 0 a\n")).is_err(), "no ports: no symbol");
        assert!(block_asy(&parse_asc("Version 4\nFLAG 0 0 GND\nIOPIN 0 0 BiDir\nFLAG 0 0 a\nIOPIN 0 0 In\n")).is_ok_and(|a| !a.contains("GND")), "ground is no pin");
        assert!(!parse_asy("x", "SYMATTR Prefix X\nSYMATTR Value x\nSYMATTR ModelFile x.lib\n").calls_sheet(), "a library subcircuit's symbol");
        assert!(!parse_asy("x", "SymbolType BLOCK\nSYMATTR SpiceModel x.sub\n").calls_sheet(), "a library block symbol");
        std::fs::write(dir.join("div.asy"), &asy).unwrap();
        std::fs::write(dir.join("Div.asc"), div).unwrap();
        // a sheet of no .param of its own: an instance passing none gets its caller's
        std::fs::write(dir.join("half.asy"), &asy).unwrap();
        // (with a diode of a model its included file has too: that one wins, as in the sheet itself)
        let diode = "FLAG 400 0 0\nFLAG 400 64 0\nSYMBOL diode 384 0 R0\nSYMATTR InstName D1\nSYMATTR Value 1N4148\nSYMBOL res 0 0 R0";
        std::fs::write(dir.join("half.asc"), div.replace("!.param R=1k\\n", "!").replace("SYMBOL res 0 0 R0", diode)).unwrap();
        // a block holding itself (through another): said, not looped
        std::fs::write(dir.join("ping.asy"), block(&["a"])).unwrap();
        std::fs::write(dir.join("ping.asc"), "Version 4\nFLAG -32 0 a\nSYMBOL pong 0 0 R0\nSYMATTR InstName U1\n").unwrap();
        std::fs::write(dir.join("pong.asy"), block(&["a"])).unwrap();
        std::fs::write(dir.join("pong.asc"), "Version 4\nFLAG -32 0 a\nSYMBOL ping 0 0 R0\nSYMATTR InstName U1\n").unwrap();
        std::fs::write(dir.join("m.lib"), ".param unused=1\n.model 1N4148 D(IS=1e-9)\n").unwrap();
        // (one including nothing: a parameter its caller lacks is said; an included file may have it)
        std::fs::write(dir.join("bare.asy"), &asy).unwrap();
        std::fs::write(dir.join("bare.asc"), div.replace("!.param R=1k\\n", "!").replace("\\n.include m.lib", "")).unwrap();
        // a library part's symbol beside its test bench, which places it: no block, quietly
        std::fs::write(dir.join("bench.asy"), "Version 4\nSymbolType CELL\nSYMATTR Prefix X\nSYMATTR Value bench\nPIN 0 0 NONE 0\nPINATTR PinName a\n").unwrap();
        std::fs::write(dir.join("bench.asc"), "Version 4\nFLAG 0 0 a\nSYMBOL bench 0 0 R0\nSYMATTR InstName U1\n").unwrap();
        // its includes and sections stay with its parts
        std::fs::write(dir.join("secs.asy"), &asy).unwrap();
        std::fs::write(dir.join("secs.asc"), "Version 4\nFLAG 0 0 in\nFLAG 0 64 out\nTEXT 0 200 Left 2 !.lib m.lib\\nR1 in out 1k\\n.lib mysect\\n.model DX D\\n.endl\n").unwrap();
        // a subcircuit symbol without a schematic: its Value as written
        std::fs::write(dir.join("amp.asy"), "Version 4\nSymbolType CELL\nSYMATTR Prefix X\nSYMATTR Value Amp\nPIN 0 0 NONE 0\n").unwrap();
        // a schematic only sharing a symbol's name (no label of its pin): no block
        std::fs::write(dir.join("other.asy"), block(&["a"])).unwrap();
        std::fs::write(dir.join("other.asc"), "Version 4\nFLAG 0 0 b\n").unwrap();
        let top = "Version 4\nSHEET 1 880 680\nFLAG 0 16 a\nFLAG 0 96 0\nSYMBOL voltage 0 0 R0\nSYMATTR InstName V1\nSYMATTR Value 1\n\
SYMBOL div 200 100 R0\nSYMATTR InstName U1\nFLAG 152 100 a\nFLAG 248 100 b\n\
SYMBOL div 400 100 R0\nSYMATTR InstName U2\nSYMATTR Value R = 3k\nFLAG 352 100 a\nFLAG 448 100 c\n\
SYMBOL half 200 300 R0\nSYMATTR InstName U3\nFLAG 152 300 a\nFLAG 248 300 d\n\
SYMBOL half 400 300 R0\nSYMATTR InstName U4\nSYMATTR Value R=3k\nFLAG 352 300 a\nFLAG 448 300 e\nTEXT 0 500 Left 2 !.op\\n.param R=1k\n";
        let mut lib = Library::builtin();
        lib.set_dir(Some(&dir), false);
        assert!(lib.get("div").is_some_and(|d| d.sheet.as_deref().is_some_and(|p| p.ends_with("Div.asc"))), "the block's sheet (any case)");
        let nl = netlist_for_sim(&parse_asc(top), &mut lib);
        assert!(nl.sheets[4].ends_with("Div.asc") && nl.sheets[3].is_empty(), "{:?}", nl.sheets);
        let t = &nl.text;
        assert_eq!(t.matches(".subckt sheet_div ").count(), 1, "one definition for both: {t}");
        assert!(t.contains(".subckt sheet_div in out params: r=0\n") && t.contains("\n.param R=1k\n") && !t.contains(".tran"), "{t}");
        assert!(t.lines().any(|l| l.starts_with("XU2 ") && l.ends_with(" sheet_div R = 3k")), "{t}");
        assert!(!nl.warnings.iter().any(|w| w.starts_with("Div.asc")), "its own test bench unsaid: {:?}", nl.warnings);
        assert!(!t.contains("\nrun\n") && t.matches(".global vcc\n+ vdd\n").count() == 1 && t.find(".global vcc") < t.find(".subckt"), "no .control, one .global at the top: {t}");
        assert!(t.matches(".include m.lib").count() == 2 && t.find(".include m.lib") > t.find(".subckt"), "a sheet's include in its subcircuit: {t}");
        assert!(t.matches(".model 1N4148").count() == 1 && t.find(".model 1N4148") < t.find(".subckt"), "a sheet's library model at the top (its include's, in its subcircuit, wins there): {t}");
        let prepared = prepare(t, Some(&dir), &mut vec![]);
        assert_eq!(prepared.text().matches("IS=1e-9").count(), 2, "the file inlined in each block's subcircuit: {}", prepared.text());
        let run = crate::ngspice::run(&prepare(t, Some(&dir), &mut vec![]).with_sensors(nl.sensors.clone())).unwrap_or_else(|l| panic!("{l:?}\n{t}"));
        let v = |n: &str| run.op.iter().find(|o| o.0 == n).unwrap_or_else(|| panic!("{n}: {:?}", run.op)).1;
        assert!((v("b") - 0.5).abs() < 1e-9 && (v("c") - 0.25).abs() < 1e-9, "the sheet's default, the passed R: {:?}", run.op);
        assert!((v("d") - 0.5).abs() < 1e-9 && (v("e") - 0.25).abs() < 1e-9, "the caller's R, the passed R: {:?}", run.op);
        let nl = netlist(&parse_asc("Version 4\nSYMBOL ping 0 0 R0\nSYMATTR InstName U1\nSYMBOL other 0 100 R0\nSYMATTR InstName U2\n"), &mut lib);
        assert!(nl.warnings.iter().any(|w| w.starts_with("ping.asc places its own block")), "{:?}", nl.warnings);
        assert!(!nl.text.contains(".subckt sheet_ping") && !nl.text.contains(".subckt sheet_pong") && nl.text.lines().any(|l| l.starts_with("XU1 ") && l.ends_with(" ping")), "a loop of sheets is no subcircuit (no endless one), its parts called by name: {}", nl.text);
        assert!(nl.warnings.iter().any(|w| w.starts_with("pong's schematic places itself")) && !nl.warnings.iter().any(|w| w.contains("`ping`")), "the loop said, no missing library: {:?}", nl.warnings);
        assert!(nl.warnings.iter().any(|w| w.starts_with("other.asc has no label a")) && !nl.text.contains(".subckt sheet_other"), "{:?}", nl.warnings);
        // a parameter the caller lacks: said (ngspice stops on it)
        let bare = top.replace("\\n.param R=1k", "").replace("SYMBOL half", "SYMBOL bare").replace("SYMBOL div", "SYMBOL bare");
        let nl = netlist(&parse_asc(&bare), &mut lib);
        assert!(nl.warnings.iter().any(|w| w.starts_with("XU1 passes no r")), "{:?}", nl.warnings);
        let nl = netlist(&parse_asc(&top.replace("\\n.param R=1k", "")), &mut lib);
        assert!(nl.warnings.iter().any(|w| w.starts_with("XU3 passes no r")), "a sheet's include is its own: {:?}", nl.warnings);
        let nl = netlist(&parse_asc("Version 4\nSYMBOL div 0 0 R0\nSYMATTR InstName U1\n"), &mut lib);
        assert!(!nl.warnings.iter().any(|w| w.contains("no ground")), "grounded in its sheet: {:?}", nl.warnings);
        // (the top's own .subckt of the name a block would get: the block takes another)
        let t = netlist(&parse_asc("Version 4\nSYMBOL div 0 0 R0\nSYMATTR InstName U1\nTEXT 0 0 Left 2 !.subckt sheet_div a b\nR1 a b 1\n.ends\n"), &mut lib).text;
        assert!(t.contains(".subckt sheet_div_2 ") && t.contains("XU1 ") && t.lines().any(|l| l.starts_with("XU1 ") && l.contains(" sheet_div_2")), "{t}");
        // (a model inside a .subckt of the top's own is no sheet's: the library's is pulled)
        let t = netlist(&parse_asc("Version 4\nSYMBOL half 0 0 R0\nSYMATTR InstName U1\nTEXT 0 0 Left 2 !.subckt mysub a\\n.model 1N4148 D\\n.ends\n"), &mut lib).text;
        assert!(t.contains(".model 1N4148 D(IS="), "{t}");
        let nl = netlist(&parse_asc("Version 4\nSYMBOL bench 0 0 R0\nSYMATTR InstName U1\n"), &mut lib);
        assert!(!nl.text.contains(".subckt sheet_bench") && nl.text.lines().any(|l| l.starts_with("XU1 ") && l.ends_with(" bench")) && !nl.warnings.iter().any(|w| w.contains("bench.asc")), "{:?}\n{}", nl.warnings, nl.text);
        let t = netlist(&parse_asc("Version 4\nSYMBOL secs 0 0 R0\nSYMATTR InstName U1\n"), &mut lib).text;
        let sub = t.find(".subckt sheet_secs").unwrap_or_else(|| panic!("{t}"));
        assert!(t.find(".lib mysect") > Some(sub) && t.find(".lib m.lib") > Some(sub) && t.find("R1 in out 1k") > Some(sub), "{t}");
        let nl = netlist(&parse_asc("Version 4\nSYMBOL amp 0 0 R0\nSYMATTR InstName U1\n"), &mut lib);
        assert!(nl.text.lines().any(|l| l.starts_with("XU1 ") && l.ends_with(" Amp")), "{}", nl.text);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn symbol_roundtrip() {
        let key = |d: &SymbolDef| format!("{:?}", (&d.prefix, &d.value, &d.desc, &d.spice_line, &d.shapes, &d.windows, d.block, d.pins.iter().map(|p| (p.x, p.y, &p.name, &p.just, p.off)).collect::<Vec<_>>()));
        for (name, src) in BUILTIN_SYMBOLS {
            let again = write_asy(&asy_items(src));
            assert_eq!(key(&parse_asy(name, &again)), key(&parse_asy(name, src)), "{name}:\n{again}");
        }
        // a pin moved, one added: where it is now, numbered on
        let mut items = asy_items(BUILTIN_SYMBOLS[0].1);
        if let Some(Item::Flag { x, .. }) = items.iter_mut().find(|it| matches!(it, Item::Flag { .. })) {
            *x += 32;
        }
        items.push(Item::Flag { x: 0, y: 48, name: "C".into(), extra: vec![] });
        let d = parse_asy("res", &write_asy(&items));
        assert_eq!(d.pins.iter().map(|p| (p.x, p.name.as_str())).collect::<Vec<_>>(), [(48, "A"), (16, "B"), (0, "C")]);
        // tabs and doubled spaces read as words
        let d = parse_asy("x", &write_asy(&asy_items("PIN 0 0 NONE 8\nPINATTR\tPinName\tA\nSYMATTR  Value  10k\nTEXT  -32 0 Left 2 hi\n")));
        assert_eq!((d.pins[0].name.as_str(), d.value.as_str()), ("A", "10k"));
        assert!(write_asy(&asy_items("TEXT  -32 0 Left 2 hi\nTEXT 0 0 Left 2   note\nSymbolType\n")).contains("TEXT -32 0 Left 2 hi\nTEXT 0 0 Left 2   note\n"));
        // a copy of a pin, its number with it: the next free one
        let a = items.iter().find(|it| matches!(it, Item::Flag { .. })).cloned().unwrap();
        items.push(match a { Item::Flag { extra, .. } => Item::Flag { x: 0, y: 64, name: "D".into(), extra }, _ => unreachable!() });
        let d = parse_asy("res", &write_asy(&items));
        assert_eq!(d.pins.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["A", "B", "C", "D"]);
        // a pin pasted into a schematic: a label there
        assert!(!write_asc(&items).contains("PIN"), "{}", write_asc(&items));
    }

    #[test]
    fn run_until_stops_mid_run() {
        let _s = SIM.lock();
        // a coarse print step (little memory), a tiny max step: it would run for hours
        let long = prepare("* long\nV1 a 0 SINE(0 1 1k)\nR1 a b 1k\nC1 b 0 1u\n.tran 1m 10 0 1n\n.end\n", None, &mut vec![]);
        assert!(crate::ngspice::run_until(&long, &|| true).is_err(), "stopped before it starts");
        let t = std::time::Instant::now();
        let r = crate::ngspice::run_until(&long, &|| t.elapsed() > std::time::Duration::from_millis(200));
        assert!(t.elapsed() < std::time::Duration::from_secs(10), "bg_halt mid-run: {:?}", t.elapsed());
        assert!(r.is_ok_and(|r| r.aborted), "partial data, marked as such");
        assert!(crate::ngspice::run(&prepare("* ok\nR1 a 0 1k\nI1 0 a 1m\n.op\n.end\n", None, &mut vec![])).is_ok(), "the next run works");
    }

    #[test]
    fn sym_key_handles_multibyte_names() {
        assert_eq!(sym_key("Opamps\\µA741x"), "µa741x");
        assert_eq!(sym_key("abcdeµfgh"), "abcdeµfgh"); // byte 7 inside `µ`: no panic
        assert_eq!(sym_key("SUBCKT:BS/MMBF170"), "subckt:bs/mmbf170");
    }

    #[test]
    fn ac_without_ac_source_warns() {
        let bode = EXAMPLES.iter().find(|e| e.0 == "rc_bode").unwrap().1;
        let warns = |src: &str| netlist(&parse_asc(src), &mut Library::builtin()).warnings.iter().any(|w| w.starts_with(".ac: no source"));
        assert!(!warns(bode));
        assert!(warns(&bode.replace("SYMATTR Value AC 1", "SYMATTR Value SINE(0 1 1k)")));
        // a source in a directive counts
        assert!(!warns(&bode.replace("SYMATTR Value AC 1", "SYMATTR Value 0").replace("TEXT -32 232 Left 2 !.ac", "TEXT -32 300 Left 2 !V9 x 0 AC 1\nTEXT -32 232 Left 2 !.ac")));
    }

    #[test]
    fn noise_shows_spectrum_and_totals() {
        let _s = SIM.lock();
        let src = EXAMPLES.iter().find(|e| e.0 == "rc_bode").unwrap().1.replace(".ac dec 50 10 1Meg", ".noise v(out) V1 dec 10 10 1Meg");
        let nl = netlist(&parse_asc(&src), &mut Library::builtin());
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).unwrap();
        assert_eq!((run.plot.as_str(), run.scale.as_str()), ("noise1", "frequency"));
        assert!(run.op.iter().any(|(n, ..)| n == "onoise_total"), "{:?}", run.op);
        assert!(crate::ngspice::trace("onoise_spectrum", &run.scale, run.id).unwrap().re.len() > 10);
    }

    #[test]
    fn recovers_after_fatal_error() {
        let _s = SIM.lock();
        let bad = "* t\nV1 a 0 1\nR1 a 0 {Rundefined}\n.op\n.end\n";
        let log = crate::ngspice::run(&prepare(bad, None, &mut vec![])).expect_err("must fail"); // fatal: ngspice wants to be detached
        assert!(log.iter().any(|l| l.contains("fatal error")), "{log:?}");
        let good = netlist(&parse_asc(EXAMPLES[0].1), &mut Library::builtin());
        let run = crate::ngspice::run(&prepare(&good.text, None, &mut vec![]));
        assert!(run.as_ref().is_ok_and(|r| r.scale == "time"), "{run:?}");
    }

    #[test]
    fn step_statements() {
        let v = |s: &str| parse_step(s).map(|p| p.values).unwrap_or_else(|e| panic!("{e}"));
        let near = |a: Vec<f64>, b: &[f64]| assert!(a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x / y - 1.0).abs() < 1e-9), "{a:?} {b:?}");
        near(v(".step param R 1k 3k 1k"), &[1e3, 2e3, 3e3]);
        near(v(".step param R 3k 1k -1k"), &[3e3, 2e3, 1e3]);
        near(v(".step param C list 100n 1u 10u"), &[100e-9, 1e-6, 10e-6]);
        let d = v(".step dec param R 1k 100k 2");
        assert!(d.len() == 5 && (d[1] / 3162.28 - 1.0).abs() < 1e-4 && (d[4] / 1e5 - 1.0).abs() < 1e-9, "{d:?}");
        near(v(".step oct param f 1 8 1"), &[1.0, 2.0, 4.0, 8.0]);
        assert_eq!(parse_step(".step V1 0 5 1").unwrap().what, Stepped::Source("V1".into()));
        assert_eq!(parse_step(".step temp list -40 25 85").unwrap().what, Stepped::Temp);
        assert!(parse_step(".step NPN 2N2222(VAF) 50 100 25").unwrap_err().contains("model parameter"));
        assert!(parse_step(".step param R 1k 2k -1").is_err() && parse_step(".step param R 1k").is_err());
        assert_eq!(short_num(1e3), "1k");
        assert_eq!(short_num(2.2e-6), "2.2u");
        assert_eq!(short_num(-40.0), "-40");
        // the variants: the first .step fastest; a param added where the deck has none
        let p = Prepared("* t\nR1 a 0 {R}\nV1 a 0 DC 5\n.op\n.end\n".into(), vec![], vec![".step param R list 1k 2k".into(), ".step V1 list 1 2".into()]);
        let st = p.steps().unwrap().unwrap();
        let s: Vec<(String, Prepared)> = (0..st.total).map(|k| st.at(k).unwrap()).collect();
        assert_eq!(s.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["R=1k V1=1", "R=2k V1=1", "R=1k V1=2", "R=2k V1=2"]);
        assert!(s[1].1.text().starts_with("* t\n.param R=2e3\n") && s[3].1.text().contains("V1 a 0 DC 2e0"), "{}", s[3].1.text());
        // a source without a DC value gets one; the deck's own .temp is the one stepped
        let p = Prepared("* t\nV1 a 0 AC 1\n.temp 50\n.end\n".into(), vec![], vec![".step V1 list 3".into(), ".step temp list -40".into()]);
        let t = p.steps().unwrap().unwrap().at(0).unwrap().1;
        assert!(t.text().contains("V1 a 0 DC 3e0 AC 1") && t.text().contains(".temp -4e1") && !t.text().contains("options"), "{}", t.text());
        let dup = Prepared("* t\n.end\n".into(), vec![], vec![".step param R list 1k 1k".into()]);
        let st = dup.steps().unwrap().unwrap();
        assert_eq!((st.at(0).unwrap().0, st.at(1).unwrap().0), ("R=1k #1".to_string(), "R=1k #2".to_string()), "labels told apart");
        let p = Prepared("* t\nV1 a 0 AC 1 DC 2\n.end\n".into(), vec![], vec![".step V1 list 3".into()]);
        assert!(p.steps().unwrap().unwrap().at(0).unwrap().1.text().contains("V1 a 0 AC 1 DC 3e0"), "the DC value where it is");
        assert!(parse_step(".step VDMOS IRF530(Rd) 1m 10m 1m").unwrap_err().contains("model parameter"));
        assert!(parse_step(".step param x 0 1 1e-300").unwrap_err().contains("more than"), "no overflow");
        assert_eq!(short_num(1000.5), "1.0005k");
    }

    #[test]
    fn stepped_run() {
        let _s = SIM.lock();
        let src = EXAMPLES.iter().find(|e| e.0 == "rc_transient").unwrap().1.replace("SYMATTR Value 100n", "SYMATTR Value {Cx}")
            .replace("!.tran 3m", "!.tran 3m\\n.step param Cx list 100n 1u\\n.meas tran vmax MAX v(out)");
        let nl = netlist_for_sim(&parse_asc(&src), &mut Library::builtin());
        assert!(nl.warnings.is_empty() && nl.steps.len() == 1, "{:?} {:?}", nl.warnings, nl.steps);
        let run = crate::ngspice::run(&nl.prepared(None, &mut vec![])).unwrap();
        assert_eq!(run.steps, ["Cx=100n", "Cx=1u"]);
        let out = crate::ngspice::trace("V(out)", &run.scale, run.id).unwrap();
        let n = out.x.len() / 2;
        assert!(n > 10 && out.x[..n] == out.x[n..] && out.re.len() == 2 * n, "a family of two on one time grid");
        assert_eq!(out.steps, Some(vec![0, 1]), "which steps its curves are");
        // a table (.op): one value per step, no x
        let op = netlist_for_sim(&parse_asc(&src.replace("!.tran 3m", "!.op")), &mut Library::builtin());
        let r = crate::ngspice::run(&op.prepared(None, &mut vec![])).unwrap();
        let v = crate::ngspice::trace("V(out)", &r.scale, r.id).unwrap();
        assert!(v.x.is_empty() && v.re.len() == 2 && v.steps == Some(vec![0, 1]), "{v:?}");
        let m = |k: &str| run.meas.iter().find(|x| x.0 == format!("vmax [{k}]")).and_then(|x| x.1).unwrap_or_else(|| panic!("{:?}", run.meas));
        assert!(m("Cx=1u") < 0.5 * m("Cx=100n"), "the larger C filters more: {:?}", run.meas);
        // .noise: each step's spectrum to plot, each step's totals in the table (sqrt(kT/C))
        let src = EXAMPLES.iter().find(|e| e.0 == "noise_rc").unwrap().1.replace("SYMATTR Value 1n", "SYMATTR Value {C}")
            .replace("!.noise V(out) V1 dec 20 10 10Meg", "!.noise V(out) V1 dec 20 10 10Meg\\n.step param C list 1n 4n");
        let nl = netlist_for_sim(&parse_asc(&src), &mut Library::builtin());
        let run = crate::ngspice::run(&nl.prepared(None, &mut vec![])).unwrap();
        assert!(run.plot.starts_with("noise") && run.steps.len() == 2, "{} {:?}", run.plot, run.steps);
        let sp = crate::ngspice::trace("onoise_spectrum", &run.scale, run.id).unwrap();
        assert!(sp.x.len() > 10 && sp.x.len() % 2 == 0, "a family of two spectra");
        let tot = |k: &str| run.op.iter().find(|o| o.0 == format!("onoise_total [{k}]")).map(|o| o.1).unwrap_or_else(|| panic!("{:?}", run.op));
        assert!((tot("C=1n") / tot("C=4n") - 2.0).abs() < 0.1, "sqrt(kT/C): 4x C, half the noise");
    }

    fn lt_random_t(t: &str) -> std::borrow::Cow<'_, str> {
        lt_random(t, &mut vec![])
    }

    #[test]
    fn netlist_import() {
        // a netlist opened: its title a comment, the rest one directive, simulated as it was
        let net = "* RC divider\nV1 in 0 DC 2\nR1 in out 1k\nR2 out 0 1k\n.op\n.end\nnot read\n";
        let items = netlist_items(net);
        assert!(matches!(&items[0], Item::Text { text, directive: false, .. } if text == "Netlist: RC divider"));
        assert!(matches!(&items[1], Item::Text { text, directive: true, .. } if text.starts_with("V1 in 0") && text.ends_with(".op")), "{items:?}");
        let _s = SIM.lock();
        let nl = netlist_for_sim(&items, &mut Library::builtin());
        assert!(nl.warnings.is_empty(), "grounded by its own lines: {:?}", nl.warnings);
        let run = crate::ngspice::run(&nl.prepared(None, &mut vec![])).unwrap();
        let v = crate::ngspice::trace("v(out)", &run.scale, run.id).unwrap();
        assert!((v.re[0] - 1.0).abs() < 1e-9, "{:?}", v.re);
        assert!(is_netlist("/a/b.CIR") && is_netlist("x.net") && !is_netlist("x.asc"));
        // ngspice's .control analyses as dot commands; a library: no title taken from it
        let items = netlist_items("* t\nV1 1 0 1\nR1 1 0 1k\n.control\n* show it\ntran 1n 1u\nop\nrun\nalter R1 2k\ntran 1n 1u\nfourier 1meg v(1)\nplot v(1)\n.endc\n.op\n.end ; deck\nmore\n");
        assert!(matches!(&items[1], Item::Text { text, .. } if text.ends_with("R1 1 0 1k\n.op\n.tran 1n 1u\n.four 1meg v(1)") && !text.contains("plot") && !text.contains("more")), "{items:?}");
        assert!(matches!(&items[0], Item::Text { text, .. } if text.contains(".tran 1n 1u .four 1meg v(1) taken") && text.contains("3 other commands")), "{items:?}");
        let lib = netlist_items(".subckt OPA 1 2 3\nR1 1 2 1k\n.ends\n");
        assert!(lib.len() == 1 && matches!(&lib[0], Item::Text { text, .. } if text.starts_with(".subckt OPA")));
    }

    #[test]
    fn raw_export() {
        // the results as a .raw file: the nodes, part currents by name, each step a plot
        let _s = SIM.lock();
        // (a net named out+: written as a name, not read as a sum)
        let src = EXAMPLES.iter().find(|e| e.0 == "step_rc").unwrap().1.replace("FLAG 224 64 out", "FLAG 224 64 out+").replace("vdb(out)", "vdb(out+)");
        let nl = netlist_for_sim(&parse_asc(&src), &mut Library::builtin());
        let run = crate::ngspice::run(&nl.prepared(None, &mut vec![])).unwrap();
        let (bytes, dropped) = crate::ngspice::export_raw(run.id).unwrap();
        assert!(dropped.is_empty(), "{dropped:?}");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.matches("Plotname:").count() == 3 && text.contains("Plotname: AC Analysis (C=47n)") && text.contains("Command: ngspice"), "one plot per step, named by it");
        assert!(text.contains("\tv(out+)\t") && text.contains("\tI(R1)\tcurrent") && !text.contains("vsense"), "{}", &text[..text.find("Binary:").unwrap_or(400).min(text.len())]);
        // the columns kept are the right ones: the first step's last v(out+), as traced (complex: .ac)
        let head_end = bytes.windows(8).position(|w| w == b"Binary:\n").unwrap() + 8;
        let head = std::str::from_utf8(&bytes[..head_end]).unwrap();
        let n = |k: &str| head.lines().find_map(|l| l.strip_prefix(k)).unwrap().trim().parse::<usize>().unwrap();
        let (vars, points) = (n("No. Variables:"), n("No. Points:"));
        let col = head.lines().skip_while(|l| !l.starts_with("Variables:")).skip(1).position(|l| l.split('\t').nth(2) == Some("v(out+)")).unwrap();
        let at = head_end + ((points - 1) * vars + col) * 16;
        let f = |o: usize| f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        let t = crate::ngspice::trace("v(\"out+\")", &run.scale, run.id).unwrap(); // (quoted, as probes write it)
        let k = t.x.len() / 3 - 1; // (the first of three steps' curves, its last point)
        assert!((f(at) - t.re[k]).abs() < 1e-12 && (f(at + 8) - t.im.as_ref().unwrap()[k]).abs() < 1e-12, "{} {}", f(at), t.re[k]);
        // the results themselves unchanged: a part current still traced as before
        assert!(crate::ngspice::trace("I(R1)", &run.scale, run.id).is_ok());
    }

    #[test]
    fn drawings_roundtrip() {
        // the schematic's own drawings: read as items, written back as they were (an unreadable one: raw)
        let src = "Version 4\nSHEET 1 880 680\nLINE Normal 0 16 64 16 2\nRECTANGLE Wide -16 0 64 80\nARC Normal 0 0 32 32 0 16 32 16\nLINE Normal 0 0\nCIRCLE Normal 0 0 8 8\nLINE 0 16 64 16 2\n";
        let items = parse_asc(src);
        assert!(matches!(&items[1], Item::Draw { kind, width, p, style: Some(2) } if kind == "LINE" && width == "Normal" && p == &vec![0, 16, 64, 16]), "{items:?}");
        assert!(matches!(&items[3], Item::Draw { kind, p, style: None, .. } if kind == "ARC" && p.len() == 8));
        assert!(matches!(&items[4], Item::Raw { .. }) && matches!(&items[6], Item::Raw { .. }), "no pen: kept as written");
        assert_eq!(write_asc(&items), src);
    }

    #[test]
    fn monte_carlo() {
        assert_eq!(lt_random_t("R1 a b {mc(1k, tol)}\n.param x=flat(1)+Gauss(2*(1+1)) y=gauss(1,0.1,3) amc(1) mc_x"), "R1 a b {unif(1k, tol)}\n.param x=aunif(0,1)+agauss(0,2*(1+1),1) y=gauss(1,0.1,3) amc(1) mc_x");
        assert_eq!(lt_random_t("R1 a b {mc(mc(1k,0.1),0.1)}"), "R1 a b {unif(unif(1k,0.1),0.1)}", "nested");
        // not in comments (an unclosed one before does not stop the rest), a B source, a file name, a .func's own
        assert_eq!(lt_random_t("* FLAT (zero TC\nR1 a b {mc(1k,0.1)} ; gauss (see\nB1 n 0 V=gauss(1)\nV1 a 0 PWL file=\"gauss (1).csv\"\n"), "* FLAT (zero TC\nR1 a b {unif(1k,0.1)} ; gauss (see\nB1 n 0 V=gauss(1)\nV1 a 0 PWL file=\"gauss (1).csv\"\n");
        assert_eq!(lt_random_t(".func flat(x) {x*2}\nR1 a b {flat(1)+mc(1,1)}\n"), ".func flat(x) {x*2}\nR1 a b {flat(1)+unif(1,1)}\n");
        assert!(matches!(lt_random_t("XU1 a b MC33078\n"), std::borrow::Cow::Borrowed(_)), "nothing to do: no copy");
        let mut w = vec![];
        assert_eq!(lt_random("R1 a b {mc(1k)} 'flat(1)'\n.func f(x) gauss(x)\n", &mut w), "R1 a b {mc(1k)} 'aunif(0,1)'\n.func f(x) agauss(0,x,1)\n");
        assert!(w.len() == 1 && w[0].contains("two arguments"), "{w:?}");
        assert_eq!(lt_random("R1 a b {flat()}\n", &mut w), "R1 a b {flat()}\n");
        assert!(w.len() == 2 && w[1].contains("one argument"), "{w:?}");
        assert_eq!(lt_random_t(".param a=1\n+ b=flat(1) f=\"gauss(1).csv\"\nR1 a b } {mc (1k,0.1)}\n.function flat(x) x\nR2 a b {flat(2)}\n"), ".param a=1\n+ b=flat(1) f=\"gauss(1).csv\"\nR1 a b } {unif(1k,0.1)}\n.function flat(x) x\nR2 a b {flat(2)}\n", "the deck's own .function flat");
        assert_eq!(lt_random_t("* Größe\nR1 a b {mc\t(1k,0.1)}\nV1 a 0 PWL file=\"r{a.csv\" flat(1)\n"), "* Größe\nR1 a b {unif(1k,0.1)}\nV1 a 0 PWL file=\"r{a.csv\" flat(1)\n", "no panic at a ö; a tab; a quoted {{");
        // each step of `.step param run …` another sample, within the tolerance
        let _s = SIM.lock();
        let deck = prepare("* mc\nV1 a 0 1\nR1 a b {mc(1k,0.5)}\nR2 b 0 1k\n.op\n.end\n", None, &mut vec![]);
        let run = crate::ngspice::run(&Prepared(deck.0.clone(), vec![], vec![".step param run 1 6 1".into()])).unwrap();
        let v = crate::ngspice::trace("V(b)", &run.scale, run.id).unwrap();
        let mut vs = v.re.clone();
        vs.sort_by(f64::total_cmp);
        vs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        assert!(v.re.len() == 6 && vs.len() > 3 && v.re.iter().all(|&x| (0.4 - 1e-9..=2.0 / 3.0 + 1e-9).contains(&x)), "{:?}", v.re);
        // a sweep's steps (root locus): one sample for all, by a seed
        let pz = Prepared(deck.0.replace(".op", ".pz a 0 b 0 vol pz"), vec![], vec![]).only_pz();
        assert!(pz.0.contains(".options seed="), "{}", pz.0);
        let at = |_: ()| crate::ngspice::trace("V(b)", &"".to_string(), crate::ngspice::run(&Prepared(pz.0.replace(".pz a 0 b 0 vol pz", ".op"), vec![], vec![])).unwrap().id).unwrap().re[0];
        assert_eq!(at(()), at(()), "the same draw each time");
    }

    #[test]
    fn pin_currents_by_name() {
        let _s = SIM.lock();
        // a transistor: a sensor at every pin, named as usual; the currents add up to zero
        let src = EXAMPLES.iter().find(|e| e.0 == "bjt_curves").unwrap().1.replace(".dc V1 0 10 50m I1 0 50u 10u", ".op");
        let nl = netlist_for_sim(&parse_asc(&src), &mut Library::builtin());
        let q = nl.insts.iter().position(|n| n == "Q1").unwrap();
        assert_eq!(nl.pin_currents[q], ["Ic(Q1)", "Ib(Q1)", "Ie(Q1)"], "{}", nl.text);
        // a subcircuit: a current per pin (open ones held by 1 TΩ, so nothing floats), no I(U1)
        let follower = "WIRE -32 48 -32 0\nWIRE -32 0 48 0\nWIRE 48 0 48 64\nWIRE 48 64 32 64\nWIRE -112 80 -32 80\nFLAG -112 160 0\nFLAG 48 64 out\nSYMBOL Opamps\\opamp 0 0 R0\nSYMATTR InstName U1\nSYMBOL voltage -112 64 R0\nSYMATTR InstName V1\nSYMATTR Value 2\nTEXT 0 200 Left 2 !.op\n";
        let f = netlist_for_sim(&parse_asc(follower), &mut Library::builtin());
        let u = f.insts.iter().position(|n| n == "XU1").unwrap();
        assert_eq!(f.pin_currents[u], ["Ix(U1:In-)", "Ix(U1:In+)", "Ix(U1:Out)"], "{}", f.text);
        assert!(!f.text.contains("Rsense_XU1_2") && f.text.contains("Rsense_XU1 "), "In+ is held by V1, out only by U1 itself: {}", f.text);
        let lone = netlist_for_sim(&parse_asc("SYMBOL Opamps\\opamp 0 0 R0\nSYMATTR InstName U1\nTEXT 0 200 Left 2 !.op\n"), &mut Library::builtin());
        assert_eq!(lone.text.matches(" 0 1T").count(), 3, "open pins: held, so nothing floats: {}", lone.text);
        let fr = crate::ngspice::run(&prepare(&f.text, None, &mut vec![]).with_sensors(f.sensors.clone())).unwrap();
        let out = fr.op.iter().find(|o| o.0 == "Ix(U1:Out)").map(|o| o.1);
        assert!(out.is_some_and(|i| i.abs() < 1e-6), "a follower without load: {:?}", fr.op);
        assert!(crate::ngspice::trace("I(U1)", &fr.scale, fr.id).unwrap_err().contains("per pin"));
        // a part drawn as D1 that calls a subcircuit (netlist XD1): named as drawn
        let z = netlist_for_sim(&parse_asc(EXAMPLES.iter().find(|e| e.0 == "zener_prefix").unwrap().1), &mut Library::builtin());
        let d = z.insts.iter().position(|n| n == "XD1").unwrap();
        assert_eq!(z.pin_currents[d], ["I(D1)", ""], "{}", z.text);
        let t1 = [Sensor { source: "vsense_qt1".into(), node: "y".into(), inst: "qt1".into(), pin: 0, label: "Ic(T1)".into(), name: "t1".into(), hold: String::new(), alias: "ic(qt1)".into() }];
        assert_eq!(sensor_currents("I(T1)+I(QT1)", &t1).0, "i(vsense_qt1)+i(vsense_qt1)", "pin 1 by the drawn name too");
        assert_eq!(sensor_currents("Ic(QT1)", &t1).0, "i(vsense_qt1)", "a name by the netlist's name");
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![]).with_sensors(nl.sensors.clone())).unwrap();
        let get = |n: &str| run.op.iter().find(|o| o.0 == n).map(|o| o.1).unwrap_or_else(|| panic!("{n}: {:?}", run.op));
        let (ic, ib, ie) = (get("Ic(Q1)"), get("Ib(Q1)"), get("Ie(Q1)"));
        assert!(ib > 0.0 && ic > 20.0 * ib && (ic + ib + ie).abs() < 1e-9, "{ic} {ib} {ie}");
        // a 3-pin MOSFET symbol: the bulk ngspice needs belongs to its source pin (Is includes it)
        let src = "FLAG 48 0 d\nFLAG 0 80 g\nFLAG 48 96 0\nSYMBOL nmos 0 0 R0\nSYMATTR InstName M1\nSYMATTR Value nm\nSYMBOL voltage -100 0 R0\nSYMATTR InstName V1\nSYMATTR Value 3\nFLAG -100 16 g\nFLAG -100 96 0\nSYMBOL voltage 200 0 R0\nSYMATTR InstName V2\nSYMATTR Value -1\nFLAG 200 16 d\nFLAG 200 96 0\nTEXT 0 200 Left 2 !.op\nTEXT 0 240 Left 2 !.model nm nmos(vto=1 kp=1e-3)\n";
        let nl = netlist_for_sim(&parse_asc(src), &mut Library::builtin());
        assert!(nl.text.contains("M1 M1_sense M1_sense_2 M1_sense_3 M1_sense_3 nm"), "{}", nl.text);
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![]).with_sensors(nl.sensors.clone())).unwrap();
        let get = |n: &str| run.op.iter().find(|o| o.0 == n).map(|o| o.1).unwrap_or_else(|| panic!("{n}: {:?}", run.op));
        let (id, ig, is) = (get("Id(M1)"), get("Ig(M1)"), get("Is(M1)"));
        assert!(id.abs() > 1e-6 && (id + ig + is).abs() < 1e-12, "Vds < 0, the body diode conducts: {id} {ig} {is}");
        // names in trace expressions, .tran: I(R1)-I(C1) is the current into the node between them
        let nl = netlist_for_sim(&parse_asc(EXAMPLES[0].1), &mut Library::builtin());
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![]).with_sensors(nl.sensors.clone())).unwrap();
        let d = crate::ngspice::trace("I(R1)-I(C1)", &run.scale, run.id).unwrap();
        let r = crate::ngspice::trace("i(vsense_r1)-i(vsense_c1)", &run.scale, run.id).unwrap();
        assert!(d.re.len() > 10 && d.re == r.re, "the names are the sensors' currents");
        assert!(run.vectors.iter().any(|v| v == "I(R1)") && !run.vectors.iter().any(|v| v.contains("vsense")), "{:?}", run.vectors);
    }

    #[test]
    fn current_sensors_work_in_ac() {
        let _s = SIM.lock();
        let nl = netlist_for_sim(&parse_asc(EXAMPLES.iter().find(|e| e.0 == "rc_bode").unwrap().1), &mut Library::builtin());
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).unwrap();
        let ir = crate::ngspice::trace("i(vsense_r1)", &run.scale, run.id).unwrap();
        let ic = crate::ngspice::trace("i(vsense_c1)", &run.scale, run.id).unwrap();
        let (f, im) = (&ir.x, ir.im.as_ref().expect("complex in .ac"));
        // series RC driven by AC 1: |I| = 1/|R + 1/(jwC)|, the same current through R and C
        let k = 20;
        let w = 2.0 * std::f64::consts::PI * f[k];
        let expect = 1.0 / (1e3f64.powi(2) + (1.0 / (w * 100e-9)).powi(2)).sqrt();
        let got = ir.re[k].hypot(im[k]);
        assert!((got - expect).abs() < 1e-3 * expect, "|I(R1)|={got} expected {expect}");
        let ic_abs = ic.re[k].hypot(ic.im.as_ref().unwrap()[k]);
        assert!((ic_abs - expect).abs() < 1e-3 * expect, "|I(C1)|={ic_abs}");
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![]).with_sensors(nl.sensors.clone())).unwrap();
        let shown = netlist(&parse_asc(EXAMPLES[0].1), &mut Library::builtin());
        assert!(!shown.text.contains("Vsense") && shown.probes.iter().any(|p| p == "i(vsense_r1)"), "{:?}", shown.probes);
        assert!(!run.vectors.iter().any(|v| v.ends_with("_sense")), "{:?}", run.vectors);
        // .op: units from ngspice's vector types, sensor currents under their device
        let nl = netlist_for_sim(&parse_asc(&EXAMPLES[0].1.replace(".tran 3m", ".op")), &mut Library::builtin());
        let op = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![]).with_sensors(nl.sensors.clone())).unwrap().op;
        let unit = |n: &str| op.iter().find(|o| o.0 == n).map(|o| o.3.as_str()).unwrap_or("missing");
        assert_eq!((unit("out"), unit("I(R1)"), unit("v1#branch")), ("V", "A", "A"), "{op:?}");
        assert!(!op.iter().any(|o| o.0.contains("vsense") || o.0.ends_with("_sense")), "{op:?}");
    }

    #[test]
    fn spice_numbers() {
        let close = |s: &str, v: f64| (spice_num(s).unwrap() - v).abs() <= v.abs() * 1e-12;
        assert!(close("3m", 3e-3) && close("1Meg", 1e6) && close("10uF", 10e-6));
        assert!(close("2.2k", 2200.0) && close("1e-3", 1e-3) && close("4.7µ", 4.7e-6));
        assert_eq!(spice_num("abc"), None);
    }

    #[test]
    fn asc_roundtrip_and_netlist() {
        let items = parse_asc(RC);
        assert_eq!(parse_asc(&write_asc(&items)), items);
        let nl = netlist(&items, &mut Library::builtin());
        assert!(nl.warnings.is_empty(), "{:?}", nl.warnings);
        assert!(nl.text.contains("V1 N001 0 SIN(0 1 1k)"), "{}", nl.text);
        assert!(nl.text.contains("R1 out N001 1k") || nl.text.contains("R1 N001 out 1k"), "{}", nl.text);
        assert!(nl.text.contains("C1 out 0 100n"), "{}", nl.text);
        assert!(nl.text.contains(".tran 3e-6 3m"), "{}", nl.text);
    }

    #[test]
    fn examples_simulate() {
        let _s = SIM.lock();
        for (name, src) in EXAMPLES {
            let nl = netlist_for_sim(&parse_asc(src), &mut Library::builtin());
            assert!(nl.warnings.is_empty(), "{name}: {:?}", nl.warnings);
            let mut warnings = vec![];
            let deck = nl.prepared(None, &mut warnings); // as the app runs it (.meas rewritten)
            assert!(warnings.is_empty(), "{name}: {warnings:?}");
            let run = crate::ngspice::run(&deck).unwrap_or_else(|log| panic!("{name}\n{}\n{}", nl.text, log.join("\n")));
            let op = |n: &str| run.op.iter().find(|o| o.0 == n).map(|o| o.1.hypot(o.2)).unwrap_or_else(|| panic!("{name}: no {n} in {:?}", run.op));
            let meas = |n: &str| run.meas.iter().find(|m| m.0 == n).and_then(|m| m.1).unwrap_or_else(|| panic!("{name}: {n} in {:?}\n{}", run.meas, deck.text()));
            let near = |v: f64, want: f64, tol: f64| assert!((v / want - 1.0).abs() < tol, "{name}: {v} (want {want}) {:?}", run.meas);
            // the diagram-type examples: what their comments promise
            match *name {
                "bjt_curves" => {
                    // 201 V_CE points x 6 base currents; at 50 µA the collector carries about beta * I_B
                    let ic = crate::ngspice::trace("i(vsense_q1)", &run.scale, run.id).unwrap();
                    assert_eq!(ic.re.len(), 6 * 201, "{name}");
                    let top = ic.re[5 * 201 + 100].abs(); // V_CE = 5 V, I_B = 50 µA
                    assert!((5e-3..15e-3).contains(&top), "{name}: Ic = {top}");
                }
                "noise_rc" => {
                    let total = op("onoise_total"); // sqrt(kT/C) at 27 °C, 1 nF: 2.03 µV
                    assert!((total - 2.03e-6).abs() < 0.1e-6, "{name}: {total}");
                }
                "phasor_rlc" => {
                    // |V_C| = 230 V / |R + j(wL - 1/wC)| / (wC) = 731 V
                    assert!((op("b") - 731.5).abs() < 2.0, "{name}: {}", op("b"));
                }
                "pz_sallen_key" => {
                    // K = 2: Q = 1, w0 = 1/RC = 1e4 -> -5000 ± j8660; Rb = 30k (K = 4): right half-plane
                    let p = run.op.iter().find(|o| o.0.starts_with("pole") && o.2 > 0.0).unwrap_or_else(|| panic!("{:?}", run.op));
                    assert!((p.1 + 5000.0).abs() < 150.0 && (p.2 - 8660.0).abs() < 200.0, "{name}: {:?}", run.op);
                    let k4 = crate::ngspice::run(&deck.at(&vary_spans(deck.text(), "Rb", true).unwrap(), 30e3)).unwrap();
                    assert!(k4.op.iter().any(|o| o.0.starts_with("pole") && o.1 > 0.0), "{name}: {:?}", k4.op);
                    let ui = netlist(&parse_asc(src), &mut Library::builtin()); // what simulate lists from
                    assert_eq!(sweepable(&ui.text, &ui.insts).first().map(String::as_str), Some("Rb"), "{name}: the locus panel's default");
                }
                // the feature examples: what their comments promise
                "meas_rc_timing" => {
                    near(meas("tdelay"), 0.686e-6, 0.03); // 0.69 tau
                    near(meas("trise"), 2.197e-6, 0.03); // 2.2 tau
                    near(meas("ratio"), 3.2, 0.04);
                    near(meas("pr"), 1e-4, 0.1); // C V^2 per 10 us period
                }
                "buck_converter" => {
                    let (vout, eff) = (meas("vout"), meas("eff"));
                    assert!((4.0..5.0).contains(&vout) && (0.8..0.99).contains(&eff) && meas("ripple") < 0.2, "{name}: {:?}", run.meas);
                    near(meas("pout"), vout * vout / 5.0, 0.03);
                }
                "rectifier_rms" => {
                    let (vrms, vavg) = (meas("vrms"), meas("vavg"));
                    assert!((8.0..9.6).contains(&vrms) && (vrms / vavg - 1.0).abs() < 0.01, "{name}: {:?}", run.meas);
                    assert!(meas("irms") > 3.0 * meas("iavg") && meas("pd") > 0.0, "{name}: {:?}", run.meas);
                    near(meas("pr"), vrms * vrms / 1e3, 0.02);
                }
                "analysis_tabs" => {
                    near(meas("peak"), 3.16, 0.05); // Q at f0 (the active .ac)
                    // the alternatives' tabs: each run as the active analysis (the .meas ac stays in)
                    for alt in [".tran", ".noise"] {
                        let src = src.replace("!.ac dec", ";.ac dec").replace(&format!("Left 2 ;{alt}"), &format!("Left 2 !{alt}"));
                        let nl = netlist_for_sim(&parse_asc(&src), &mut Library::builtin());
                        let mut warnings = vec![];
                        let r = crate::ngspice::run(&nl.prepared(None, &mut warnings)).unwrap_or_else(|log| panic!("{name} {alt}: {}", log.join("\n")));
                        assert!(warnings.is_empty() && nl.warnings.is_empty(), "{name} {alt}: {warnings:?} {:?}", nl.warnings);
                        assert!(r.plot.starts_with(&alt[1..]) && !r.aborted && !r.scale.is_empty(), "{name} {alt}: {} {}", r.plot, r.scale);
                    }
                }
                "laplace_lowpass" => {
                    near(meas("fc"), 159.15, 0.02); // 1/(2*pi*tau)
                    // its transient: the step response 1 - exp(-t/tau), 63 % one tau after the step at 1 ms
                    let tr = src.replace("!.ac dec 20 1 10k", ";.ac dec 20 1 10k").replace(";.tran 6m", "!.tran 6m").replace("!.meas ac fc", ";.meas ac fc");
                    let nl = netlist_for_sim(&parse_asc(&tr), &mut Library::builtin());
                    let r = crate::ngspice::run(&nl.prepared(None, &mut vec![])).unwrap();
                    let out = crate::ngspice::trace("V(out)", &r.scale, r.id).unwrap();
                    let at = |t: f64| out.re[out.x.iter().position(|&x| x >= t).unwrap()];
                    assert!((at(2e-3) - 0.632).abs() < 0.02 && at(0.9e-3).abs() < 0.01, "{name}: {} {}", at(2e-3), at(0.9e-3));
                }
                "rc_bode" => {
                    // the group delay, -dφ/dω: RC at low frequencies, about half of it at fc (the first point past it)
                    let gd = crate::ngspice::trace("group_delay(V(out))", &run.scale, run.id).unwrap();
                    let at = |f: f64| gd.re[gd.x.iter().position(|&x| x >= f).unwrap()];
                    assert!((at(10.0) - 1e-4).abs() < 1e-6 && (at(1591.0) - 0.5e-4).abs() < 4e-6, "{name}: {} {}", at(10.0), at(1591.0));
                }
                "monte_carlo" => {
                    // 20 samples: fc = 1/(2*pi*R*C) within the tolerances (1.25 … 2.09 kHz), and spread
                    assert_eq!(run.steps.len(), 20, "{name}");
                    let fc: Vec<f64> = run.meas.iter().filter(|m| m.0.starts_with("fc [")).filter_map(|m| m.1).collect();
                    let (lo, hi) = fc.iter().fold((f64::MAX, 0f64), |(a, b), &v| (a.min(v), b.max(v)));
                    assert!(fc.len() == 20 && lo > 1.24e3 && hi < 2.1e3 && hi - lo > 50.0, "{name}: {fc:?}");
                }
                "step_rc" => {
                    assert_eq!(run.steps, ["C=47n", "C=100n", "C=220n"], "{name}");
                    near(meas("fc [C=47n]"), 3386.0, 0.02);
                    near(meas("fc [C=100n]"), 1591.5, 0.02);
                    near(meas("fc [C=220n]"), 723.4, 0.02);
                }
                "zener_prefix" => {
                    let out = crate::ngspice::trace("v(out)", &run.scale, run.id).unwrap();
                    let at = |v: f64| out.re[out.x.iter().position(|&x| x >= v - 1e-6).unwrap()]; // (the sweep's V1)
                    near(at(12.0), 5.1, 0.08); // V1 = 12 V: clamped near 5 V
                    near(at(3.0), 3.0 * 10.0 / 11.0, 0.03); // V1 = 3 V: below breakdown, the divider R1 R2
                }
                _ => {
                    assert!(!run.scale.is_empty(), "{name}: {run:?}");
                    let out = ["v(out)", "i(vsense_d1)"].iter().find_map(|e| crate::ngspice::trace(e, &run.scale, run.id).ok()).unwrap();
                    assert!(out.re.len() > 10, "{name}");
                }
            }
        }
    }

    #[test]
    fn hostile_files_cannot_run_commands() {
        let items = parse_asc("FLAG 0 0 x);shell touch /tmp/pwned;echo (\nWIRE 0 0 32 0\nFLAG 32 0 0\nTEXT 0 64 Left 2 !.control\\nshell touch /tmp/pwned\\n.endc\\n.op\n");
        let mut nl = netlist(&items, &mut Library::builtin());
        let text = prepare(&nl.text, None, &mut nl.warnings);
        assert!(!text.text().contains("shell") && !text.text().contains(".control"), "{}", text.text());
        assert!(text.text().contains(".op"));
        let text = prepare("* t\n*# shell touch /tmp/nomospice-pwned\n.op\n.end\n", None, &mut vec![]);
        assert!(!text.text().contains("shell"), "{}", text.text());
        // (ngspice makes a line's first # a *: `##shell` would be `*#shell`; an Ω at a line's start
        // must not hide a command: the checks read the line as it goes out, `_.control` is none)
        let text = prepare("* t\n##shell touch /tmp/nomospice-pwned\n  ##shell touch /tmp/nomospice-pwned\nΩ*# shell touch /tmp/nomospice-pwned\nΩ.control\nshell touch /tmp/nomospice-pwned\nΩ.endc\n# note\n.op\n.end\n", None, &mut vec![]);
        let out = text.text();
        assert!(out.lines().map(|l| l.trim_start().to_ascii_lowercase()).all(|l| !l.starts_with(".control") && !l.starts_with("*#") && !l.starts_with('#')) && out.contains("* note\n"), "{out}");
        assert_eq!(ascii("Ω.include x.lib\u{a0}R1 a b 10kΩ 4.7µ Größe 10\u{2126}"), ".include x.lib R1 a b 10k 4.7u Gr__e 10");
        // (a deck ngspice would run as a script: `*ng_script` as its title, which .title sets; an
        // .include as the title it would read itself)
        let out = prepare(".include /tmp/nomospice-x.cir\n.TITLE *ng_script\n*ng_script\nV1 a 0 1\n.op\n.end\n", None, &mut vec![]).0;
        assert!(out.starts_with("* .include /tmp/nomospice-x.cir\nV1 a 0 1\n") && !out.lines().any(command_line), "{out}");
        assert!(prepare("*ng_script\nV1 a 0 1\n.end\n", None, &mut vec![]).0.starts_with("* *ng_script\n"));
        let mut w = vec![];
        assert!(!prepare("* t\n.title Amplifier ng_script demo\nV1 a 0 1\n.end\n", None, &mut w).0.contains(".title") && w.is_empty(), "{w:?}"); // (a plain .title: left out, unsaid)
        let mut w = vec![];
        assert!(prepare(".control\nshell touch /tmp/nomospice-pwned\n.endc\n.end\n", None, &mut w).0.starts_with("* .control\n") && w.iter().any(|w| w.contains("title")), "{w:?}"); // (said)
        let mut w = vec![];
        prepare(".title Amplifier\nV1 a 0 1\n.end\n", None, &mut w);
        prepare(".include models.lib\nV1 a 0 1\n.end\n", None, &mut w);
        assert!(w.len() == 1 && w[0].contains(".include"), "{w:?}"); // (a plain .title unsaid; an .include as the title said)
        assert!(include_args(".lib x.lib // corner").1.is_none() && include_args(".include //nas/share/x.lib").0 == "//nas/share/x.lib");
        assert_eq!(include_args(".lib x.lib TT;typical"), ("x.lib".into(), Some("TT")));
        assert_eq!(include_args(".include models.lib;vendor"), ("models.lib".into(), None));
        assert_eq!(include_args(".lib \"/Volumes/Models $ Lib;v2/x.lib\" TT\t$ x"), ("/Volumes/Models $ Lib;v2/x.lib".into(), Some("TT")));
        assert!(is_end(".end;done") && is_end(".end\t* x") && !is_end(".ends"));
        assert_eq!(before_end("* t\nR1 a 0 1\n.END\t; done\n", ".op"), "* t\nR1 a 0 1\n.op\n.END\t; done\n");
        let _s = SIM.lock();
        for deck in ["* t\n.title *ng_script\nshell echo x\n.end\n", "* t\n.include /tmp/nomospice-x.cir\n.end\n", "*ng_script\nshell echo x\n.end\n"] {
            assert!(crate::ngspice::run(&Prepared(deck.into(), vec![], vec![])).is_err_and(|e| e[0].contains("security")), "{deck}"); // (the last gate)
        }
        assert!(nl.nets[0][0].chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{:?}", nl.nets[0]);
        assert!(crate::ngspice::trace("v(a);shell ls", "", 0).is_err());
    }

    #[test]
    fn includes_are_inlined_and_sanitized() {
        let dir = std::env::temp_dir().join(format!("nomospice-inc-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join("with space")).unwrap();
        std::fs::write(dir.join("sub/models.lib"), ".control\nshell touch pwned\n.endc\n.model DX D(Is=1n)\n.lib fast ; the fast corner\n.model DF D\n.endl\n").unwrap();
        std::fs::write(dir.join("with space/q.lib"), ".model DQ D\n.END\t* of the lib\n").unwrap();
        let mut w = vec![];
        std::fs::write(dir.join("sub/cycle.lib"), ".include cycle.lib\n.model DC D\n").unwrap();
        let out = prepare("* t\n.include sub/models.lib\n.lib sub/models.lib fast ; vendor\n.lib \"with space/q.lib\"\n.inc missing.lib\n.lib sub/models.lib nope\n.inc sub\\cycle.lib\n.include sub/models.lib\n.end\n", Some(&dir), &mut w);
        let out = out.text();
        assert_eq!(out.matches(".model DX").count(), 1, "{out}");
        assert_eq!(out.matches(".model DC").count(), 1, "{out}");
        assert!(!out.contains("shell") && out.contains(".model DX D(Is=1n)") && out.contains(".model DF D") && out.contains(".model DQ D") && !out.contains(".END"), "{out}");
        assert!(!w.iter().any(|w| w.contains("corner") || w.contains("vendor")), "{w:?}"); // (no comment taken for a file or a section)
        assert!(w.iter().any(|w| w.contains("missing.lib")) && w.iter().any(|w| w.contains("`nope`")), "{w:?}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn digit_and_reserved_net_names_probe() {
        let _s = SIM.lock();
        let src = "FLAG 0 0 3V3\nFLAG 0 80 0\nFLAG 100 0 time\nFLAG 100 80 0\nSYMBOL voltage 0 -16 R0\nSYMATTR Value 3.3\nSYMBOL voltage 100 -16 R0\nSYMATTR InstName V2\nSYMATTR Value 2\nTEXT 0 200 Left 2 !.tran 1m\n";
        let nl = netlist(&parse_asc(src), &mut Library::builtin());
        assert_eq!(nl.nets[2][0], "time_");
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).unwrap();
        let v = crate::ngspice::trace("v(\"3V3\")", &run.scale, run.id).unwrap();
        assert!((v.re[0] - 3.3).abs() < 1e-9, "{:?}", &v.re[..1]);
        assert!(crate::ngspice::trace("v(a) + `touch /tmp/nomospice-x`", "", 0).is_err());
    }

    #[test]
    fn auto_names_never_collide() {
        let src = "FLAG 0 0 N001\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMBOL res 100 0 R0\nSYMBOL res 200 0 R0\nSYMATTR InstName R2\n";
        let nl = netlist(&parse_asc(src), &mut Library::builtin());
        let nets: Vec<String> = nl.nets.iter().flatten().map(|n| n.to_lowercase()).collect();
        assert_eq!(nets.iter().filter(|n| *n == "n001").count(), 1, "{nets:?}");
        assert_eq!(nl.insts[2], "R3", "{:?}", nl.insts);
    }

    #[test]
    fn sensor_names_never_clash_with_user_names() {
        let src = "FLAG 16 16 R1_sense\nSYMBOL res 0 0 R0\nSYMATTR InstName R1\nSYMBOL voltage 200 0 R0\nSYMATTR InstName Vsense_R1\nSYMATTR Value 0\n";
        let nl = netlist_for_sim(&parse_asc(src), &mut Library::builtin());
        assert_eq!(nl.text.matches("Vsense_R1 ").count(), 1, "{}", nl.text); // only the user's
        assert!(nl.text.contains("Vsense_R1_ R1_sense R1_sense_ 0"), "{}", nl.text);
        assert_eq!(nl.probes[1], "i(vsense_r1_)");
    }

    #[test]
    fn t_junction_connects_crossing_does_not() {
        let w = |x1, y1, x2, y2| Item::Wire { x1, y1, x2, y2 };
        let items = vec![w(0, 0, 100, 0), w(50, 0, 50, 50), w(80, -20, 80, 20)];
        let nl = netlist(&items, &mut Library::builtin());
        assert_eq!(nl.nets[0][0], nl.nets[1][0]);
        assert_ne!(nl.nets[0][0], nl.nets[2][0]);
    }

    #[test]
    fn simulate_rc() {
        let _s = SIM.lock();
        let nl = netlist_for_sim(&parse_asc(RC), &mut Library::builtin());
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).expect("sim");
        assert_eq!(run.scale, "time");
        let t = crate::ngspice::trace("v(out)", &run.scale, run.id).unwrap();
        assert_eq!(t.x.len(), t.re.len());
        assert!(t.re.iter().cloned().fold(0.0, f64::max) > 0.5);
        let i = crate::ngspice::trace("i(vsense_r1)", &run.scale, run.id).unwrap();
        assert_eq!(i.re.len(), t.re.len());
        assert!(crate::ngspice::trace("v(nonexistent)", &run.scale, run.id).is_err(), "stale nmsp_trace returned");
    }

    #[test]
    fn builtin_models_and_opamp_follower() {
        let _s = SIM.lock();
        let mut items = parse_asc(RC);
        items.push(Item::Sym { sym: "npn".into(), x: 600, y: 0, rot: "R0".into(), attrs: BTreeMap::new(), extra: vec![] });
        items.push(Item::Sym { sym: "nmos".into(), x: 800, y: 0, rot: "R0".into(), attrs: BTreeMap::new(), extra: vec![] });
        let nl = netlist(&items, &mut Library::builtin());
        assert!(nl.text.contains("Q1 N002 N003 N004 NPN") && nl.text.contains(".model NPN NPN"), "{}", nl.text); // LTspice default
        let m = nl.text.lines().find(|l| l.starts_with("M1 ")).unwrap();
        assert_eq!(m.split_whitespace().count(), 6, "{m}");
        if crate::library::definitions("bsp89", &mut HashSet::new()).is_some() {
            // a library VDMOS keeps its 3 terminals
            let mut items = parse_asc(RC);
            items.push(Item::Sym { sym: "nmos".into(), x: 800, y: 0, rot: "R0".into(), attrs: BTreeMap::from([("Value".into(), "BSP89".into())]), extra: vec![] });
            let nl = netlist(&items, &mut Library::builtin());
            let m = nl.text.lines().find(|l| l.starts_with("M1 ")).unwrap();
            assert_eq!(m.split_whitespace().count(), 5, "{m}");
        }

        let follower = "WIRE -32 48 -32 0\nWIRE -32 0 48 0\nWIRE 48 0 48 64\nWIRE 48 64 32 64\nWIRE -112 80 -32 80\nFLAG -112 160 0\nFLAG 48 64 out\nSYMBOL Opamps\\opamp 0 0 R0\nSYMATTR InstName U1\nSYMBOL voltage -112 64 R0\nSYMATTR InstName V1\nSYMATTR Value 2\nTEXT 0 200 Left 2 !.op\n";
        let nl = netlist(&parse_asc(follower), &mut Library::builtin());
        assert!(nl.text.contains("XU1 N001 N002 out opamp Aol=100K GBW=10Meg") || nl.text.contains("XU1 out N001 out"), "{}", nl.text);
        let run = crate::ngspice::run(&prepare(&nl.text, None, &mut vec![])).unwrap_or_else(|log| panic!("{}\n{}", nl.text, log.join("\n")));
        let out = run.op.iter().find(|(n, ..)| n == "out").unwrap().1;
        assert!((out - 2.0).abs() < 1e-3, "{out}");
    }
}

/// `NOMOSPICE_CORPUS=<dir with .asc> cargo test corpus -- --ignored --nocapture`
#[test]
#[ignore]
fn corpus() {
    let dir = std::env::var("NOMOSPICE_CORPUS").expect("NOMOSPICE_CORPUS");
    let mut lib = Library::builtin();
    let (mut files, mut unknown) = (0, std::collections::BTreeMap::<String, usize>::new());
    for e in std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "asc")) {
        let src = decode(&std::fs::read(e.path()).unwrap());
        let items = parse_asc(&src);
        let out = write_asc(&items);
        assert_eq!(parse_asc(&out), items, "{:?}", e.path());
        let kept: std::collections::HashSet<&str> = out.lines().collect();
        for l in src.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()) {
            assert!(kept.contains(l), "{:?} lost line {l:?}", e.path());
        }
        for w in netlist(&items, &mut lib).warnings.into_iter().filter(|w| w.starts_with("unknown")) {
            *unknown.entry(w).or_default() += 1;
        }
        files += 1;
    }
    println!("{files} files, unknown symbols: {unknown:?}");
}
