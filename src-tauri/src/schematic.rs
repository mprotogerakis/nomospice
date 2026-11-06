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

/// Symbol definitions by key, cached misses, and the library generation the `subckt:` entries
/// were generated for.
pub struct Library(pub HashMap<String, SymbolDef>, HashSet<String>, u64);

impl Library {
    pub fn builtin() -> Self {
        Library(BUILTIN_SYMBOLS.iter().map(|(n, s)| (n.to_string(), parse_asy(n, s))).collect(), HashSet::new(), crate::library::generation())
    }

    /// Resolve a symbol; unknown names fall back to a local LTspice install (graphics + pins only).
    /// Misses are cached so netlisting never rescans the disk for the same name.
    pub fn get(&mut self, name: &str) -> Option<&SymbolDef> {
        let key = sym_key(name);
        let g = crate::library::generation();
        if self.2 != g {
            // other libraries now: regenerate subckt boxes, retry what was missing
            self.0.retain(|k, _| !k.starts_with("subckt:"));
            self.1.clear();
            self.2 = g;
        }
        if self.1.contains(&key) {
            return None;
        }
        if let (false, Some(sub)) = (self.0.contains_key(&key), key.strip_prefix("subckt:")) {
            // a library subcircuit placed from the library browser: generated box symbol
            match crate::library::subckt_symbol(sub) {
                Some(def) => self.0.insert(key.clone(), def),
                None => return self.1.insert(key).then_some(None).flatten(),
            };
        }
        if !self.0.contains_key(&key) {
            let file = format!("{key}.asy");
            let def = ltspice_sym_dirs().into_iter().find_map(|dir| {
                let subdirs = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path());
                let path = std::iter::once(dir.join(name.replace('\\', "/") + ".asy"))
                    .chain(std::iter::once(dir.clone()).chain(subdirs).map(|d| d.join(&file)))
                    .find(|p| p.is_file())?;
                Some(parse_asy(&key, &decode(&std::fs::read(path).ok()?)))
            });
            let Some(def) = def else {
                self.1.insert(key);
                return None;
            };
            self.0.insert(key.clone(), def);
        }
        self.0.get(&key)
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
    examples!("rc_transient", "rc_bode", "opamp_inverting", "ce_amplifier", "diode_sweep", "bjt_curves", "noise_rc", "phasor_rlc", "pz_sallen_key");

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
    /// Sensors of the simulated netlist (names are reserved in both builds, so they match).
    #[serde(skip)]
    pub sensors: Vec<Sensor>,
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
    let defs: Vec<(usize, usize)> = lines.iter().filter(|l| l.top && l.param())
        .flat_map(|l| param_defs(l.line, &l.low).into_iter().filter(|(n, _)| n.eq_ignore_ascii_case(t)).map(move |(_, (a, e))| (l.start + a, l.start + e)))
        .collect();
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
    let lines = deck_lines(text);
    let i = lines.iter().position(|l| l.head == ".pz")?;
    let more = lines[i + 1..].iter().take_while(|l| l.skip || l.low.trim_start().starts_with('+'));
    Some(std::iter::once(&lines[i]).chain(more).filter(|l| !l.skip).map(|l| l.low.trim_start().trim_start_matches('+')).collect::<Vec<_>>().join(" "))
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
            let tok = spice_tokens(line.split(';').next().unwrap_or_default()).into_iter().skip(1);
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
        Some(".step" | ".wave" | ".net" | ".savebias" | ".loadbias") => {
            warnings.push(format!("`{line}` is LTspice-only and was skipped"));
            None
        }
        Some(".backanno") => None,
        _ => Some(line.to_string()),
    }
}

/// A netlist that went through [`prepare`]; the only thing `ngspice::run` accepts.
pub struct Prepared(String, Vec<Sensor>);

impl Prepared {
    pub fn text(&self) -> &str {
        &self.0
    }

    /// The current sensors in this deck, so results can hide them (see `Sensor`).
    pub fn with_sensors(mut self, sensors: Vec<Sensor>) -> Self {
        self.1 = sensors;
        self
    }

    /// This deck with its other analyses commented out (a sweep step needs only `.pz`; a `.tran`
    /// or `.meas` would run every step for nothing).
    pub fn only_pz(&self) -> Prepared {
        const OTHER: [&str; 14] = [".tran", ".ac", ".dc", ".op", ".noise", ".tf", ".sens", ".four", ".meas", ".measure", ".disto", ".pss", ".sp", ".hb"];
        // with their `+` continuation lines
        let text = deck_lines(&self.0).iter().map(|l| {
            if !l.skip && OTHER.contains(&l.head.as_str()) { format!("* {}", l.line) } else { l.line.to_string() }
        }).collect::<Vec<_>>().join("\n") + "\n";
        Prepared(text, self.1.clone())
    }
    /// This deck with the value at `spans` set to `value` (one step of a sweep such as a root locus).
    pub fn at(&self, spans: &[(usize, usize)], value: f64) -> Prepared {
        Prepared(splice(&self.0, spans, value), self.1.clone())
    }


    pub fn sensors(&self) -> &[Sensor] {
        &self.1
    }
}

/// A generated 0 V current sensor: source name, its extra node, the device it measures
/// (all lowercase, as ngspice names vectors).
#[derive(Clone, Debug, Default)]
pub struct Sensor {
    pub source: String,
    pub node: String,
    pub inst: String,
}

impl Netlist {
    /// The deck ngspice runs for this (simulation) netlist: includes inlined and filtered, its
    /// current sensors known. The GUI's simulation and root locus and MCP all run this. It reads the
    /// .include/.lib files (can be slow): call it on a blocking thread, without the library lock.
    pub fn prepared(&self, dir: Option<&std::path::Path>, warnings: &mut Vec<String>) -> Prepared {
        prepare(&self.text, dir, warnings).with_sensors(self.sensors.clone())
    }
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
    Prepared(x.out, vec![])
}

/// The warnings `prepare` would give for the schematic's own lines, without reading any
/// included file (cheap enough for every edit).
pub fn prepare_warnings(text: &str, dir: Option<&std::path::Path>, warnings: &mut Vec<String>) {
    let mut x = Expander { warnings, out: String::new(), seen: HashSet::new(), read: false, diode: None, behavioural: vec![] };
    x.expand(text, dir, None, 0);
    x.warn_behavioural();
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

/// The netlist that is simulated: every probe-able device (R C L D Q M J) gets a 0 V current
/// sensor `Vsense_<inst>` at pin 1, so I(device) is `i(vsense_<inst>)` in every analysis: with
/// phase in .ac (where ngspice leaves @r[i]/@c[i] empty and reports DC currents for D/Q/M),
/// including ESR/parasitics, and following value changes.
pub fn netlist_for_sim(items: &[Item], lib: &mut Library) -> Netlist {
    build_netlist(items, lib, true)
}

/// Prefixes that get a current sensor (see `netlist_for_sim`); mirrored by geom.ts `currentProbe`.
/// (L too: its native branch current misses Rpar/Cpar from the LTspice database.)
pub const SENSED: &str = "RCLDQMJ";

fn build_netlist(items: &[Item], lib: &mut Library, sense: bool) -> Netlist {
    let mut nl = Netlist { nets: vec![vec![]; items.len()], insts: vec![String::new(); items.len()], probes: vec![String::new(); items.len()], ..Default::default() };

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
                None => nl.warnings.push(format!("unknown symbol `{sym}`")),
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
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let l = line.to_lowercase();
                if skipped && l.starts_with('+') || l == ".end" || l.starts_with(".end ") {
                    continue;
                }
                let d = directive(line, &mut nl.warnings);
                skipped = d.is_none();
                directives.extend(d);
            }
        }
    }
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
    let prefix_of = |def: &SymbolDef| def.prefix.chars().next().unwrap_or('X').to_ascii_uppercase();
    let mut taken: HashSet<String> = items.iter().filter_map(|it| match it {
        Item::Sym { sym, attrs, .. } => {
            let p = prefix_of(lib.get(sym)?);
            attrs.get("InstName").filter(|n| !n.is_empty()).map(|n| spice_inst(n, p).to_uppercase())
        }
        _ => None,
    }).collect();
    let mut seen = HashSet::new();
    let mut jumped: HashMap<String, String> = HashMap::new(); // nets already tied by jumpers
    let mut any_ac = false; // some V/I source has an AC amplitude (else .ac results are all 0)
    for (i, it) in items.iter().enumerate() {
        let Item::Sym { sym, attrs, .. } = it else { continue };
        let Some(def) = lib.get(sym).cloned() else { continue };

        let prefix = prefix_of(&def);
        let inst = match attrs.get("InstName").filter(|n| !n.is_empty()) {
            Some(n) => spice_inst(n, prefix),
            None => (1..).map(|n| format!("{prefix}{n}")).find(|n| taken.insert(n.to_uppercase())).unwrap(),
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
        // ponytail: a sensor on every device (+1 node, +1 branch each); sense only probed devices
        // if huge imported schematics get slow
        if SENSED.contains(prefix) && !nodes.is_empty() {
            // reserved in both builds (identical names), never clashing with the user's own
            let mut src = format!("Vsense_{inst}");
            while !taken.insert(src.to_uppercase()) {
                src.push('_');
            }
            let mut node = format!("{inst}_sense");
            while !used.insert(node.to_lowercase()) {
                node.push('_');
            }
            nl.probes[i] = format!("i({})", src.to_lowercase());
            nl.sensors.push(Sensor { source: src.to_lowercase(), node: node.to_lowercase(), inst: inst.to_lowercase() });
            if sense {
                lines.push(format!("{src} {} {node} 0", nodes[0]));
                if prefix == 'M' && nodes.len() == 4 && nodes[3] == nodes[0] {
                    nodes[3] = node.clone(); // bulk tied to the drain stays tied to it
                }
                nodes[0] = node;
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
        assert_eq!((out.3.as_str(), get("r1#branch").3.as_str()), ("V", "A"));
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
        assert_eq!(pz_statement("R1 a b 1k\n.PZ in 0 out 0 vol\n* c\n+ zer\nR2 b 0 1k\n+ x\n").as_deref(), Some(".pz in 0 out 0 vol  zer"));
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
        assert_eq!((unit("out"), unit("r1#branch"), unit("v1#branch")), ("V", "A", "A"), "{op:?}");
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
            let deck = prepare(&nl.text, None, &mut vec![]).with_sensors(nl.sensors.clone());
            let run = crate::ngspice::run(&deck).unwrap_or_else(|log| panic!("{name}\n{}\n{}", nl.text, log.join("\n")));
            let op = |n: &str| run.op.iter().find(|o| o.0 == n).map(|o| o.1.hypot(o.2)).unwrap_or_else(|| panic!("{name}: no {n} in {:?}", run.op));
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
