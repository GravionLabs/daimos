import { vi } from "vitest";
import type { Icon, Item, ResultBatch } from "./bindings";
import type { Host } from "./host";

export function fakeHost() {
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
    firstPaint: vi.fn(),
    copy: vi.fn(() => Promise.resolve()),
    loadView: vi.fn(() => Promise.reject(new Error("no view"))),
    submitForm: vi.fn(() => Promise.resolve()),
    query: vi.fn(),
    runAction: vi.fn(),
    iconSrc: (icon: Icon) => (icon.type === "file" ? `asset://${icon.path}` : null),
    hide: vi.fn(),
    resize: vi.fn(),
  };
  return {
    host,
    show: () => shown?.(),
    sendQuery: (text: string) => query?.(text),
    send: (batch: ResultBatch) => results?.(batch),
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
