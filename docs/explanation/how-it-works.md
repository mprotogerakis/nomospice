# How a simulation runs

A schematic is a drawing; ngspice understands a netlist. Between the two, nomospice does what a careful person would do by hand, every time you run.

## From the drawing to nets

Wires that touch at their ends or at a pin form a **net**. A wire crossing another without a dot is not connected. A **net label** gives its net a name; two labels of the same name connect their nets, even far apart, so labels save wires. The label `0` is the **ground**, the reference every voltage is measured against. A circuit without a ground cannot be solved, and nomospice warns about it.

Nets without a label get numbered names, `N001`, `N002` …, which may change when you edit. Labels are names you can rely on, in the plot and in `.meas` directives.

## From nets to a netlist

Every part becomes one line: its instance name, the nets at its pins in the order its symbol defines, its value, and its model. Directives are copied as written. Model definitions from libraries, and the definitions they depend on, are added; a block becomes a subcircuit (see [Hierarchy](hierarchy.md)). ++cmd+l++ shows the netlist.

What runs is slightly more than that. nomospice adds 0 V sources as current sensors, so that every part's current can be plotted, in AC analyses with its phase too. It copies included files into the netlist, so ngspice never opens a file itself, and it removes `.control` blocks, which ngspice would execute on loading, shell commands included. *as simulated* in the netlist panel shows this version, and the log's line numbers refer to it.

## The run

ngspice runs inside nomospice, as a library. It reads the netlist, solves the circuit for the analysis, and keeps the results as vectors: one per node voltage and current, over time, frequency or the swept value. Halt asks it to stop; what it computed so far is kept.

## From the results to the plot

Nothing is copied until you ask for it: clicking a wire or a part asks ngspice for that one vector, evaluated as an expression, and the plot draws it. That is why plotting is instant even for long runs, and why an expression like `V(in)*I(R1)` costs nothing extra: ngspice evaluates it.

Each run has a number. The results nomospice shows belong to one run, and when you change the schematic they are marked *stale* until you run again. Live mode simply runs again shortly after each edit.

## Result tabs

A schematic can hold several analyses: the one SPICE runs, and alternatives kept as comments. Each gets a result tab with its own traces, zoom and cursors. Switching tabs runs that analysis in place of the other, so the schematic stays a valid file that other programs read as before, with one active analysis.
