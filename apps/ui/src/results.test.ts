import type { Item, ResultBatch } from "./bindings";
import {
  applyBatch,
  emptyResults,
  highlightRuns,
  move,
  scrollInto,
  selectAt,
  selectedIndex,
  VIEWPORT_ROWS,
} from "./results";

function item(id: string, title = id): Item {
  return {
    id,
    title,
    subtitle: null,
    icon: null,
    keywords: [],
    accessories: [],
    actions: [],
    detail: null,
  };
}

function batch(seq: number, provider: string, ids: string[], done = false): ResultBatch {
  return { session: 1, seq, provider, items: ids.map((id) => item(id)), done };
}

const ids = (items: Item[]) => items.map((i) => i.id);

describe("applyBatch", () => {
  it("shows the first batch and selects its first item", () => {
    const state = applyBatch(emptyResults, batch(0, "apps", ["a", "b"]));
    expect(ids(state.items)).toEqual(["a", "b"]);
    expect(state.selected).toBe("a");
  });

  it("merges later batches in place without moving the selection", () => {
    let state = applyBatch(emptyResults, batch(0, "apps", ["a", "b", "c"]));
    state = selectAt(state, 2);
    state = applyBatch(state, batch(1, "calc", ["x"]));
    state = applyBatch(state, batch(2, "apps", ["d"]));
    expect(ids(state.items)).toEqual(["a", "b", "c", "d", "x"]);
    expect(state.selected).toBe("c");
  });

  it("moves the selection to the first item only when the selected one disappeared", () => {
    let state = applyBatch(emptyResults, batch(0, "apps", ["a", "b"]));
    state = selectAt(state, 1);
    // The next query lists b again: the selection resets because a new query starts at the top.
    const next = applyBatch(state, batch(0, "apps", ["b", "a"]));
    expect(next.selected).toBe("b");
    // Within a query, a merge never removes items, so the selection stays; if it did vanish ...
    const gone: typeof state = { ...state, selected: "zzz" };
    expect(applyBatch(gone, batch(1, "calc", ["q"])).selected).toBe("a");
  });

  it("a new query replaces the old results at its first batch", () => {
    let state = applyBatch(emptyResults, batch(0, "apps", ["a"]));
    state = applyBatch(state, batch(1, "calc", ["x"]));
    state = applyBatch(state, batch(0, "calc", ["y"]));
    expect(ids(state.items)).toEqual(["y"]);
  });

  it("an empty first batch clears the list", () => {
    let state = applyBatch(emptyResults, batch(0, "apps", ["a"]));
    state = applyBatch(state, batch(0, "apps", [], true));
    expect(state.items).toEqual([]);
    expect(state.selected).toBeNull();
  });
});

describe("keyboard movement", () => {
  const state = applyBatch(emptyResults, batch(0, "apps", ["a", "b", "c"]));

  it("moves down and up and stops at the ends", () => {
    expect(move(state, 1).selected).toBe("b");
    expect(move(move(state, 1), -1).selected).toBe("a");
    expect(move(state, -1).selected).toBe("a");
    expect(move(selectAt(state, 2), 1).selected).toBe("c");
  });

  it("jumps to the first and last item", () => {
    expect(selectAt(state, 0).selected).toBe("a");
    expect(selectAt(state, 99).selected).toBe("c");
    expect(selectedIndex(selectAt(state, 99))).toBe(2);
  });

  it("does nothing on an empty list", () => {
    expect(move(emptyResults, 1)).toBe(emptyResults);
  });
});

describe("scrollInto", () => {
  it("scrolls only when the selection leaves the viewport", () => {
    expect(scrollInto(0, 3, 100)).toBe(0);
    expect(scrollInto(0, VIEWPORT_ROWS, 100)).toBe(1);
    expect(scrollInto(5, 2, 100)).toBe(2);
    expect(scrollInto(5, 6, 100)).toBe(5);
  });

  it("never scrolls past the end or before the start", () => {
    expect(scrollInto(0, 99, 100)).toBe(100 - VIEWPORT_ROWS);
    expect(scrollInto(3, 0, 5)).toBe(0);
    expect(scrollInto(0, 0, 3)).toBe(0);
  });
});

describe("highlightRuns", () => {
  it("groups matched characters into runs", () => {
    expect(highlightRuns("Firefox", [0, 4, 6])).toEqual([
      { text: "F", hit: true },
      { text: "ire", hit: false },
      { text: "f", hit: true },
      { text: "o", hit: false },
      { text: "x", hit: true },
    ]);
    expect(highlightRuns("abc", [0, 1])).toEqual([
      { text: "ab", hit: true },
      { text: "c", hit: false },
    ]);
  });

  it("counts characters, not UTF-16 units", () => {
    expect(highlightRuns("😀ab", [1])).toEqual([
      { text: "😀", hit: false },
      { text: "a", hit: true },
      { text: "b", hit: false },
    ]);
  });

  it("returns one plain run without matches", () => {
    expect(highlightRuns("Calc")).toEqual([{ text: "Calc", hit: false }]);
    expect(highlightRuns("")).toEqual([]);
  });
});
