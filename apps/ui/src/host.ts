import { convertFileSrc } from "@tauri-apps/api/core";
import { commands, events, type Icon, type ResultBatch } from "./bindings";

/** What the palette needs from the Rust shell; tests replace it. */
export interface Host {
  /** Calls `callback` every time the window is shown; returns an unsubscribe function. */
  onShown(callback: () => void): () => void;
  /** Calls `callback` with text sent by `daimos show "text"`; returns an unsubscribe function. */
  onQuery(callback: (query: string) => void): () => void;
  /** Tells the shell the first frame after a show is on screen. */
  firstPaint(): void;
  /** Starts a query for the session; the answer arrives through `onResults`. */
  query(session: number, text: string): void;
  /** Calls `callback` with every batch of results; returns an unsubscribe function. */
  onResults(callback: (batch: ResultBatch) => void): () => void;
  /** Runs an action of a result (the window hides when it succeeded). */
  runAction(itemId: string, actionId: string): void;
  /** The URL the web view can load an icon from, or null when there is none (placeholder). */
  iconSrc(icon: Icon): string | null;
  hide(): void;
  /** Tells the shell how many result rows are visible so the window can fit them. */
  resize(rows: number): void;
}

export const tauriHost: Host = {
  onShown(callback) {
    const unlisten = events.windowShown.listen(callback);
    return () => void unlisten.then((fn) => fn());
  },
  onQuery(callback) {
    const unlisten = events.querySet.listen((event) => callback(event.payload));
    return () => void unlisten.then((fn) => fn());
  },
  firstPaint: () => void commands.firstPaint(),
  query: (session, text) => void commands.query(session, text),
  onResults(callback) {
    const unlisten = events.queryResults.listen((event) => callback(event.payload));
    return () => void unlisten.then((fn) => fn());
  },
  runAction: (itemId, actionId) => void commands.runAction(itemId, actionId),
  iconSrc(icon) {
    if (icon.type === "file") return convertFileSrc(icon.path);
    if (icon.type === "dataUrl") return icon.url;
    return null;
  },
  hide: () => void commands.hideWindow(),
  resize: (rows) => void commands.resizeWindow(rows),
};
