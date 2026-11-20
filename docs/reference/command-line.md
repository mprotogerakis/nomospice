# Command line and MCP

## Starting nomospice

```sh
nomospice [file] [--run]
nomospice --mcp
nomospice --version
nomospice --help
```

| Argument | Does |
|---|---|
| `file` | opens an `.asc`, `.asy` or netlist file at start |
| `--run` | runs the file's analysis once it is open |
| `--mcp` | starts the MCP server on standard input and output instead of the app |
| `--version`, `--help` | print the version or a short usage and exit, without a window |

On macOS the program is `nomospice.app/Contents/MacOS/nomospice`.

## Environment

| Variable | Does |
|---|---|
| `NGSPICE_LIB` | the ngspice shared library to load, instead of the one next to the program or the system's |

## MCP tools

The MCP server offers these tools to an AI assistant (see [Let an AI assistant use nomospice](../how-to/ai-assistant.md)). Each takes a schematic as `path` (an `.asc` or a netlist; absolute, or relative to where nomospice was started) or as `asc` text.

| Tool | Arguments | Returns |
|---|---|---|
| `simulate` | `path` or `asc`; `traces`: ngspice expressions; `points`: samples per trace (default 200, at most 5000) | the analysis and its scale, all vector names, operating-point values, `.meas` results, warnings, the log, and the traces asked for |
| `netlist` | `path` or `asc` | the netlist with warnings, each item's nets, each part's current expression and pin names |
| `read_schematic` | `path` or `asc` | parts (instance, symbol, value, pin nets), net labels, directives |
| `write_schematic` | `path`, `asc`, `overwrite` | writes the file; an existing one only with `overwrite` |
| `library_search` | `query`, `kind` (a model type or `SUBCKT`), `limit` | matching models and subcircuits |
| `library_get` | `name` | the part's kind, pins, library and symbol; the `.model` text only of nomospice's built-in models (a model library's text stays on the computer: in the netlist a comment line stands in its place, and for a circuit using one, or a file it includes, `simulate` passes on neither ngspice's log nor the warnings about the deck) |
| `examples` | | the example schematics as `.asc` text |
