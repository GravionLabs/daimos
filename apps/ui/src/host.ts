import { convertFileSrc } from "@tauri-apps/api/core";
import {
  type AliasEntry,
  type Capabilities,
  commands,
  events,
  type Icon,
  type PasteResult,
  type ResultBatch,
  type SettingsView,
  type ThemeInfo,
  type ThemeSetting,
  type View,
} from "./bindings";

/** What the palette needs from the Rust shell; tests replace it. */
export interface Host {
  /** Calls `callback` every time the window is shown; returns an unsubscribe function. */
  onShown(callback: () => void): () => void;
  /** Calls `callback` with text sent by `daimos show "text"`; returns an unsubscribe function. */
  onQuery(callback: (query: string) => void): () => void;
  /** Tells the shell that every event listener is registered; what it sent earlier is replayed. */
  ready(): Promise<void>;
  /** Tells the shell the first frame after a show is on screen. */
  firstPaint(): void;
  /** Starts a query for the session; the answer arrives through `onResults`. */
  query(session: number, text: string): void;
  /** Calls `callback` with every batch of results; returns an unsubscribe function. */
  onResults(callback: (batch: ResultBatch) => void): () => void;
  /** Runs an action of a result (the window hides when it succeeded). */
  runAction(itemId: string, actionId: string): void;
  /** Puts text on the clipboard. */
  copy(text: string): Promise<void>;
  /** The view a `push` or `form` action of a result of `provider` points to. */
  loadView(provider: string, viewId: string): Promise<View>;
  /** Hands the values of a submitted form to its provider. */
  submitForm(provider: string, viewId: string, values: Record<string, string>): Promise<void>;
  /** The color scheme and accent to use now. */
  theme(): Promise<ThemeInfo>;
  /** Calls `callback` when the OS theme or the theme setting changes; returns an unsubscribe function. */
  onTheme(callback: (theme: ThemeInfo) => void): () => void;
  /** Calls `callback` when the tray asks for the settings page; returns an unsubscribe function. */
  onOpenSettings(callback: () => void): () => void;
  settings(): Promise<SettingsView>;
  capabilities(): Promise<Capabilities>;
  /** Tries a new hotkey; resolves to the system's error message, or null when it was set. */
  setHotkey(accelerator: string): Promise<string | null>;
  setTheme(setting: ThemeSetting): Promise<SettingsView>;
  setProvider(id: string, enabled: boolean, weight: number | null): Promise<SettingsView>;
  setAliases(aliases: AliasEntry[]): Promise<SettingsView>;
  setClipboardHistory(enabled: boolean): Promise<SettingsView>;
  /** Hides the window and pastes the clipboard into the previous window; `pasted` is false when it could only be copied (the window stays open then). */
  paste(): Promise<PasteResult>;
  /** The URL the web view can load an icon from, or null when there is none (placeholder). */
  iconSrc(icon: Icon): string | null;
  hide(): void;
  /** Tells the shell how many result rows are visible so the window can fit them. */
  resize(rows: number): void;
}

/** The data of a command result, or its error as an exception. */
async function unwrap<T>(
  result: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>,
): Promise<T> {
  const done = await result;
  if (done.status === "error") throw new Error(done.error);
  return done.data;
}

/** Listeners that are still being registered: `listen` is asynchronous. */
const registrations: Promise<unknown>[] = [];

function track<T>(registration: Promise<T>): Promise<T> {
  registrations.push(registration);
  return registration;
}

export const tauriHost: Host = {
  onShown(callback) {
    const unlisten = track(events.windowShown.listen(callback));
    return () => void unlisten.then((fn) => fn());
  },
  onQuery(callback) {
    const unlisten = track(events.querySet.listen((event) => callback(event.payload)));
    return () => void unlisten.then((fn) => fn());
  },
  async ready() {
    await Promise.all(registrations);
    await commands.uiReady();
  },
  firstPaint: () => void commands.firstPaint(),
  query: (session, text) => void commands.query(session, text),
  onResults(callback) {
    const unlisten = track(events.queryResults.listen((event) => callback(event.payload)));
    return () => void unlisten.then((fn) => fn());
  },
  runAction: (itemId, actionId) => void commands.runAction(itemId, actionId),
  copy: (text) => navigator.clipboard.writeText(text),
  theme: () => commands.theme(),
  onOpenSettings(callback) {
    const unlisten = track(events.openSettings.listen(() => callback()));
    return () => void unlisten.then((fn) => fn());
  },
  settings: () => unwrap(commands.settings()),
  capabilities: () => commands.capabilities(),
  async setHotkey(accelerator) {
    const result = await commands.setHotkey(accelerator);
    return result.status === "ok" ? null : result.error.message;
  },
  async setTheme(setting) {
    await unwrap(commands.setTheme(setting));
    return unwrap(commands.settings());
  },
  setProvider: (id, enabled, weight) => unwrap(commands.setProvider(id, enabled, weight)),
  setAliases: (aliases) => unwrap(commands.setAliases(aliases)),
  setClipboardHistory: (enabled) => unwrap(commands.setClipboardHistory(enabled)),
  paste: () => unwrap(commands.paste()),
  onTheme(callback) {
    const unlisten = track(events.themeChanged.listen((event) => callback(event.payload)));
    return () => void unlisten.then((fn) => fn());
  },
  async loadView(provider, viewId) {
    const result = await commands.loadView(provider, viewId);
    if (result.status === "error") throw new Error(result.error);
    return result.data;
  },
  async submitForm(provider, viewId, values) {
    const result = await commands.submitForm(provider, viewId, values);
    if (result.status === "error") throw new Error(result.error);
  },
  iconSrc(icon) {
    if (icon.type === "file") return convertFileSrc(icon.path);
    if (icon.type === "dataUrl") return icon.url;
    return null;
  },
  hide: () => void commands.hideWindow(),
  resize: (rows) => void commands.resizeWindow(rows),
};
