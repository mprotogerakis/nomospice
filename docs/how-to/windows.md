# Work with several windows

Each window holds one schematic or symbol. Open as many as you like.

## Open windows

- ++cmd+n++ (File → New Window) opens an empty window.
- ++cmd+o++ opens a file in the current window if it holds nothing yet, else in a new window.
- A file is open in one window only: opening it again brings its window to the front. Opening it again from its own window reads it from disk again, after asking about unsaved changes.

On macOS, *Window → Merge All Windows* turns the windows into tabs of one window. The Window menu lists them all.

## Copy parts between schematics

Select parts, press ++cmd+c++ (or ++cmd+x++), go to the other window and press ++cmd+v++. The parts follow the mouse until you click to place them. Built-in and library symbols are found there too; a symbol from the first schematic's own folder must be in the other's folder as well. The same works between two running copies of nomospice, and the Edit menu's Cut, Copy and Paste do the same as the keys.

## Close and quit

- ++cmd+w++ closes the window, after asking about unsaved changes. The last window is not closed but emptied.
- ++cmd+q++ asks each window with unsaved changes in turn, bringing it to the front. *Cancel* in any of them keeps all windows open.

## Simulate in several windows

Every window simulates on its own, and its Halt stops only its own run. All windows share one ngspice, which holds the results of the latest run only. If another window ran in between, nomospice runs again when you add a trace or export results. See [One ngspice, many windows](../explanation/windows-and-engine.md).
