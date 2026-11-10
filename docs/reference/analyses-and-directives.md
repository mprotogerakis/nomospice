# Analyses and directives

nomospice runs ngspice. Directives are SPICE dot commands written as text in the schematic (press ++s++). Most of them can be written with forms that show the line they write; typing SPICE directly works everywhere.

## Analyses

Written with the simulation command form (the toolbar's sliders). One analysis per kind runs; others are kept as commented alternatives in result tabs.

| Directive | Form tab | Notes |
|---|---|---|
| `.tran` | Transient | stop time, time step, start of saving, maximum step, `uic` |
| `.ac` | AC | decade, octave or linear sweep; one frequency gives a phasor table |
| `.dc` | DC sweep | a source, a parameter or `temp`; nested sweeps make curve families |
| `.noise` | Noise | output and input source; spectra and totals |
| `.op` | DC op pnt | the operating point as a table |
| `.tf` | Transfer fn | gain, input and output resistance |
| `.pz` | Poles/zeros | poles and zeros as a map in the s-plane |
| `.sens` | Sensitivity | the sensitivities of an output to every part |

## Other directives

Written with the directive form (++s++), its tabs in brackets.

| Directive | Does |
|---|---|
| `.param` (Parameters) | names a value: `.param R=1k`; use it in values as `{R}` or `{R*2}` |
| `.model` (Model) | a device model, its parameters explained in the form |
| `.meas` (Measurement) | a number measured after the run: a statistic over an interval, a value at a point, where a signal reaches a value, or the delay between two crossings |
| `.ic` (Initial conditions) | node voltages a transient starts from |
| `.temp` (Temperature) | the circuit temperature |
| `.include`, `.lib` (Include file) | a model file, whole or one section |
| `.step` (Text) | runs the analysis once per value: `.step param C list 47n 100n 220n`; also `lin`, `oct`, `dec`, a source's value, `temp`; up to three nested |

Other dot commands that ngspice knows pass through as they are written.

## Numbers

SPICE reads a number with a suffix, case-insensitively:

| Suffix | f | p | n | u, µ | m | k | Meg | G | T |
|---|---|---|---|---|---|---|---|---|---|
| Factor | 10⁻¹⁵ | 10⁻¹² | 10⁻⁹ | 10⁻⁶ | 10⁻³ | 10³ | 10⁶ | 10⁹ | 10¹² |

`1M` is one *milli*, and `10F` is ten *femto*. Value fields show how SPICE reads what you type and warn about these two.

## Values and expressions

- `{expression}`: a value computed from parameters, such as `{R*2}`.
- `mc(x, tol)`: x with a tolerance, uniform within ±tol (as a fraction); `flat(x)`: uniform within ±x; `gauss(x)`: normal with σ = x. They draw anew in each run of a `.step`, so `.step param run 1 100 1` is a Monte Carlo analysis.
- `Laplace=H(s)` on an `e` or `g` source: a transfer function, any ratio of polynomials in `s`.
- Independent sources (V, I) are edited in a form: DC, sine, pulse, exponential, FM, piecewise linear (with a preview), an AC amplitude, and series resistance and parallel capacitance.

## Trace expressions

*Add trace* takes ngspice expressions: `V(out)`, `V(a,b)`, `I(R1)`, `Ib(Q1)` (a pin's current), arithmetic on them, and functions such as `db()`, `ph()`, `group_delay()`. Units follow from the expression: a voltage times a current is in watts, an integral of power in joules.
