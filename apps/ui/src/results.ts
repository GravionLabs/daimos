import type { Item, ResultBatch } from "./bindings";

/** Rows the list shows at once; the window is sized for this many (see `height_for` in Rust). */
export const VIEWPORT_ROWS = 8;

/** The text of the badge on results of a script extension. */
export const UNSANDBOXED_LABEL = "Runs unsandboxed";

/**
 * Whether the item comes from a script extension, which runs with the user's rights. Wasm items
 * (`true`) and the built-in ones (`null`/missing) get no badge.
 */
export function runsUnsandboxed(item: Item | null | undefined): boolean {
  return item?.sandboxed === false;
}

/** The one query session of the palette window. */
export const SESSION = 1;

/** The results of the running query, merged from the batches of all providers. */
export interface Results {
  /** Providers, the heavier ones first, ties in the order their first batch arrived. */
  providers: string[];
  weights: Record<string, number>;
  byProvider: Record<string, Item[]>;
  /** All items, provider after provider. */
  items: Item[];
  /** The id of the selected item; it survives merges as long as the item is in the list. */
  selected: string | null;
}

export const emptyResults: Results = {
  providers: [],
  weights: {},
  byProvider: {},
  items: [],
  selected: null,
};

/**
 * Merges one batch. The first batch of a query (`seq` 0) replaces everything and selects the first
 * item; later batches add to their provider's items and keep the selection, unless the selected
 * item is gone.
 */
export function applyBatch(state: Results, batch: ResultBatch): Results {
  const fresh = batch.seq === 0;
  const base = fresh ? emptyResults : state;
  // The weight is a float: a NaN would arrive as null.
  const weights = { ...base.weights, [batch.provider]: batch.weight ?? 1 };
  // `sort` is stable: equal weights keep the order of arrival.
  const providers = (
    base.providers.includes(batch.provider) ? base.providers : [...base.providers, batch.provider]
  ).sort((a, b) => (weights[b] ?? 1) - (weights[a] ?? 1));
  const byProvider = {
    ...base.byProvider,
    [batch.provider]: [...(base.byProvider[batch.provider] ?? []), ...batch.items],
  };
  const items = providers.flatMap((provider) => byProvider[provider] ?? []);
  const keep = !fresh && base.selected !== null && items.some((item) => item.id === base.selected);
  return {
    providers,
    weights,
    byProvider,
    items,
    selected: keep ? base.selected : (items[0]?.id ?? null),
  };
}

export function selectedIndex(results: Results): number {
  return results.items.findIndex((item) => item.id === results.selected);
}

/** Selects the item at `index`, clamped to the list. */
export function selectAt(results: Results, index: number): Results {
  if (results.items.length === 0) return results;
  const clamped = Math.max(0, Math.min(results.items.length - 1, index));
  return { ...results, selected: results.items[clamped]?.id ?? results.selected };
}

export function move(results: Results, delta: number): Results {
  return selectAt(results, Math.max(selectedIndex(results), 0) + delta);
}

/** The first visible row after the selection moved to `index`: scrolls only when it has to. */
export function scrollInto(first: number, index: number, length: number): number {
  let next = first;
  if (index < next) next = index;
  else if (index >= next + VIEWPORT_ROWS) next = index - VIEWPORT_ROWS + 1;
  return Math.max(0, Math.min(next, Math.max(0, length - VIEWPORT_ROWS)));
}

/** The primary action of an item: the first one. */
export function primaryAction(item: Item) {
  return item.actions[0] ?? null;
}

/** Splits a title into runs that are or are not highlighted. Indices count characters, not UTF-16 units. */
export function highlightRuns(
  title: string,
  indices: number[] = [],
): { text: string; hit: boolean }[] {
  const hits = new Set(indices);
  const runs: { text: string; hit: boolean }[] = [];
  for (const [position, char] of Array.from(title).entries()) {
    const hit = hits.has(position);
    const last = runs[runs.length - 1];
    if (last && last.hit === hit) last.text += char;
    else runs.push({ text: char, hit });
  }
  return runs;
}

/** The provider that returned the item. */
export function providerOf(results: Results, itemId: string): string | null {
  return results.providers.find((p) => results.byProvider[p]?.some((i) => i.id === itemId)) ?? null;
}

/**
 * Whether a keyboard event is the shortcut an action names, like `Ctrl+Enter` or `Ctrl+P`. Ctrl
 * stands for Cmd on macOS, so either modifier counts.
 */
export function matchesShortcut(event: KeyboardEvent, shortcut: string): boolean {
  const parts = shortcut.split("+");
  const key = parts.pop() ?? "";
  const wants = (name: string) => parts.includes(name);
  if ((event.ctrlKey || event.metaKey) !== wants("Ctrl")) return false;
  if (event.shiftKey !== wants("Shift") || event.altKey !== wants("Alt")) return false;
  return event.key.toLowerCase() === key.toLowerCase();
}
