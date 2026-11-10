# Export schematics, plots and results

## The schematic as SVG

Right-click the canvas and choose *Export schematic as SVG…* (or File → Export Schematic as SVG…). The SVG is drawn in a light print style, whatever theme you use, and opens in Inkscape, browsers, PowerPoint and Keynote.

## The plot as SVG

Right-click the plot and choose *Export plot as SVG…* (File → Export Plot as SVG…). It has the same axes, zoom and cursors as on screen, and a legend.

## The data as CSV

Right-click the plot and choose *Copy data as CSV*: the x values and one column per trace go to the clipboard, ready to paste into a spreadsheet.

## All results as a .raw file

Right-click the plot and choose *Export results as .raw…* (File → Export Results as .raw…): every vector of the run in ngspice's binary raw format, for other waveform viewers and scripts. Part currents keep their names, each step of a `.step` becomes a plot of its own, and a noise analysis adds its totals.

## The netlist

++cmd+l++ shows the netlist in the bottom panel. *as simulated* shows the netlist that actually ran, with current sensors and included files; the log's line numbers refer to that one. Select and copy it from there.
