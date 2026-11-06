# nomospice

A circuit simulator that works like LTspice — schematic capture, click-to-probe waveforms, `.asc`
files — with a modern UI, ngspice as the engine, a Rust core and a React frontend (Tauri 2).

## What it does

- **LTspice workflow**: `R C L D V I G W N S T` place parts, wires, labels and directives; `⌃R`/`⌃E`
  rotate and mirror; `F2`/`P`/`⌘K` opens the component library; `⌘↵` runs. After a run, click a wire
  to plot its voltage, a part to plot its current (`I`, `Ic`, `Id`) — in `.tran`, `.dc` **and `.ac`**
  (every R C L D Q M J gets a 0 V current sensor in the simulated netlist, so currents have phase).
- **LTspice files**: opens and saves `.asc` (Latin-1/UTF-16 like LTspice, every unknown line kept;
  checked against 2,321 LTspice example files). Symbols from a local LTspice install are used for
  parts nomospice does not ship. LTspice-only syntax is translated or reported (`.tran 1m`, `SINE`,
  `Rser=`/`Lser=`/`Rpar=`/`Cpar=`, diode `N=`, `ako:` models, `MISC\JUMPER`, `.step` …).
- **Analyses**: `.tran`, `.ac` (Bode: magnitude + phase panes), `.dc` (nested sweeps as curve
  families), `.op`, `.noise` (spectrum + totals), `.tf`, `.pz`, `.sens`; Live mode re-runs on every edit.
- **Waveforms** (uPlot): one pane per unit, synced zoom, cursors A/B with Δ and 1/Δx, expressions in
  ngspice syntax, copy as CSV.
- **Model libraries**: indexes LTspice's `lib/cmp` + `lib/sub`, the freeware **Micro-Cap 12** library
  (one click downloads the original package from the Internet Archive; ~45,000 models and
  subcircuits) and your own folders. Search, preview, place — subcircuits get a generated box symbol;
  definitions and their dependencies are pulled into the netlist automatically.
- **Git**: branch and file state in the status bar; History panel to commit, compare a revision
  on the canvas (added/removed parts highlighted) and restore it.
- **MCP server**: `nomospice --mcp` lets AI assistants read/write schematics, simulate them and
  search the libraries (see below).
- Context menus everywhere, light/dark theme, crash recovery for unsaved work, macOS / Windows / Linux.

## Requirements

| | |
|---|---|
| ngspice shared library | macOS `brew install libngspice` · Debian/Ubuntu `apt install libngspice0` · Windows: bundled (or `NGSPICE_LIB=…\ngspice.dll`) |
| Build | Rust (stable), Node ≥ 20, pnpm |

ngspice is loaded at runtime (`NGSPICE_LIB`, next to the executable, Homebrew/Linux paths, or the
system loader); its XSPICE code models are loaded too, which vendor macro models need.

## Develop

```sh
pnpm install
pnpm tauri dev                                   # app
pnpm tauri dev -- -- examples/rc_bode.asc --run  # open a file and simulate
cd src-tauri && cargo test                       # netlister, ngspice, library, git, MCP
NOMOSPICE_CORPUS=<folder with .asc> cargo test corpus -- --ignored      # .asc round-trip check
NOMOSPICE_MC12=<Micro-Cap LIBRARY folder> cargo test microcap -- --ignored
pnpm tauri build                                 # installers; CI builds all three OSes (.github/workflows)
```

## MCP

```json
{ "mcpServers": { "nomospice": { "command": "/path/to/nomospice", "args": ["--mcp"] } } }
```

Tools: `simulate` (traces downsampled), `netlist`, `read_schematic`, `write_schematic`,
`library_search`, `library_get`, `examples`.

## Layout

| | |
|---|---|
| `src-tauri/src/schematic.rs` | model, `.asc`/`.asy` I/O, net extraction, netlisting, `prepare` (includes, safety filter) |
| `src-tauri/src/ngspice.rs` | libngspice via `libloading`: run, traces, halt, reload after fatal errors |
| `src-tauri/src/library.rs` | model/subckt index, dependency resolution, generated subckt symbols |
| `src-tauri/src/git.rs`, `mcp.rs`, `main.rs` | git, MCP server, Tauri commands, atomic save, recovery |
| `src-tauri/symbols/*.asy` | built-in symbols (LTspice-compatible pins) |
| `src/Schematic.tsx`, `Waveform.tsx`, `Dialogs.tsx`, `History.tsx`, `Menu.tsx` | canvas, plots, dialogs, version history, context menus |
| `src/store.ts`, `actions.ts`, `geom.ts` | state, editing actions, geometry |

## License

AGPL-3.0-or-later (it includes code from OpenPCB). Third-party notices: [NOTICE.md](NOTICE.md).
