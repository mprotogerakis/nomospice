# Hierarchy

A **block** is a part that stands for a schematic. It lets you draw a circuit once and use it many times, and keeps large circuits readable.

## What makes a block

A symbol is a block when a schematic of the same name lies beside it, and the symbol calls for one: its type is *block*, or it is a subcircuit call (prefix X) without a model of its own. *Create block symbol* makes such a symbol from a schematic. The pins are the schematic's labels marked as ports, matched by name: inputs on the left, outputs on the right.

## What runs

When a schematic uses a block, nomospice netlists the block's schematic as a subcircuit, `.subckt`, with the ports as its connections, and places it with an `X` line. A block inside a block works the same way. Nets inside a block are its own: a label `mid` inside does not connect to a `mid` outside, except through a port. The ground `0` is everywhere the same ground.

A block's schematic keeps what it needs as a subcircuit: its parts, its `.param`, `.model` and `.include` directives, and its `.ic`. Its analyses and `.meas` directives are its own business when you open it on its own, and are left out of the subcircuit.

## Parameters

A block can take parameters, as a part takes its value:

1. the block's own value, `R=10k`, wins;
2. else a `.param R=1k` in the block's schematic is the default;
3. else the value of `R` in the schematic above is used.

So a block can be configured where it is placed, have a sensible default, or simply follow the circuit around it.

## What is refused

Some things cannot be a subcircuit, and nomospice says so in the warnings instead of netlisting something wrong: a block that places itself (directly or through others), a schematic that lacks a label for one of the symbol's pins, a port on ground, two ports on the same net, or a schematic that cannot be read.
