# Let an AI assistant use nomospice

nomospice includes an MCP server: started with `--mcp`, it lets an AI assistant that speaks the Model Context Protocol read and write schematics, simulate them and search the model libraries. It uses the same libraries as the app.

## Set it up

Add nomospice to the assistant's MCP configuration, with the path to the nomospice program:

```json
{ "mcpServers": { "nomospice": { "command": "/Applications/nomospice.app/Contents/MacOS/nomospice", "args": ["--mcp"] } } }
```

On Windows the program is `nomospice.exe` in the installation folder.

## What the assistant can do

| Tool | Does |
|---|---|
| `simulate` | runs a schematic or netlist and returns the analysis, vector names, results, `.meas` values, warnings, the log, and the traces asked for (downsampled) |
| `netlist` | the SPICE netlist of a schematic, with each item's nets and each part's current expression |
| `read_schematic` | parts, net labels and directives of a schematic |
| `write_schematic` | writes `.asc` text to a file (an existing file only when asked to overwrite it) |
| `library_search` | searches the models and subcircuits of the libraries |
| `library_get` | a library part's kind, pins and library; the `.model` text only of nomospice's own default models |
| `examples` | the example schematics, a good starting point for the `.asc` format |

A model library's text stays on your computer: model files come with their makers' terms. The netlist the assistant gets has a comment line in place of each library definition, and when a circuit uses one (or a file it includes), a failed run's ngspice log and the warnings about the deck are not passed on; nomospice shows them.

Open a schematic the assistant wrote in nomospice to see it, and save it from there.
