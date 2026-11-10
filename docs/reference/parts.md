# Parts and symbols

## Built-in symbols

| Symbol | Part | Prefix |
|---|---|---|
| `res` | resistor | R |
| `cap`, `polcap` | capacitor, polarized capacitor | C |
| `ind` | inductor | L |
| `diode`, `zener`, `schottky`, `led` | diodes | D |
| `npn`, `pnp` | bipolar transistors | Q |
| `nmos`, `pmos` | MOSFETs | M |
| `njf`, `pjf` | JFETs | J |
| `voltage`, `current` | independent sources | V, I |
| `e`, `g` | voltage-controlled voltage and current sources | E, G |
| `sw`, `csw` | voltage- and current-controlled switches | S, W |
| `opamp` | an op-amp (a subcircuit) | X |
| `jumper` | a connection between two nets | |

The ground is a net label named `0`.

## Default models

Without a model library, these models are known: the diode 1N4148, the transistors 2N3904 and 2N3906, and switch models. More come from [model libraries](../how-to/model-libraries.md).

## Symbols from elsewhere

A schematic's symbols are looked up in this order: the built-in symbols, the schematic's own folder, an installed LTspice's symbols, and the model library folders. A subcircuit without a symbol gets a generated box with its pins. A symbol of type *block* beside a schematic of the same name stands for that schematic: see [Hierarchy](../explanation/hierarchy.md).
