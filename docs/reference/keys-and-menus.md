# Keys and menus

The keys are the macOS ones. On Windows and Linux press ++ctrl++ for ++cmd++. In nomospice, ++question++ or ++f1++ shows a short version of this page.

## Drawing

| Keys | Does |
|---|---|
| ++r++ ++c++ ++l++ ++d++ ++v++ ++i++ | place a resistor, capacitor, inductor, diode, voltage or current source |
| ++p++, ++f2++, ++cmd+k++ | any part: the component library (symbols, models, subcircuits) |
| ++g++ | ground |
| ++w++, ++f3++ | wire: click, click; ++esc++ ends it |
| ++n++, ++f4++ | net label |
| ++s++ | SPICE directive (`.tran`, `.param`, `.step` …) |
| ++t++ | comment |
| ++cmd+r++, ++cmd+e++ (or ++ctrl+r++, ++ctrl+e++) | rotate, mirror (the part being placed, or the selection) |
| drag | move parts; their wires follow |
| ++m++, ++f7++ | move parts off their wires; ++esc++ or ++f8++ ends it |
| ++cmd+c++ ++cmd+x++ ++cmd+v++ | copy, cut, paste (also between windows) |
| ++cmd+d++, ++f6++ | duplicate |
| ++cmd+z++, ++shift+cmd+z++ | undo, redo |
| ++cmd+a++ | select all |
| ++delete++, ++backspace++ | delete the selection |
| ++f5++ | the delete tool: click what goes |
| ++cmd+f++ | find parts, labels, texts |
| ++space++ | fit the view |
| ++cmd+equal++, ++cmd+minus++ | zoom in, out |
| double-click | edit a part (value, model, its L/W/area) |
| right-click | a part's or the canvas' menu |

## Simulating

| Keys | Does |
|---|---|
| ++cmd+enter++ | run |
| ++cmd+l++ | the log and the netlist |

## Files and windows

| Keys | Does |
|---|---|
| ++cmd+n++ | new window |
| ++cmd+o++ | open (here while this window is empty, else in a window of its own) |
| ++cmd+s++, ++shift+cmd+s++ | save, save as |
| ++cmd+w++ | close the window (the last one is emptied) |
| ++cmd+comma++ | model libraries |
| ++cmd+q++ | quit (asks each window with unsaved changes) |

## Plot

| Mouse | Does |
|---|---|
| click a wire, a part | plot its voltage, its current (near a pin: that pin's current) |
| click, shift-click | cursor A, cursor B |
| shift-drag | the range A–B (average, RMS, integral) |
| drag, wheel, double-click | zoom; zoom and pan; see everything |
| right-click a trace's row | right axis, another unit's pane, group delay, remove |
| right-click the plot | FFT, Nyquist, Nichols, y axis, CSV, SVG, `.raw` |

## The menu bar

| Menu | Items |
|---|---|
| nomospice (macOS) | About nomospice · Check for Updates… · Model Libraries… · Services · Hide · Quit |
| File | New Window · New Symbol · Open… · Close · Save · Save As… · Create Block Symbol… · Export Schematic as SVG… · Export Plot as SVG… · Export Results as .raw… · (Windows, Linux: Model Libraries…, Exit) |
| Edit | Undo · Redo · Cut · Copy · Paste · Duplicate · Delete · Select All · Rotate · Mirror · Find… |
| Place | Wire · Ground · Net Label… · Resistor · Capacitor · Inductor · Diode · Voltage Source · Current Source · Component… · Comment… · SPICE Directive… · Select · Move · Delete Mode |
| Simulate | Run · Halt · Edit Simulation Command… · Live Mode · Add Trace… |
| View | Fit to Window · Zoom In · Zoom Out · Log & Netlist · Version History · Light or Dark Theme · Back to the Schematic Above · Enter Full Screen (macOS) |
| Window (macOS) | Minimize · Zoom · the open windows · Merge All Windows |
| Help | Quick Reference · (Windows, Linux: Check for Updates…, About nomospice) |

On Windows and Linux the menus show the keys but leave them to the window, so a key never runs twice.
