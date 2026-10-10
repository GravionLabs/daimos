import { commands, events } from "./bindings";

/** What the palette needs from the Rust shell; tests replace it. */
export interface Host {
  /** Calls `callback` every time the window is shown; returns an unsubscribe function. */
  onShown(callback: () => void): () => void;
  /** Tells the shell the first frame after a show is on screen. */
  firstPaint(): void;
  hide(): void;
  /** Tells the shell how many result rows are visible so the window can fit them. */
  resize(rows: number): void;
}

export const tauriHost: Host = {
  onShown(callback) {
    const unlisten = events.windowShown.listen(callback);
    return () => void unlisten.then((fn) => fn());
  },
  firstPaint: () => void commands.firstPaint(),
  hide: () => void commands.hideWindow(),
  resize: (rows) => void commands.resizeWindow(rows),
};
