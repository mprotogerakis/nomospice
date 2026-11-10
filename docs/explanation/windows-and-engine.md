# One ngspice, many windows

Each window of nomospice is its own document: its schematic, its undo history, its results, zoom and cursors. Some things are shared by all windows of the app.

## Shared: the engine

ngspice runs inside nomospice as a library, once. It can run one simulation at a time and holds the results of the latest run only. So:

- When two windows run at the same moment, the second waits for the first.
- When window B runs after window A, A's results are no longer in ngspice. A's plot is unaffected: what is drawn has been copied out. But adding a trace or exporting in A needs the results again, and nomospice runs A's analysis once more first.
- Halt stops only the run of its own window, and closing a window stops its run.

## Shared: libraries and symbols

The model libraries are the app's, not a window's. When you change them in one window, every window loads its parts anew. A symbol saved in one window is drawn anew in every window with a schematic in that folder.

## Not shared: files

A file is open in one window at most. Opening it again brings that window to the front, and *Save As* refuses a file another window has open, so two windows never write over each other.

## Recovery

Each window keeps its own copy of its unsaved changes. After a crash every copy is offered, each in a window of its own. Quit asks the windows one after another, and a *Cancel* in any of them keeps them all open: a quit never half-happens.
