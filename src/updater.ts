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
  if (useUpdate.getState().busy || asking) {
    if (manual) await message("An update is being installed.", { title: "nomospice" });
    return; // confirming or installing: the current handle is in use
  }
  try {
    const update = await check();
    if (useUpdate.getState().busy || asking) return void update?.close(); // an install started meanwhile
    const old = useUpdate.getState().update;
    if (old && old !== update) void old.close(); // each check returns a new resource handle
    useUpdate.setState({ update });
    if (manual && !update) await message("nomospice is up to date.", { title: "nomospice" });
    if (manual && update) await installUpdate(); // asks, with the release notes
  } catch (e) {
    // releases cover Apple Silicon Macs and x64 Windows; other builds (Linux, Intel Mac) come from source
    if (/none of the fallback platforms|platform .* (was )?not found/i.test(String(e))) { // TargetsNotFound / TargetNotFound
      if (manual) await message("There are no automatic updates for this kind of build: get the new version from the releases page or build it from source.", { title: "nomospice" });
      return;
    }
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
  let yes = false;
  try {
    yes = await ask(`Install nomospice ${update.version} and restart?${notes}${unsaved}`, { title: "Update", kind: unsaved ? "warning" : "info" });
  } catch (e) {
    console.error("update confirmation:", e);
  } finally {
    asking = false;
  }
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
  } catch (e) {
    useUpdate.setState({ busy: "" });
    return void (await message(`The update failed: ${e}`, { title: "nomospice", kind: "error" }));
  }
  // installed: from here on a failure is only the restart (do not offer the same update again)
  useUpdate.setState({ busy: "Restart to finish" });
  try {
    await relaunch(); // Windows: the installer has already closed the app
  } catch (e) {
    await message(`nomospice ${update.version} is installed: restart nomospice to use it. (${e})`, { title: "nomospice" });
  }
}
