//! `nomospice --mcp`: a Model Context Protocol server on stdio (newline-delimited JSON-RPC 2.0),
//! so AI clients can read/write schematics, netlist and simulate them with ngspice, and search
//! the model libraries, without the GUI.

use crate::{library, ngspice, schematic};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL: &str = "2025-06-18";

fn tools() -> Value {
    let path = json!({ "type": "string", "description": "Path to an LTspice-compatible .asc schematic, or a SPICE netlist (.cir, .net, .sp, .spi, .ckt: read as a schematic of one directive holding it); absolute, or relative to the server's working directory" });
    let asc = json!({ "type": "string", "description": "Schematic as .asc text (alternative to path)" });
    json!([
        {
            "name": "simulate",
            "description": "Simulate a schematic with ngspice. Returns the analysis (plot, scale), all vector names, operating-point/result values, .meas results (LTspice syntax works), netlist warnings, the ngspice log tail on failure (when the circuit uses a model library's text or an included file, which stay on this computer: not the log, nor the warnings about the deck), and the requested traces downsampled to `points` samples. A .step run lists its steps; each trace then comes as {steps: [{step, x, re, im}]}, one curve per step, `points` shared by them. Trace expressions use ngspice syntax plus part currents by name: v(\"net\"), v(\"a\")-v(\"b\"), I(R1) for the current into R1's pin 1, Ic(Q1)/Ib(Q1)/Ie(Q1), Id(M1)/Ig(M1)/Is(M1), Ix(U1:OUT) for a subcircuit's pin (the netlist tool lists them per part as pin_currents), i(v1) for sources, db(v(\"out\")).",
            "inputSchema": { "type": "object", "properties": {
                "path": path, "asc": asc,
                "traces": { "type": "array", "items": { "type": "string" }, "description": "ngspice expressions to return as waveforms" },
                "points": { "type": "integer", "description": "samples per trace (default 200, max 5000)" }
            } }
        },
        {
            "name": "netlist",
            "description": "The SPICE netlist of a schematic (as shown to users; simulation adds current sensors), with warnings, per-item net names, the ngspice expression for each part's current and the names of its pin currents (pin_currents: I(R1), Ib(Q1), Ix(U1:OUT)).",
            "inputSchema": { "type": "object", "properties": { "path": path, "asc": asc } }
        },
        {
            "name": "read_schematic",
            "description": "Parts (instance, symbol, value, pin nets), net labels and SPICE directives of a schematic.",
            "inputSchema": { "type": "object", "properties": { "path": path, "asc": asc } }
        },
        {
            "name": "write_schematic",
            "description": "Write .asc text to a file (Latin-1 like LTspice). Refuses to replace an existing file unless overwrite is true.",
            "inputSchema": { "type": "object", "required": ["path", "asc"], "properties": {
                "path": path, "asc": asc, "overwrite": { "type": "boolean" }
            } }
        },
        {
            "name": "library_search",
            "description": "Search symbols' models and subcircuits in the configured libraries (LTspice, Micro-Cap 12, user folders). kind: a model type (D, NPN, PNP, NMOS, PMOS, VDMOS, NJF, PJF) or SUBCKT; several separated by |.",
            "inputSchema": { "type": "object", "required": ["query"], "properties": {
                "query": { "type": "string" }, "kind": { "type": "string" }, "limit": { "type": "integer" }
            } }
        },
        {
            "name": "library_get",
            "description": "A library part: its kind, pins and library. The .model/.subckt text only for nomospice's built-in models; a model library's text stays on this computer (vendor, LTspice and Micro-Cap libraries come with their own terms), and nomospice puts it into the netlist itself when the part is simulated.",
            "inputSchema": { "type": "object", "required": ["name"], "properties": { "name": { "type": "string" } } }
        },
        {
            "name": "examples",
            "description": "Example schematics (.asc text) shipped with nomospice, a good starting point for the .asc format: symbols res cap ind voltage current diode npn pnp nmos pmos opamp ..., grid of 16 units, pins listed by the netlist tool.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

/// Relative tool paths resolve against the directory the server was started in (the process
/// itself moves to $HOME so ngspice never reads a stray .spiceinit).
fn resolve(base: &Path, p: &str) -> PathBuf {
    let p = Path::new(p);
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
}

/// The schematic of a request (inline `asc` or a file `path`); `lib` is set to its folder (its own
/// .asy symbols), as in the app.
fn schematic_of(base: &Path, args: &Value, lib: &mut schematic::Library) -> Result<(Vec<schematic::Item>, Option<PathBuf>), String> {
    let (items, path) = schematic_at(base, args)?;
    lib.set_dir(path.as_deref().and_then(Path::parent), false);
    Ok((items, path))
}

fn schematic_at(base: &Path, args: &Value) -> Result<(Vec<schematic::Item>, Option<PathBuf>), String> {
    match (args["asc"].as_str(), args["path"].as_str()) {
        (Some(text), _) => Ok((schematic::parse_asc(text), None)),
        (None, Some(p)) => {
            let path = resolve(base, p);
            if schematic::is_symbol(&path.to_string_lossy()) {
                return Err(format!("{}: a symbol, not a schematic", path.display()));
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((schematic::load(&path.to_string_lossy(), &bytes).0, Some(path))) // (a netlist too)
        }
        _ => Err("give `path` or `asc`".into()),
    }
}

fn downsample(v: &[f64], n: usize) -> Vec<f64> {
    ngspice::even(v.len(), n).into_iter().map(|k| v[k]).collect()
}

fn call(base: &Path, name: &str, args: &Value) -> Result<Value, String> {
    let mut lib = schematic::Library::builtin();
    match name {
        "simulate" => {
            let (items, path) = schematic_of(base, args, &mut lib)?;
            let shown = schematic::netlist(&items, &mut lib);
            let sim = schematic::netlist_for_sim(&items, &mut lib);
            let mut warnings = shown.warnings.clone();
            let dir = path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf));
            // (the deck holds a model library's text: a library's definitions, a file it includes. Then
            // ngspice's log and the warnings about the deck are not passed on: ngspice echoes lines it
            // rejects, rewritten, and a warning may quote a statement: no line can be told free of it)
            let mut about_deck = vec![];
            let (prepared, read) = sim.prepared_reading(dir.as_deref(), &mut about_deck);
            let private = !sim.library.is_empty() || read;
            if private && !about_deck.is_empty() {
                warnings.push(format!("{} warnings about the deck are not passed on: it holds a model library's text (nomospice shows them)", about_deck.len()));
            } else {
                warnings.extend(about_deck);
            }
            // `nomospice --mcp` is a process of its own (stdio, one request at a time): no GUI, no Halt
            // and no root locus share this engine, so the run and its traces below cannot be interleaved
            let run = match ngspice::run(&prepared) {
                Ok(run) => run,
                Err(log) => {
                    if private {
                        warnings.push("ngspice's log is not passed on: the circuit uses a model library's text (nomospice shows the log)".into());
                    }
                    return Ok(json!({ "ok": false, "warnings": warnings, "log": (!private).then(|| &log[log.len().saturating_sub(40)..]) }));
                }
            };
            let points = args["points"].as_u64().unwrap_or(200).clamp(2, 5000) as usize;
            let mut traces = serde_json::Map::new();
            for expr in args["traces"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                traces.insert(expr.into(), match ngspice::trace(expr, &run.scale, run.id) {
                    // a .step run: one curve per step (end to end in the trace), each downsampled
                    Ok(t) if t.steps.as_ref().is_some_and(|ix| !ix.is_empty() && t.re.len() % ix.len() == 0) => {
                        let ix = t.steps.as_deref().unwrap_or_default();
                        let n = t.re.len() / ix.len(); // (a table, .op: one value per step, no x)
                        // (`points` for the whole answer: shared by the steps' curves, at least 2 each, so
                        // very many steps: the first points/2 of them)
                        let ix = &ix[..ix.len().min((points / 2).max(1))];
                        let each = (points / ix.len()).max(2);
                        let part = |v: &[f64], k: usize| if v.is_empty() { vec![] } else { downsample(&v[k * n..(k + 1) * n], each) };
                        json!({ "steps": ix.iter().enumerate().map(|(k, &s)| json!({
                            "step": run.steps.get(s), "x": part(&t.x, k), "re": part(&t.re, k), "im": t.im.as_ref().map(|im| part(im, k)),
                        })).collect::<Vec<_>>() })
                    }
                    Ok(t) => json!({
                        "x": downsample(&t.x, points),
                        "re": downsample(&t.re, points),
                        "im": t.im.as_ref().map(|im| downsample(im, points)),
                    }),
                    Err(e) => json!({ "error": e }),
                });
            }
            Ok(json!({
                "ok": true, "plot": run.plot, "scale": run.scale, "aborted": run.aborted,
                // .step: each step's label; values, measurements and traces are per step (`name [step]`)
                "steps": run.steps,
                "vectors": run.vectors,
                "values": run.op.iter().map(|(n, re, im, unit)| json!({ "name": n, "re": re, "im": im, "unit": unit })).collect::<Vec<_>>(),
                // .meas results (value null: ngspice could not evaluate it)
                "measurements": run.meas.iter().map(|(n, v, info, _)| json!({ "name": n, "value": v, "info": info })).collect::<Vec<_>>(),
                // (a note not already among the warnings: a bad .step is said once)
                "warnings": warnings.iter().chain(run.notes.iter().filter(|n| !warnings.iter().any(|w| n.contains(w.as_str())))).collect::<Vec<_>>(), "traces": traces,
            }))
        }
        "netlist" => {
            let (items, _) = schematic_of(base, args, &mut lib)?;
            let nl = schematic::netlist_shared(&items, &mut lib); // (a model library's text left out)
            Ok(json!({ "netlist": nl.text, "warnings": nl.warnings, "nets": nl.nets, "instances": nl.insts, "current_probes": nl.probes, "pin_currents": nl.pin_currents }))
        }
        "read_schematic" => {
            let (items, _) = schematic_of(base, args, &mut lib)?;
            let nl = schematic::netlist(&items, &mut lib);
            let mut parts = vec![];
            let mut labels = vec![];
            let mut directives = vec![];
            for (i, it) in items.iter().enumerate() {
                match it {
                    schematic::Item::Sym { sym, attrs, x, y, rot, .. } => parts.push(json!({
                        "instance": nl.insts[i], "symbol": sym, "value": attrs.get("Value"), "attributes": attrs,
                        "pins": lib.get(sym).map(|d| d.pins.iter().map(|p| p.name.clone()).collect::<Vec<_>>()).unwrap_or_default(),
                        "nets": nl.nets[i], "at": [x, y], "orientation": rot,
                    })),
                    schematic::Item::Flag { name, x, y, .. } => labels.push(json!({ "name": name, "net": nl.nets[i].first(), "at": [x, y] })),
                    schematic::Item::Text { text, directive: true, .. } => directives.push(text.clone()),
                    _ => {}
                }
            }
            Ok(json!({ "parts": parts, "labels": labels, "directives": directives, "warnings": nl.warnings }))
        }
        "write_schematic" => {
            let path = resolve(base, args["path"].as_str().ok_or("`path` missing")?);
            let asc = args["asc"].as_str().ok_or("`asc` missing")?;
            if schematic::is_symbol(&path.to_string_lossy()) {
                return Err(format!("{}: a symbol, not a schematic", path.display()));
            }
            if path.exists() && !args["overwrite"].as_bool().unwrap_or(false) {
                return Err(format!("{} exists; pass overwrite: true to replace it", path.display()));
            }
            let items = schematic::parse_asc(asc);
            crate::write_atomic(&path, &schematic::encode(&schematic::write_asc(&items))).map_err(|e| e.to_string())?;
            Ok(json!({ "written": path, "items": items.len() }))
        }
        "library_search" => {
            let ix = library::index();
            let hits = ix.as_ref().map(|ix| ix.search(args["query"].as_str().unwrap_or(""), args["kind"].as_str(), args["limit"].as_u64().unwrap_or(30).min(500) as usize)
                .into_iter().map(|e| json!({ "name": e.name, "kind": e.kind, "pins": e.pins, "source": e.source, "symbol": e.symbol })).collect::<Vec<_>>()).unwrap_or_default();
            Ok(json!(hits))
        }
        "library_get" => {
            let name = args["name"].as_str().ok_or("`name` missing")?;
            let ix = library::index();
            let ix = ix.as_ref().ok_or(format!("`{name}` is not in the libraries"))?;
            let e = ix.get(name).ok_or(format!("`{name}` is not in the libraries"))?;
            // (resolved on the index held: another index() here could wait on itself)
            let text = e.builtin().then(|| ix.resolve(name, &mut Default::default()).join("\n")).filter(|t| !t.is_empty());
            Ok(json!({ "name": e.name, "kind": e.kind, "pins": e.pins, "source": e.source, "symbol": e.symbol, "text": text }))
        }
        "examples" => Ok(json!(schematic::EXAMPLES.iter().map(|(n, s)| json!({ "name": n, "asc": s })).collect::<Vec<_>>())),
        _ => Err(format!("unknown tool {name}")),
    }
}

fn handle(base: &Path, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or("");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "nomospice", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Circuit simulation with ngspice on LTspice-compatible .asc schematics. Start with `examples` to see the format, `library_search` for part models, then `simulate` with traces."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call(base, name, &args) {
                Ok(v) => json!({ "content": [{ "type": "text", "text": if v.is_string() { v.as_str().unwrap().to_string() } else { v.to_string() } }] }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            })
        }
        _ if id.is_none() => return None, // notifications (initialized, cancelled ...)
        _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
    };
    let id = id?;
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

/// Serve until stdin closes. `base`: the working directory the client started us in.
pub fn serve(base: PathBuf) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&base, &msg),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_text_stays_local() {
        let _s = crate::ngspice::TEST_SIM.lock();
        // a library folder of the user's: its model in the netlist and library_get without its text;
        // nomospice's own 2N3904 with it
        let dir = std::env::temp_dir().join(format!("nomospice-mcp-lib-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("q.lib"), ".model QLOCAL NPN(BF=123\n+ VA=9)\n").unwrap();
        library::set_sources(vec![library::Source { name: "test".into(), path: dir.clone(), enabled: true }]);
        let asc = "SYMBOL npn 0 0 R0\nSYMATTR InstName Q1\nSYMATTR Value QLOCAL\nSYMBOL npn 100 0 R0\nSYMATTR InstName Q2\nSYMATTR Value 2N3904\n";
        let r = call(&dir, "netlist", &json!({ "asc": asc })).unwrap();
        let text = r["netlist"].as_str().unwrap();
        assert!(!text.contains("BF=123") && text.contains("QLOCAL: from a model library") && text.contains(".model 2N3904 NPN("), "{text}");
        let g = call(&dir, "library_get", &json!({ "name": "QLOCAL" })).unwrap();
        assert!(g["kind"] == "NPN" && g["text"].is_null(), "{g}");
        assert!(call(&dir, "library_get", &json!({ "name": "2N3904" })).unwrap()["text"].as_str().unwrap().contains("BF=180"));
        // a failed run: no log when the deck holds a library's text (a library's model, a file it
        // reads, written with an Ω the deck's reading drops too), the log without
        let own = asc.replace("QLOCAL", "2N3904");
        let fails = |asc: &str, more: &str| call(&dir, "simulate", &json!({ "asc": format!("{asc}TEXT 0 300 Left 2 !.tran 1m\nTEXT 0 340 Left 2 !XU1 a b nosuchsubckt\n{more}") })).unwrap();
        for r in [fails(asc, ""), fails(&own, &format!("TEXT 0 380 Left 2 !Ω.inc {}\n", dir.join("q.lib").display()))] {
            assert!(r["ok"] == false && r["log"].is_null() && r["warnings"].to_string().contains("log is not passed on"), "{r}");
        }
        for r in [fails(&own, ""), fails(&own, "TEXT 0 380 Left 2 !.lib opamp.sub\n")] {
            assert!(r["ok"] == false && r["log"].is_array(), "{r}");
        }
        // a definition written in a comment before the library's own: the library's left out all the same
        let r = call(&dir, "netlist", &json!({ "asc": format!("{asc}TEXT 0 300 Left 2 !* .model QLOCAL NPN(BF=123\n") })).unwrap();
        assert_eq!(r["netlist"].as_str().unwrap().matches("BF=123").count(), 1, "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn protocol_and_tools() {
        let _s = crate::ngspice::TEST_SIM.lock();
        let base = std::env::temp_dir();
        let init = handle(&base, &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } })).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "nomospice");
        assert!(handle(&base, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let list = handle(&base, &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        assert!(list["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "simulate"));
        let asc = schematic::EXAMPLES[0].1;
        let r = call(&base, "simulate", &json!({ "asc": asc, "traces": ["v(\"out\")", "i(vsense_r1)"], "points": 50 })).unwrap();
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["traces"]["v(\"out\")"]["re"].as_array().unwrap().len(), 50);
        assert!(r["traces"]["i(vsense_r1)"]["re"].is_array(), "{r}");
        let parts = call(&base, "read_schematic", &json!({ "asc": asc })).unwrap();
        assert_eq!(parts["parts"].as_array().unwrap().len(), 3);
        let bad = handle(&base, &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "write_schematic", "arguments": { "path": std::env::temp_dir(), "asc": "" } } })).unwrap();
        assert_eq!(bad["result"]["isError"], true);
    }
}
