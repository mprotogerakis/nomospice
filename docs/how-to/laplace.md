# Use a transfer function as a source

A controlled source can follow a transfer function H(s): a quick way to model a filter, a sensor or a controller without building it from parts.

1. Place a voltage-controlled voltage source, `e` (or a voltage-controlled current source, `g`), from the component library (++p++).
2. Wire its control input to the signal and its output to the load.
3. Give it the value `Laplace=1/(1+s*tau)`, and define the parameter with a directive: `.param tau=1m`.

Any ratio of polynomials in `s` works, and `.param` names may appear in it. The source works in operating point, AC and transient analyses.

The example laplace_lowpass is a first-order low-pass with τ = 1 ms: its AC tab shows the corner at 159 Hz, its Transient tab the step response 1 − e^(−t/τ).
