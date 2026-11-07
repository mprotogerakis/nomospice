# nomospice

A circuit simulator with schematic capture, click-to-probe waveforms, `.asc`
files — with a modern UI, ngspice as the engine, a Rust core and a React frontend (Tauri 2).

## What it does

- **Workflow**: `R C L D V I G W N S T` place parts, wires, labels and directives; `⌃R`/`⌃E`
  rotate and mirror; `⌘C`/`⌘X`/`⌘V` copy, cut and paste (also from one schematic into another);
  `F2`/`P`/`⌘K` opens the component library; `⌘↵` runs. After a run, click a wire
  to plot its voltage, a part to plot its current — in `.tran`, `.dc` **and `.ac`** (0 V current
  sensors in the simulated netlist, so currents have phase). Near a pin, its own current: `Ib(Q1)`,
  `Ig(M1)`, `Ix(U1:OUT)`. Expressions take these names: `I(R1)-I(R2)`, `V(in)*I(R1)`.
- **File format**: opens and saves `.asc` (Latin-1/UTF-16, every unknown line kept). 
- **Forms instead of syntax**: sources (sine, pulse, exp, FM, PWL with a waveform preview, AC, Rser/Cpar),
  every analysis, `.param`, `.model` (parameters explained), `.meas`, `.ic`, `.temp`, `.include` are edited
  in forms that show the SPICE text they write; number fields show how SPICE reads them and catch
  `1M` (milli, not mega) and `10F` (femto). Typing SPICE directly still works everywhere.
- **Analyses**: `.tran`, `.ac` (Bode: magnitude + phase panes), `.dc` (nested sweeps as curve
  families), `.op`, `.noise` (spectrum + totals), `.tf`, `.pz`, `.sens`; Live mode re-runs on every edit.
  **A result tab per analysis**: keep e.g. a transient and an AC analysis in one schematic, each tab with its own traces, zoom and cursors.
- **Parameter sweeps**: `.step param C list 47n 100n 220n` (also lin/oct/dec, a source's DC value,
  `temp`, up to three nested) runs the analysis once per value; every trace becomes a family of
  curves, one per step, and `.meas` is measured per step. **Monte Carlo**: `{mc(1k,0.05)}`,
  `flat(x)`, `gauss(x)` draw anew in each run of `.step param run 1 100 1`.
- **Laplace sources**: an E or G source with `Laplace=1/(1+s*tau)` (any ratio of polynomials in s,
  `.param`s inside) works in `.op`, `.ac` and `.tran`.
- **Measurements**: `.meas` results are listed with the plot.
- **Poles and zeros**: `.pz` results as a map in the s-plane (× poles, ○ zeros, unstable half-plane
  shaded, f₀ and damping ζ per pole) and a **root locus**: sweep a `.param` or an R/C/L value and see
  the poles move, one `.pz` run per step.
- **Waveforms** (uPlot): one pane per unit (units from the expression: V·A = W, integrals in J), a
  right axis per pane for traces of other sizes, synced zoom, cursors A/B with Δ and 1/Δx, average,
  RMS and integral between them (shift-drag marks the range), expressions in ngspice syntax (a
  constant draws a reference line), a part's power (right-click > Plot power), copy as CSV;
  an **FFT** of a transient (of A–B or the shown range; Hann window, dB of the amplitude), a log
  or fixed y axis per pane, a trace moved into another unit's pane (on its right axis). A single-frequency `.ac` (`.ac lin 1 50 50`) gives a phasor table
  (magnitude ∠ phase) instead of a plot.
- **SVG export** for slides and papers (right-click the canvas or the plot): the schematic or the plots as
  shown, in a light print style, readable by Inkscape, browsers, PowerPoint and Keynote.
- **Model libraries**: the freeware **Micro-Cap 12** library
  (one click downloads the original package from the Internet Archive; ~45,000 models and
  subcircuits) and your own folders. Search, preview, place — subcircuits get a generated box symbol;
  definitions and their dependencies are pulled into the netlist automatically.
- **Git**: branch and file state in the status bar; History panel to commit, compare a revision
  on the canvas (added/removed parts highlighted) and restore it.
- **MCP server**: `nomospice --mcp` lets AI assistants read/write schematics, simulate them and
  search the libraries (see below).
- **Examples** on the start screen, one per feature: RC transient and Bode, inverting op-amp, CE
  amplifier, diode sweep, BJT curves, noise, phasors, Sallen-Key poles and root locus, RC timing with
  `.meas`, buck converter (efficiency), rectifier (RMS, power), result tabs, `Prefix X` subcircuit,
  `.step` of a capacitor, a Laplace low-pass, Monte Carlo of an RC filter.
- Context menus everywhere, light/dark theme, crash recovery for unsaved work, automatic updates
  from the GitHub releases, macOS / Windows / Linux.

## Download

[Releases](https://github.com/mprotogerakis/nomospice/releases): Windows installer (x64) and a signed,
notarized macOS app (Apple Silicon, macOS 11+). Both include ngspice. From 0.1.5 on the app looks for a
newer release at startup (status bar: *Update x.y.z*; About → *Check for updates*) and installs it after
asking; updates are signed and verified against the key built into the app.

## Requirements (building from source)

| | |
|---|---|
| ngspice shared library | macOS `brew install libngspice` · Debian/Ubuntu `apt install libngspice0` · Windows: `NGSPICE_LIB=…\ngspice.dll` |
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
node scripts/licenses.mjs                        # after dependency changes: THIRD-PARTY-LICENSES.txt + NOTICE.md
scripts/macos-release.sh                         # signed + notarized macOS app with its own ngspice (see script)
NOTES_FILE=notes.md scripts/publish-release.sh   # release: macOS build + Windows installers from CI + updater files
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
| `src-tauri/symbols/*.asy` | built-in symbols |
| `src/Schematic.tsx`, `Waveform.tsx`, `Dialogs.tsx`, `History.tsx`, `Menu.tsx` | canvas, plots, dialogs, version history, context menus |
| `src/store.ts`, `actions.ts`, `geom.ts` | state, editing actions, geometry |

## License

AGPL-3.0-or-later (it includes code from OpenPCB). Third-party software and its licenses: [NOTICE.md](NOTICE.md)
(overview) and [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt) (every component with its full license
text); in the app under About (ⓘ in the toolbar).

LTspice is a trademark of Analog Devices, Inc.; Micro-Cap is a trademark of Spectrum Software. nomospice is an independent project, not affiliated with or endorsed by them, and ships none of their files. It may read and write the LTspice file formats for compatibility and import the Micro-Cap library.
