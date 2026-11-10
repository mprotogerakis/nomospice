# Choose an analysis

## Set the analysis

Click the sliders in the toolbar (Simulate → Edit Simulation Command…). The form has a tab for each analysis and shows the SPICE line it writes. Click OK; a new analysis is placed where you click.

| Tab | Directive | What you get |
|---|---|---|
| Transient | `.tran` | voltages and currents over time |
| AC | `.ac` | magnitude and phase over frequency (Bode, Nyquist, Nichols plots); a single frequency gives a table of phasors |
| DC sweep | `.dc` | values over a swept source; a nested sweep gives a family of curves |
| Noise | `.noise` | noise spectra (add the trace `onoise_spectrum` or `inoise_spectrum`) and their totals under *Results* |
| DC op pnt | `.op` | a table of node voltages and currents |
| Transfer fn | `.tf` | gain, input and output resistance at the operating point |
| Poles/zeros | `.pz` | a map of poles and zeros: see [Find poles and zeros](poles-and-zeros.md) |
| Sensitivity | `.sens` | how much the output depends on each part (add a part's name as a trace) |

You can also type the directive yourself: press ++s++ and write it in the *Text* tab.

## Keep several analyses

Click the plus beside the result tabs to add another analysis. It is written as a comment under the first, so the schematic's own analysis stays as it is, and gets a result tab of its own that opens and runs at once. Each tab keeps its own traces, zoom and cursors. The cross on a tab removes its analysis from the schematic (++cmd+z++ brings it back). Double-click a tab to edit its analysis.

## Run

- ++cmd+enter++ (Simulate → Run) runs the analysis of the shown tab.
- Halt (the toolbar's square while it runs) stops it; the results so far are kept, marked aborted.
- **Live mode**, the lightning in the toolbar, runs again shortly after every edit.

The status bar shows the analysis that ran, and *stale* when the schematic changed since. ++cmd+l++ opens the log and the netlist.

## The examples

The start screen has an example for most analyses: rc_transient, rc_bode, diode_sweep and bjt_curves (DC sweeps), noise_rc, phasor_rlc (a single-frequency AC analysis), pz_sallen_key, and analysis_tabs with several analyses in tabs.
