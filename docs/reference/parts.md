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
| `bv`, `bi` | behavioral voltage and current sources: `V=V(a)*V(b)`, `I=V(in)/1k`, any formula of voltages, currents and `time` | B |
| `tline`, `ltline` | transmission lines: lossless (`Td=50n Z0=50`), lossy (a `LTRA` model; the built-in one is 10 m of 50 Ω coax) | T, O |
| `opamp2` | an op-amp with supply pins: output within the supplies less `Vsat`, slew rate, `GBW`, `Aol`; as Value also an op-amp of the model libraries (`TL072` …) | X |
| `and2`, `nand2`, `or2`, `nor2`, `xor2`, `not`, `dff` | logic gates and a D flip-flop (rising edge), drawn per DIN EN 60617, made of behavioral sources: levels `Vlow`/`Vhigh` (0/5 V), threshold halfway, `Td` the outputs' time constant | X |
| `sw`, `csw` | voltage- and current-controlled switches | S, W |
| `opamp` | an op-amp (a subcircuit) | X |
| `jumper` | a connection between two nets | |

The ground is a net label named `0`.

## Default models

Without a model library, these models are known: the diode 1N4148, the transistors 2N3904 and 2N3906, and switch models. More come from [model libraries](../how-to/model-libraries.md).

## Symbols from elsewhere

A schematic's symbols are looked up in this order: the built-in symbols, the schematic's own folder, an installed LTspice's symbols, and the model library folders. A subcircuit without a symbol gets a generated box with its pins. A symbol of type *block* beside a schematic of the same name stands for that schematic: see [Hierarchy](../explanation/hierarchy.md).
