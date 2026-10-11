import { vi } from "vitest";
import type { Capabilities, Icon, Item, ResultBatch, SettingsView } from "./bindings";
import type { Host } from "./host";

export function settingsView(extra: Partial<SettingsView> = {}): SettingsView {
  return {
    hotkey: "Alt+Space",
    theme: "system",
    providers: [
      { id: "apps", enabled: true, weight: 1, default_weight: 1 },
      { id: "calc", enabled: true, weight: 10, default_weight: 10 },
    ],
    aliases: [{ alias: "ff", target: "Firefox" }],
    clipboard: {
      enabled: false,
      max_entries: 500,
      max_image_mb: 10,
      level: "available",
      reason: null,
    },
    ...extra,
  };
}

export function capabilitiesView(): Capabilities {
  return {
    session: "x11",
    window_mode: "toplevel",
    hotkey: { state: "active", session: "x11", binding: "Alt+Space", message: null },
    items: [
      {
        id: "hotkey",
        label: "Global hotkey",
        level: "available",
        reason: "Registered through XGrabKey.",
        instructions: null,
      },
      {
        id: "paste",
        label: "Paste into the previous window",
        level: "unavailable",
        reason: "Arrives with M3.",
        instructions: null,
      },
    ],
  };
}

export function fakeHost() {
  let openSettings: (() => void) | undefined;
  let shown: (() => void) | undefined;
  let query: ((query: string) => void) | undefined;
  let results: ((batch: ResultBatch) => void) | undefined;
  const host: Host = {
    onShown: (callback) => {
      shown = callback;
      return () => {
        shown = undefined;
      };
    },
    onQuery: (callback) => {
      query = callback;
      return () => {
        query = undefined;
      };
    },
    onResults: (callback) => {
      results = callback;
      return () => {
        results = undefined;
      };
    },
    ready: vi.fn(() => Promise.resolve()),
    firstPaint: vi.fn(),
    copy: vi.fn(() => Promise.resolve()),
    theme: vi.fn(() =>
      Promise.resolve({ setting: "system" as const, system: "light", accent: null }),
    ),
    onTheme: vi.fn(() => () => {}),
    onOpenSettings: vi.fn((callback: () => void) => {
      openSettings = callback;
      return () => {
        openSettings = undefined;
      };
    }),
    settings: vi.fn(() => Promise.resolve(settingsView())),
    capabilities: vi.fn(() => Promise.resolve(capabilitiesView())),
    setHotkey: vi.fn(() => Promise.resolve(null)),
    setTheme: vi.fn(() => Promise.resolve(settingsView())),
    setProvider: vi.fn(() => Promise.resolve(settingsView())),
    setAliases: vi.fn(() => Promise.resolve(settingsView())),
    paste: vi.fn(() => Promise.resolve({ pasted: true, reason: null })),
    setClipboardHistory: vi.fn((enabled: boolean) =>
      Promise.resolve(settingsView({ clipboard: { ...settingsView().clipboard, enabled } })),
    ),
    loadView: vi.fn(() => Promise.reject(new Error("no view"))),
    submitForm: vi.fn(() => Promise.resolve()),
    query: vi.fn(),
    runAction: vi.fn(() => Promise.resolve()),
    iconSrc: (icon: Icon) => (icon.type === "file" ? `asset://${icon.path}` : null),
    hide: vi.fn(),
    resize: vi.fn(),
  };
  return {
    host,
    show: () => shown?.(),
    sendQuery: (text: string) => query?.(text),
    send: (batch: ResultBatch) => results?.(batch),
    openSettings: () => openSettings?.(),
  };
}

export function item(id: string, extra: Partial<Item> = {}): Item {
  return {
    id,
    title: id,
    subtitle: null,
    icon: null,
    keywords: [],
    accessories: [],
    actions: [
      { id: "open", title: "Open", shortcut: "Enter", kind: "open", payload: id },
      { id: "reveal", title: "Reveal", shortcut: null, kind: "run", payload: null },
    ],
    detail: null,
    ...extra,
  };
}

export function batch(seq: number, items: Item[], provider = "apps"): ResultBatch {
  return { session: 1, seq, provider, items, done: false, weight: 1 };
}

/** `list[index]`, failing the test instead of returning undefined. */
export function at<T>(list: T[], index: number): T {
  const value = list[index];
  if (value === undefined) throw new Error(`no element at ${index}`);
  return value;
}
