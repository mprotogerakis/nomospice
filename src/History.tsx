import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { GitCommit, Item } from "./types";
import { commit, loadSymbols, refreshGit, useS } from "./store";
import { Button } from "./ui/button";
import { FIELD_BASE, FIELD_FOCUS } from "./ui/focus";
import { cn } from "@/lib/utils";

/** Version history of the open schematic (git): commit it, compare a revision on the canvas, restore it. */
export function History() {
  const path = useS((s) => s.path);
  const git = useS((s) => s.git);
  const dirty = useS((s) => s.dirty);
  const diff = useS((s) => s.diff);
  const [log, setLog] = useState<GitCommit[]>([]);
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");
  const reload = () => {
    setErr("");
    return path && git?.repo ? invoke<GitCommit[]>("git_log", { path }).then(setLog, (e) => setErr(String(e))) : setLog([]);
  };
  useEffect(() => void reload(), [path, git?.repo, git?.file]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!path) return <div className="font-sans text-text-tertiary">Save the schematic to put it under version control.</div>;
  if (!git?.repo) return (
    <div className="flex items-center gap-3 font-sans">
      <span className="text-text-secondary">This folder is not a git repository.</span>
      <Button onClick={() => invoke("git_init", { path }).then(refreshGit, (e) => setErr(String(e)))}>Initialize repository</Button>
      {err && <span className="text-status-danger">{err}</span>}
    </div>
  );
  const doCommit = async () => {
    setErr("");
    try {
      await invoke("git_commit", { path, message: msg });
      setMsg("");
      await refreshGit();
      reload();
    } catch (e) {
      setErr(String(e));
    }
  };
  const revision = async (c: GitCommit) => {
    const items = await invoke<Item[]>("git_show", { path, hash: c.hash });
    await loadSymbols(items);
    return items;
  };
  const act = (f: () => Promise<unknown>) => () => (setErr(""), f().catch((e) => setErr(String(e))));
  const clean = git.file === "clean";
  return (
    <div className="flex flex-col gap-2 font-sans">
      <form className="flex items-center gap-2" onSubmit={(e) => (e.preventDefault(), doCommit())}>
        <input
          className={cn(FIELD_BASE, FIELD_FOCUS, "h-[24px] flex-1 px-2")}
          placeholder={dirty ? "Save first, then commit" : clean ? "No changes to commit" : `Commit message for ${path.split(/[\\/]/).pop()}`}
          value={msg} onChange={(e) => setMsg(e.target.value)} disabled={dirty || clean}
        />
        <Button type="submit" variant="primary" disabled={dirty || clean || !msg.trim()}>Commit</Button>
        <span className="text-2xs text-text-tertiary">⎇ {git.branch} · {git.file}</span>
      </form>
      {err && <div className="text-status-danger">{err}</div>}
      {diff && (
        <div className="flex items-center gap-2 rounded-control border border-selection/40 bg-selection-soft px-2 py-1 text-text">
          Comparing with {diff.hash.slice(0, 7)} on the canvas: <span style={{ color: "var(--series-3)" }}>added</span> · <span style={{ color: "var(--series-8)" }}>removed</span>
          <Button size="sm" className="ml-auto" onClick={() => useS.setState({ diff: null })}>Close comparison</Button>
        </div>
      )}
      <table className="w-full font-mono text-2xs">
        <tbody>
          {log.map((c) => (
            <tr key={c.hash} className={cn("group h-[24px] border-b border-border-subtle", diff?.hash === c.hash && "bg-selection-soft")}>
              <td className="w-16 text-text-tertiary">{c.hash.slice(0, 7)}</td>
              <td className="w-32 text-text-secondary">{c.date}</td>
              <td className="w-28 truncate text-text-secondary">{c.author}</td>
              <td className="truncate font-sans text-xs text-text">{c.subject}</td>
              <td className="row-actions w-44 text-right">
                <Button size="sm" variant="ghost" onClick={act(async () => useS.setState({ diff: { hash: c.hash, items: await revision(c) } }))}>Compare</Button>
                <Button size="sm" variant="ghost" title="Replace the schematic with this version (undoable)"
                  onClick={act(async () => (commit(await revision(c)), useS.setState({ diff: null })))}>Restore</Button>
              </td>
            </tr>
          ))}
          {!log.length && <tr><td className="py-1 font-sans text-text-tertiary">No commits of this file yet.</td></tr>}
        </tbody>
      </table>
    </div>
  );
}
