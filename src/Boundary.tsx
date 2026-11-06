import { Component, type ReactNode } from "react";
import { Button } from "./ui/button";

/** A render error shows its message (to copy into a bug report) instead of blanking the window.
 *  Continue renders the children again; the document lives in the store, so nothing is lost. */
export class Boundary extends Component<{ children: ReactNode; onReset?: () => void }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  render() {
    const e = this.state.error;
    if (!e) return this.props.children;
    const text = `${e.message}\n\n${e.stack ?? ""}`;
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-scrim">
        <div className="flex w-[560px] flex-col gap-2 rounded-float border border-menu-border bg-menu-bg p-3 text-xs text-text shadow-dialog">
          <div className="text-sm font-medium text-status-danger">Something went wrong</div>
          <pre className="max-h-[40vh] select-text overflow-auto whitespace-pre-wrap font-mono text-2xs text-text-secondary">{text}</pre>
          <div className="flex justify-end gap-2">
            <Button onClick={() => navigator.clipboard.writeText(text)}>Copy error</Button>
            <Button variant="primary" onClick={() => { this.props.onReset?.(); this.setState({ error: null }); }}>Continue</Button>
          </div>
        </div>
      </div>
    );
  }
}
