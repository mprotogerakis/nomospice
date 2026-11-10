# Recover unsaved work

nomospice keeps a copy of every window's unsaved changes, written shortly after each change. A normal close or quit asks you about unsaved changes and removes the copy. If nomospice ends any other way (a crash, a forced quit, the computer turned off), the copy stays.

## Restore after a crash

Start nomospice again. It asks:

> Restore unsaved changes to *file* from the last session?

- **Yes** opens the schematic with your changes, marked unsaved. Save it to keep them.
- **No** deletes the copy.

If several windows had unsaved changes, each is offered in a window of its own. A schematic that had never been saved asks where to save it, as it would have.

## Where the copies are

In nomospice's data folder: `~/Library/Application Support/de.nomospice.app` on macOS, `%APPDATA%\de.nomospice.app` on Windows, `~/.local/share/de.nomospice.app` on Linux. They are named `recovery-…json`. You do not need to touch them.
