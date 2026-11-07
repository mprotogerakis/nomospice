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
    /// Any .asc line we do not interpret (SHEET, LINE/RECTANGLE/CIRCLE/ARC drawings, BUSTAP, ...).
    Raw { line: String },
}

// ---------------------------------------------------------------- symbols

#[derive(Serialize, Clone, Debug, Default)]
pub struct Pin {
    pub x: i32,
    pub y: i32,
    pub name: String,
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
    /// draw pin names (generated subckt boxes)
    pub pin_labels: bool,
}

macro_rules! builtin {
    ($($n:literal),*) => { &[$(($n, include_str!(concat!("../symbols/", $n, ".asy")))),*] };
}
const BUILTIN_SYMBOLS: &[(&str, &str)] = builtin!(
    "res", "cap", "polcap", "ind", "voltage", "current", "diode", "schottky", "zener", "led", "npn", "pnp",
    "nmos", "pmos", "njf", "pjf", "opamp", "e", "g", "jumper", "sw", "csw"
);

pub fn parse_asy(name: &str, src: &str) -> SymbolDef {
    let mut d = SymbolDef { name: name.into(), ..Default::default() };
    let mut pins: Vec<(u32, Pin)> = vec![];
    for line in src.lines() {
        let tok: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| tok.get(i).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let rest = |n: usize| line.splitn(n + 1, ' ').nth(n).unwrap_or("").trim().to_string();
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
            Some("PIN") => pins.push((pins.len() as u32 + 1, Pin { x: num(1), y: num(2), name: String::new() })),
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
}

/// Where symbols came from, so they can go again.
#[derive(Default)]
pub struct Local {
    /// the schematic's folder
    dir: Option<std::path::PathBuf>,
    /// keys of the symbols loaded from it
    own: HashSet<String>,
    /// a fingerprint of its .asy files when they were (an added, removed, edited, renamed or
    /// replaced symbol changes it)
    stamp: Option<(usize, u64)>,
    /// keys of symbols lent by library folders (they go when the libraries change)
    lent: HashSet<String>,
}

/// The schematic folder's own symbol files: (key, path) of each .asy (any case), not those of a
/// built-in symbol (nomospice's win). Library::set_dir and Save As (carry them along) share it.
pub fn folder_symbols(dir: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let mut list: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("asy")))
        .filter_map(|p| Some((sym_key(p.file_stem()?.to_str()?), p.clone())))
        .filter(|(k, _)| !builtin_symbol(k))
        .collect();
    list.sort(); // (Foo.asy and foo.asy on a case-sensitive disk: always the same one wins, the last)
    list
}

/// (count, a sum over names, modification times and sizes) of a folder's symbol files (the list
/// `folder_symbols` gave).
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
        }
    }

    /// The schematic's folder: its symbols (LTspice looks there first) are all loaded, so the
    /// component list shows them; they replace one cached from LTspice or a library folder (it
    /// comes back with another folder), not a built-in symbol. Another folder (or
    /// none) drops them again; `rescan` reloads the same folder (a symbol saved meanwhile).
    /// Whether the folder's symbols changed (were reloaded).
    pub fn set_dir(&mut self, dir: Option<&std::path::Path>, rescan: bool) -> bool {
        let moved = self.local.dir.as_deref() != dir;
        if !moved && !rescan {
            return false; // netlisting: no disk access
        }
        let list = dir.map(folder_symbols).unwrap_or_default(); // one listing for the stamp and the load
        let stamp = dir.and_then(|d| std::fs::metadata(d).ok()).map(|_| asy_stamp(&list)); // (None: no such folder)
        if !moved && stamp == self.local.stamp {
            return false;
        }
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
                    self.defs.insert(key.clone(), parse_asy(&key, &decode(&bytes)));
                    self.misses.remove(&key);
                    self.local.lent.remove(&key); // the folder's own now, no longer lent
                    self.local.own.insert(key);
                }
                // unreadable: left out; a write that finishes changes the stamp, and the next rescan reads it
                Err(_) => {}
            }
        }
        true
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
            Some("Version") | None => {}
            _ => items.push(Item::Raw { line: line.to_string() }),
        }
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
                    extra.iter().for_each(|l| out += &format!("{l}\n"));
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
                (4, Item::Raw { line }) if !line.starts_with("SHEET ") => out += &format!("{line}\n"),
                _ => {}
            }
        }
    };
    (0..5).for_each(&mut by_kind);
    out
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
    /// Sensors of the simulated netlist (names are reserved in both builds, so they match).
    #[serde(skip)]
    pub sensors: Vec<Sensor>,
    /// The `.step` statements (`+` lines joined): ngspice has none, the run steps (see `Prepared::steps`).
    #[serde(skip)]
    pub steps: Vec<String>,
    pub warnings: Vec<String>,
}

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
];

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
    /// a comment or blank line (SPICE skips these, also between a line and its `+` continuations)
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
        let skip = first.is_empty() || first.starts_with('*');
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
            for k in lt {
                warnings.push(match k.to_lowercase().as_str() {
                    "startup" => "`.tran startup` is LTspice-only: ngspice starts from the DC operating point (supplies are not ramped up)".into(),
                    "steady" => "`.tran steady` is LTspice-only: ngspice runs to the full stop time".into(),
                    _ => "`.tran nodiscard` is LTspice-only: ngspice discards data before Tstart".into(),
                });
            }
            let uic = if rest.iter().any(|t| t.eq_ignore_ascii_case("uic")) { " uic" } else { "" };
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
            Some(format!(".tran {}{uic}", v.join(" ")))
        }
        Some(".wave" | ".net" | ".savebias" | ".loadbias") => {
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
        // with their `+` continuation lines
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
                // (the DC value where it is written: right after the nodes, or after a `DC` anywhere)
                let k = sp.iter().skip(3).position(|&(a, e)| line.line[a..e].eq_ignore_ascii_case("dc")).map_or(3, |p| p + 4);
                match sp.get(k).filter(|&&(a, e)| spice_num(&line.line[a..e]).is_some() || line.line[a..e].starts_with('{')) {
                    Some(&(a, e)) => Ok(splice(text, &[(line.start + a, line.start + e)], v)),
                    // `V1 in 0 AC 1`, a SINE: no DC value written (0): one put after the nodes
                    None => {
                        let at = line.start + sp.get(2).ok_or_else(|| format!("`.step {n}`: {n} has no nodes"))?.1;
                        Ok(format!("{} DC {v:e}{}", &text[..at], &text[at..]))
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
    match deck_lines(text).iter().find(|l| l.top && (l.low.trim() == ".end" || l.low.starts_with(".end "))) {
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
/// - `.control` blocks and `*#` lines are dropped everywhere (ngspice executes them on load,
///   including `shell`), in the schematic and in included files alike.
pub fn prepare(text: &str, dir: Option<&std::path::Path>, warnings: &mut Vec<String>) -> Prepared {
    let mut x = Expander { warnings, out: String::new(), seen: HashSet::new(), read: true, diode: None, behavioural: vec![] };
    x.expand(text, dir, None, 0);
    x.warn_behavioural();
    // (also in included files; Laplace first: a random call there is said by its own name)
    let out = crate::laplace::rewrite(&x.out, x.warnings);
    let out = lt_random(&out, x.warnings).into_owned();
    Prepared(out, vec![], vec![])
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
    // (`.func gauss(x) …`: the deck's own)
    let own: Vec<String> = text.lines().filter_map(|l| {
        let t = l.trim_start().as_bytes();
        // (bytes: a comment's ö at byte 5 is no place to cut a str; LTspice also spells it .function)
        let head = [&b".function"[..], b".func"].into_iter().find(|h| t.len() > h.len() && t[..h.len()].eq_ignore_ascii_case(h) && t[h.len()].is_ascii_whitespace())?;
        let rest = l.trim_start()[head.len()..].trim_start();
        Some(rest.split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or("").to_ascii_lowercase())
    }).collect();
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

const RANDOM: [&str; 3] = ["mc", "flat", "gauss"];
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
        let close = open.and_then(|open| {
            let mut depth = 0;
            s[open..].char_indices().find_map(|(i, c)| {
                depth += match c { '(' => 1, ')' => -1, _ => 0 };
                (depth == 0).then_some(open + i)
            })
        });
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
        let args = {
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
    let mut x = Expander { warnings, out: String::new(), seen: HashSet::new(), read: false, diode: None, behavioural: vec![] };
    x.expand(text, dir, None, 0);
    x.warn_behavioural();
    let expanded = crate::laplace::rewrite(&std::mem::take(&mut x.out), x.warnings);
    let expanded = lt_random(&expanded, x.warnings).into_owned(); // (mc(1k): said while editing too)
    ltspice_meas(&expanded, &[], warnings); // a .meas left out is said before a run too (the schematic's own: included files are read only by a run)
}

/// `.lib "path with spaces.lib" section` -> (path, section)
fn include_args(t: &str) -> (String, Option<&str>) {
    let rest = t.split_once(char::is_whitespace).map_or("", |r| r.1).trim();
    let (file, tail) = match rest.chars().next() {
        Some(q @ ('"' | '\'')) => rest[1..].split_once(q).unwrap_or((&rest[1..], "")),
        _ => rest.split_once(char::is_whitespace).unwrap_or((rest, "")),
    };
    (file.replace('\\', "/"), tail.split_whitespace().next()) // Windows-authored paths
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
    /// (file, section) already inlined: repeated includes and include cycles are skipped.
    seen: HashSet<(std::path::PathBuf, Option<String>)>,
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
        // logical lines: trimmed (subckt bodies are often indented), `+` continuations joined
        let mut logical: Vec<String> = vec![];
        for l in self.out.lines().map(str::trim) {
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
        for (n, line) in text.lines().enumerate() {
            let t = line.trim();
            let l = t.to_lowercase();
            let tok: Vec<&str> = t.split_whitespace().collect();
            if let Some(sec) = section {
                if !in_section {
                    in_section = l.starts_with(".lib") && tok.len() == 2 && tok[1].eq_ignore_ascii_case(sec);
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
            if l.starts_with("*#") {
                self.warnings.push(format!("`{t}`: *# command lines are not executed (security)"));
                continue;
            }
            if control || l.starts_with(".endc") || l.starts_with(".endl") || (depth > 0 && (l == ".end" || l.starts_with(".end "))) {
                control &= !l.starts_with(".endc");
                continue;
            }
            if (l.starts_with(".inc") || l.starts_with(".lib")) && !(depth == 0 && n == 0) {
                self.include(t, &l, tok.len(), dir, depth);
                continue;
            }
            // LTspice's behavioural diode (Ron/Roff/Vfwd/Vrev) becomes an XSPICE device in ngspice:
            // without its code models loaded the whole deck fails to parse, so say why up front
            if !l.starts_with('+') {
                let t: Vec<&str> = l.split(|c: char| c.is_whitespace() || c == '(').filter(|s| !s.is_empty()).collect();
                self.diode = (t.first() == Some(&".model") && t.get(2) == Some(&"d")).then(|| t[1].to_string());
            }
            if let Some(m) = &self.diode {
                if ["ron=", "roff=", "vfwd=", "vrev="].iter().any(|p| l.replace(' ', "").contains(p)) {
                    self.behavioural.push(m.clone());
                    self.diode = None; // once per model
                }
            }
            // libngspice rejects the whole deck on non-ASCII bytes (`10µ`, `4.7μF`, `10kΩ`)
            self.out.extend(line.chars().filter(|c| *c != 'Ω').map(|c| match c {
                'µ' | 'μ' => 'u',
                c if c.is_ascii() => c,
                _ => '_',
            }));
            self.out.push('\n');
        }
        if !found {
            self.warnings.push(format!("library section `{}` not found", section.unwrap_or_default()));
        }
    }

    fn include(&mut self, t: &str, l: &str, ntok: usize, dir: Option<&std::path::Path>, depth: usize) {
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
                let key = (path.canonicalize().unwrap_or(path.clone()), sec.map(str::to_lowercase));
                if !self.seen.insert(key) {
                    return; // already inlined (or an include cycle)
                }
                match std::fs::read(&path) {
                    Ok(bytes) => self.expand(&decode(&bytes), path.parent(), sec, depth + 1),
                    Err(e) => self.warnings.push(format!("{}: {e}", path.display())),
                }
            }
            // inside an included file, `.lib name` without a file is a section marker
            None if depth > 0 && ntok == 2 => {}
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
    build_netlist(items, lib, false)
}

/// The netlist that is simulated: every probe-able device gets 0 V current sensors: two-pin parts
/// (R C L D) `Vsense_<inst>` at pin 1, transistors (Q M J) and subcircuits (X) one at every pin
/// (`Vsense_<inst>_2` …), so `I(R1)`, `Ib(Q1)` … are sensor currents in every analysis: with phase
/// in .ac (where ngspice, also with `.options savecurrents`, leaves @r[i]/@c[i]/@q[ie] empty and
/// reports DC currents for D/Q/M), including ESR/parasitics, and following value changes.
pub fn netlist_for_sim(items: &[Item], lib: &mut Library) -> Netlist {
    build_netlist(items, lib, true)
}

/// Prefixes that get current sensors (see `netlist_for_sim`).
/// (L too: its native branch current misses Rpar/Cpar from the LTspice database.)
pub const SENSED: &str = "RCLDQMJX";

fn build_netlist(items: &[Item], lib: &mut Library, sense: bool) -> Netlist {
    let mut nl = Netlist { nets: vec![vec![]; items.len()], insts: vec![String::new(); items.len()], probes: vec![String::new(); items.len()], pin_currents: vec![vec![]; items.len()], ..Default::default() };

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
            Item::Text { .. } | Item::Raw { .. } => {}
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
    if ground.is_none() {
        nl.warnings.push("no ground (0) in schematic".into());
    }

    // 4. directives (`.control` blocks are removed later by `prepare`, the single security filter)
    let mut directives = vec![];
    for it in items {
        if let Item::Text { text, directive: true, .. } = it {
            let mut skipped = false; // `+` continuation lines of a skipped directive go too
            let mut stepping = false; // ... of a .step: they belong to it
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let l = line.to_lowercase();
                // (each line without its inline comment: a `;` must not swallow the next line's values;
                // comment lines between a .step and its + lines skipped, as SPICE does)
                if stepping && (l.starts_with('*') || l.starts_with(';')) {
                    directives.push(format!("* {}", line.trim_start_matches(['*', ';']).trim())); // (kept, as a comment)
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
                if skipped && l.starts_with('+') || l == ".end" || l.starts_with(".end ") {
                    continue;
                }
                let d = directive(line, &mut nl.warnings);
                skipped = d.is_none();
                directives.extend(d);
            }
        }
    }
    // a .step that will not run is said before a run; the netlist shows them (nomospice runs them)
    nl.warnings.extend(nl.steps.iter().filter_map(|s| parse_step(s).err()));
    directives.extend(nl.steps.iter().map(|s| format!("* {s} (nomospice: one run per step)")));
    let defined = |name: &str| directives.iter().any(|d| {
        let t: Vec<String> = d.split_whitespace().take(2).map(str::to_lowercase).collect();
        matches!(t.first().map(String::as_str), Some(".model" | ".subckt")) && t.get(1).map(String::as_str) == Some(name)
    });
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
    let mut lines = vec!["* nomospice netlist".to_string()];
    let mut used_models = vec![];
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
        let value = one_line(&attrs.get("Value").cloned().unwrap_or(def.value.clone())).replace("SINE(", "SIN(");
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
        if "DQMXJSW".contains(prefix) && !defined(&model) && !used_models.contains(&model) {
            used_models.push(model);
        }
        let extra: Vec<String> = ["Value2", "SpiceLine", "SpiceLine2"].iter()
            .filter_map(|k| attrs.get(*k).cloned().or_else(|| (*k == "SpiceLine").then(|| def.spice_line.clone())))
            .filter(|s| !s.is_empty()).map(|s| one_line(&s)).collect();
        // LTspice accepts Rser=... in Value too: value and extras go through one pass
        let (rest, parasitics) = lt_instance_params(prefix, &inst, &format!("{value} {}", extra.join(" ")), &mut nl.warnings);
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
        lines.push(line);
        nl.insts[i] = inst;
    }
    let is = |d: &str, k: &str| spice_tokens(d).first().is_some_and(|t| t.eq_ignore_ascii_case(k));
    // sources written as SPICE directives count too (`V1 in 0 AC 1`: AC after the two nodes)
    any_ac |= directives.iter().any(|d| d.starts_with(['V', 'v', 'I', 'i']) && spice_tokens(d).iter().skip(3).any(|t| t.eq_ignore_ascii_case("ac")));
    if !any_ac && directives.iter().any(|d| is(d, ".ac")) {
        nl.warnings.push(".ac: no source has an AC amplitude, so every result is 0; add e.g. `AC 1` to the input source".into());
    }
    lines.push(".param pi=3.141592653589793".into()); // LTspice built-in; a user .param pi comes later and wins
    let includes = directives.iter().any(|d| { let l = d.to_lowercase(); l.starts_with(".inc") || l.starts_with(".lib") });
    lines.extend(directives);
    // after the user's directives (ngspice keeps the first definition, so .include/.lib models win):
    // the libraries (LTspice's stock models as LTspice loads them implicitly, Micro-Cap 12, user
    // folders; with their dependencies), else ours. `opamp` is always ours: the symbol's model.
    let mut emitted = HashSet::new();
    for m in used_models {
        let builtin = BUILTIN_MODELS.iter().find(|(n, _)| *n == m).map(|(_, t)| t.to_string());
        let lib = if m == "opamp" { None } else { crate::library::definitions(&m, &mut emitted) };
        match (lib, builtin) {
            (Some(defs), _) => lines.extend(defs),
            (None, Some(text)) => lines.push(text),
            (None, None) if !includes && !emitted.contains(&m) => nl.warnings.push(format!("`{m}`: no model or subcircuit of that name in the libraries")),
            _ => {}
        }
    }
    lines.push(".end".into());
    nl.text = lines.join("\n") + "\n";
    nl
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
        assert_eq!(t(".tran 1u 1m startup"), ".tran 1u 1m");
        assert_eq!(t(".tran 0 1m 0 1u uic"), ".tran 1e-6 1m 0 1u uic");
        assert_eq!(t(".tran 0 {T} startup"), ".tran {(T)/1000} {T}");
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
        let pin = |n: &str| Pin { x: 0, y: 0, name: n.into() };
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
        assert!(nl.nets[0][0].chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{:?}", nl.nets[0]);
        assert!(crate::ngspice::trace("v(a);shell ls", "", 0).is_err());
    }

    #[test]
    fn includes_are_inlined_and_sanitized() {
        let dir = std::env::temp_dir().join(format!("nomospice-inc-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join("with space")).unwrap();
        std::fs::write(dir.join("sub/models.lib"), ".control\nshell touch pwned\n.endc\n.model DX D(Is=1n)\n.lib fast\n.model DF D\n.endl\n").unwrap();
        std::fs::write(dir.join("with space/q.lib"), ".model DQ D\n").unwrap();
        let mut w = vec![];
        std::fs::write(dir.join("sub/cycle.lib"), ".include cycle.lib\n.model DC D\n").unwrap();
        let out = prepare("* t\n.include sub/models.lib\n.lib sub/models.lib fast\n.lib \"with space/q.lib\"\n.inc missing.lib\n.lib sub/models.lib nope\n.inc sub\\cycle.lib\n.include sub/models.lib\n.end\n", Some(&dir), &mut w);
        let out = out.text();
        assert_eq!(out.matches(".model DX").count(), 1, "{out}");
        assert_eq!(out.matches(".model DC").count(), 1, "{out}");
        assert!(!out.contains("shell") && out.contains(".model DX D(Is=1n)") && out.contains(".model DF D") && out.contains(".model DQ D"), "{out}");
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
