# Compatibility

## Schematics

nomospice reads and writes the `.asc` schematic format that is widespread in teaching and in vendors' application notes. Opening and saving a file keeps it as it was, including lines nomospice does not understand; it is written in Latin-1, or in UTF-16 where its characters need it. A schematic you draw in nomospice opens elsewhere, as long as it uses parts and directives that simulator knows.

## Simulation

The schematics are simulated with **ngspice**, an open-source SPICE. Most dot commands mean the same in every SPICE. Where schematics in the `.asc` world use forms of their own, nomospice handles them when it writes the netlist and runs it:

- `.step`, including nested sweeps and Monte Carlo runs with `mc()`, `flat()` and `gauss()`, each step a run of its own;
- `.meas` in its common syntax, measured per step;
- `Laplace=` sources;
- blocks and the parameters passed to them.

Some features of other simulators have no counterpart in ngspice. The log tells you what ngspice did not accept.

## Model libraries

Vendors publish SPICE models for their parts. Many of them work in ngspice as they are, some need features ngspice lacks. nomospice can use:

- your own folders of model files;
- the freeware **Micro-Cap 12** library, downloaded from the Internet Archive on request: about 45,000 models and subcircuits;
- the libraries of an installed LTspice, which it finds by itself.

## What nomospice ships

nomospice is free software under the AGPL-3.0-or-later and contains code from OpenPCB. It ships ngspice and its own symbols and default models, and none of the files of other simulators or of the Micro-Cap package: they are used where they are installed or downloaded by you. The licenses of everything included are listed under About (ⓘ in the toolbar).

LTspice is a trademark of Analog Devices, Inc.; Micro-Cap is a trademark of Spectrum Software. nomospice is an independent project, not affiliated with or endorsed by them.
