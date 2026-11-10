# Open schematics, symbols and netlists

## Open a schematic

Press ++cmd+o++ (File → Open…) and choose an `.asc` file, or drop it on the window. If the window holds nothing yet, the schematic opens there; otherwise it opens in a window of its own. A file that is open already is not opened twice: its window comes to the front.

nomospice reads `.asc` files in Latin-1 and UTF-16 and keeps every line it does not understand, so a schematic made elsewhere keeps its parts when you save it again. Its symbols are looked up among the built-in symbols first, then in the schematic's folder, and then in your [model libraries](model-libraries.md).

## Open a symbol

Open an `.asy` file the same way to edit it: see [Draw and edit symbols](symbols.md).

## Open a netlist

Open a `.cir`, `.net`, `.sp`, `.spi` or `.ckt` file and nomospice makes it a schematic of one SPICE directive that holds the whole netlist. It runs as it is, and you plot its nodes by name with *Add trace*. Analyses inside a `.control` block are turned into dot commands. The first ++cmd+s++ asks where to save the schematic as an `.asc` file, so the netlist itself is never overwritten.

## Add a model file

Drop a `.lib`, `.sub`, `.mod` or `.inc` file on a schematic: nomospice adds an `.include` directive for it, relative to the schematic's folder when the file is inside it.
