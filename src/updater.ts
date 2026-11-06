// Updates from the GitHub releases: check latest.json (signed artifacts, see scripts/publish-release.sh),
// then download, verify, install and restart on request.
import { create } from "zustand";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { useS } from "./store";

export const useUpdate = create<{ update: Update | null; busy: string }>(() => ({ update: null, busy: "" }));

/** Looks for a newer release; `manual` (About dialog) also reports "up to date" and errors. */
export async function checkForUpdate(manual = false) {
  if (import.meta.env.DEV && !manual) return; // dev builds are not the released app
  // releases (and latest.json) cover macOS and Windows; Linux builds come from CI or source
  if (/linux/i.test(navigator.userAgent) && !/android/i.test(navigator.userAgent)) {
    if (manual) await message("Automatic updates are available for the macOS and Windows releases; on Linux, build the new version from source.", { title: "nomospice" });
    return;
  }
  try {
    const update = await check();
    useUpdate.setState({ update });
    if (manual && !update) await message("nomospice is up to date.", { title: "nomospice" });
  } catch (e) {
    if (manual) await message(`Could not check for updates: ${e}`, { title: "nomospice", kind: "error" });
  }
}

let asking = false; // a second click while the confirmation is open must not start a second install

/** Downloads, verifies (the updater's public key) and installs the update, then restarts. */
export async function installUpdate() {
  const { update, busy } = useUpdate.getState();
  if (!update || busy || asking) return;
  asking = true;
  const notes = update.body?.trim() ? `\n\n${update.body.trim().slice(0, 600)}` : "";
  const unsaved = useS.getState().dirty ? "\n\nThe schematic has unsaved changes: they are lost unless you save first." : "";
  const yes = await ask(`Install nomospice ${update.version} and restart?${notes}${unsaved}`, { title: "Update", kind: unsaved ? "warning" : "info" }).finally(() => (asking = false));
  if (!yes) return;
  let total = 0, got = 0;
  try {
    useUpdate.setState({ busy: "Downloading…" });
    await update.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? 0;
      if (e.event === "Progress") {
        got += e.data.chunkLength;
        if (total) useUpdate.setState({ busy: `Downloading ${Math.round((100 * got) / total)} %` });
      }
      if (e.event === "Finished") useUpdate.setState({ busy: "Installing…" });
    });
    await relaunch(); // Windows: the installer has already closed the app
  } catch (e) {
    useUpdate.setState({ busy: "" });
    await message(`The update failed: ${e}`, { title: "nomospice", kind: "error" });
  }
}
