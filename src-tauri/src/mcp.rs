//! `nomospice --mcp`: a Model Context Protocol server on stdio (newline-delimited JSON-RPC 2.0),
//! so AI clients can read/write schematics, netlist and simulate them with ngspice, and search
//! the model libraries, without the GUI.

use crate::{library, ngspice, schematic};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL: &str = "2025-06-18";

fn tools() -> Value {
    let path = json!({ "type": "string", "description": "Path to an LTspice-compatible .asc schematic (absolute, or relative to the server's working directory)" });
    let asc = json!({ "type": "string", "description": "Schematic as .asc text (alternative to path)" });
    json!([
        {
            "name": "simulate",
            "description": "Simulate a schematic with ngspice. Returns the analysis (plot, scale), all vector names, operating-point/result values, netlist warnings, the ngspice log tail on failure, and the requested traces downsampled to `points` samples. Trace expressions use ngspice syntax: v(\"net\"), v(\"a\")-v(\"b\"), i(vsense_r1) for the current into R1's pin 1 (every R C L D Q M J has such a sensor), i(v1) for sources, db(v(\"out\")).",
            "inputSchema": { "type": "object", "properties": {
                "path": path, "asc": asc,
                "traces": { "type": "array", "items": { "type": "string" }, "description": "ngspice expressions to return as waveforms" },
                "points": { "type": "integer", "description": "samples per trace (default 200, max 5000)" }
            } }
        },
        {
            "name": "netlist",
            "description": "The SPICE netlist of a schematic (as shown to users; simulation adds current sensors), with warnings, per-item net names and the ngspice expression for each part's current.",
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
            "description": "The .model/.subckt text of a library part, with the definitions it depends on.",
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

fn schematic_of(base: &Path, args: &Value) -> Result<(Vec<schematic::Item>, Option<PathBuf>), String> {
    match (args["asc"].as_str(), args["path"].as_str()) {
        (Some(text), _) => Ok((schematic::parse_asc(text), None)),
        (None, Some(p)) => {
            let path = resolve(base, p);
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((schematic::parse_asc(&schematic::decode(&bytes)), Some(path)))
        }
        _ => Err("give `path` or `asc`".into()),
    }
}

fn downsample(v: &[f64], n: usize) -> Vec<f64> {
    if v.len() <= n || n < 2 {
        return v.to_vec();
    }
    (0..n).map(|k| v[k * (v.len() - 1) / (n - 1)]).collect()
}

fn call(base: &Path, name: &str, args: &Value) -> Result<Value, String> {
    let mut lib = schematic::Library::builtin();
    match name {
        "simulate" => {
            let (items, path) = schematic_of(base, args)?;
            let shown = schematic::netlist(&items, &mut lib);
            let sim = schematic::netlist_for_sim(&items, &mut lib);
            let mut warnings = shown.warnings.clone();
            let dir = path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf));
            let prepared = sim.prepared(dir.as_deref(), &mut warnings);
            // `nomospice --mcp` is a process of its own (stdio, one request at a time): no GUI, no Halt
            // and no root locus share this engine, so the run and its traces below cannot be interleaved
            let run = match ngspice::run(&prepared) {
                Ok(run) => run,
                Err(log) => return Ok(json!({ "ok": false, "warnings": warnings, "log": log.iter().rev().take(40).rev().collect::<Vec<_>>() })),
            };
            let points = args["points"].as_u64().unwrap_or(200).clamp(2, 5000) as usize;
            let mut traces = serde_json::Map::new();
            for expr in args["traces"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                traces.insert(expr.into(), match ngspice::trace(expr, &run.scale, run.id) {
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
                "vectors": run.vectors,
                "values": run.op.iter().map(|(n, re, im, unit)| json!({ "name": n, "re": re, "im": im, "unit": unit })).collect::<Vec<_>>(),
                "warnings": warnings, "traces": traces,
            }))
        }
        "netlist" => {
            let (items, _) = schematic_of(base, args)?;
            let nl = schematic::netlist(&items, &mut lib);
            Ok(json!({ "netlist": nl.text, "warnings": nl.warnings, "nets": nl.nets, "instances": nl.insts, "current_probes": nl.probes }))
        }
        "read_schematic" => {
            let (items, _) = schematic_of(base, args)?;
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
            let defs = library::definitions(name, &mut Default::default()).ok_or(format!("`{name}` is not in the libraries"))?;
            Ok(json!(defs.join("\n")))
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
