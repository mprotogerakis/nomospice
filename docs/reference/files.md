# Files and folders

## File types

| Extension | What | nomospice |
|---|---|---|
| `.asc` | schematic | opens and saves; Latin-1, or UTF-16 if needed; unknown lines kept |
| `.asy` | symbol | opens and saves in the symbol editor; a block symbol's `.asy` lies beside its `.asc` |
| `.cir`, `.net`, `.sp`, `.spi`, `.ckt` | SPICE netlist | opens as a schematic of one directive; saves as `.asc` beside it |
| `.lib`, `.sub`, `.mod`, `.inc` … | models | included with `.include`/`.lib`; searchable when in a model library folder |
| `.svg` | picture | exported schematic or plot |
| `.raw` | results | exported, ngspice's binary format |

Saving writes a new file next to the old one and then replaces it, so a failed save never leaves half a file.

## Where nomospice keeps its things

| What | Where |
|---|---|
| model library list | `libraries.json` in the configuration folder: `~/Library/Application Support/de.nomospice.app` (macOS), `%APPDATA%\de.nomospice.app` (Windows), `~/.config/de.nomospice.app` (Linux) |
| recovery copies of unsaved work | `recovery-…json` in the data folder: the same folder on macOS and Windows, `~/.local/share/de.nomospice.app` on Linux |
| a downloaded Micro-Cap 12 library | `libraries/Micro-Cap 12` in the data folder |
| crash reports of the app itself | `panic.log` in the configuration folder |

The theme and the panel sizes are kept by the app's web view.
