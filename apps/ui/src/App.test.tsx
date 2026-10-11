import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { App, mounts } from "./App";
import type { Icon, Item, ResultBatch } from "./bindings";
import type { Host } from "./host";

function fakeHost() {
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

function item(id: string, extra: Partial<Item> = {}): Item {
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

function batch(seq: number, items: Item[], provider = "apps"): ResultBatch {
  return { session: 1, seq, provider, items, done: false };
}

/** `list[index]`, failing the test instead of returning undefined. */
function at<T>(list: T[], index: number): T {
  const value = list[index];
  if (value === undefined) throw new Error(`no element at ${index}`);
  return value;
}

const selected = () =>
  screen.getAllByRole("option").find((o) => o.getAttribute("aria-selected") === "true");
const titles = () =>
  screen.getAllByRole("option").map((o) => o.querySelector(".title")?.textContent);

describe("App", () => {
  beforeEach(() => {
    mounts.count = 0;
  });

  it("clears the query on show without recreating the component tree", async () => {
    const { host, show } = fakeHost();
    render(() => <App host={host} />);
    const box = screen.getByRole("searchbox");
    fireEvent.input(box, { target: { value: "calc" } });
    expect(box).toHaveValue("calc");

    show();

    await waitFor(() => expect(box).toHaveValue(""));
    expect(screen.getByRole("searchbox")).toBe(box);
    expect(mounts.count).toBe(1);
  });

  it("fills the search field with text sent by the CLI", async () => {
    const { host, show, sendQuery } = fakeHost();
    render(() => <App host={host} />);
    show();
    sendQuery("calc 2+2");
    await waitFor(() => expect(screen.getByRole("searchbox")).toHaveValue("calc 2+2"));
    expect(host.query).toHaveBeenLastCalledWith(1, "calc 2+2");
  });

  it("reports the first paint after a show", async () => {
    const { host, show } = fakeHost();
    render(() => <App host={host} />);
    show();
    await waitFor(() => expect(host.firstPaint).toHaveBeenCalledOnce());
  });

  it("hides on Escape with an empty query and clears a non-empty one first", () => {
    const { host } = fakeHost();
    render(() => <App host={host} />);
    const box = screen.getByRole("searchbox");

    fireEvent.input(box, { target: { value: "x" } });
    fireEvent.keyDown(box, { key: "Escape" });
    expect(box).toHaveValue("");
    expect(host.hide).not.toHaveBeenCalled();

    fireEvent.keyDown(box, { key: "Escape" });
    expect(host.hide).toHaveBeenCalledOnce();
  });

  it("asks for results on every keystroke and on show", () => {
    const { host, show } = fakeHost();
    render(() => <App host={host} />);
    const box = screen.getByRole("searchbox");
    fireEvent.input(box, { target: { value: "f" } });
    fireEvent.input(box, { target: { value: "fi" } });
    show();
    expect(vi.mocked(host.query).mock.calls).toEqual([
      [1, "f"],
      [1, "fi"],
      [1, ""],
    ]);
  });
});

describe("result list", () => {
  it("shows title, subtitle and accessories, and highlights the matched characters", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send(
      batch(0, [
        item("Firefox", { subtitle: "Web Browser", accessories: ["v1"], highlight: [0, 4, 6] }),
      ]),
    );
    const row = screen.getByRole("option");
    expect(row).toHaveTextContent("Firefox");
    expect(row).toHaveTextContent("Web Browser");
    expect(row).toHaveTextContent("v1");
    expect([...row.querySelectorAll("mark")].map((m) => m.textContent)).toEqual(["F", "f", "x"]);
  });

  it("shows an icon from the asset URL and a placeholder when there is none or it fails", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send(
      batch(0, [
        item("with", { icon: { type: "file", path: "/icons/a.png" } }),
        item("named", { icon: { type: "named", name: "firefox" } }),
        item("none"),
      ]),
    );
    const rows = screen.getAllByRole("option");
    expect(at(rows, 0).querySelector("img")).toHaveAttribute("src", "asset:///icons/a.png");
    expect(at(rows, 1).querySelector("img")).toBeNull();
    expect(at(rows, 1).querySelector(".placeholder")).toHaveTextContent("N");
    expect(at(rows, 2).querySelector(".placeholder")).toHaveTextContent("N");

    // A file that cannot be loaded becomes a placeholder, never a broken image.
    const img = at(rows, 0).querySelector("img") as HTMLImageElement;
    fireEvent.error(img);
    expect(at(rows, 0).querySelector("img")).toBeNull();
    expect(at(rows, 0).querySelector(".placeholder")).toHaveTextContent("W");
  });

  it("moves the selection with the arrow keys, Home and End", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send(batch(0, [item("a"), item("b"), item("c")]));
    const box = screen.getByRole("searchbox");
    expect(selected()).toHaveTextContent("a");

    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(selected()).toHaveTextContent("b");
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(selected()).toHaveTextContent("c");
    fireEvent.keyDown(box, { key: "ArrowUp" });
    expect(selected()).toHaveTextContent("b");
    fireEvent.keyDown(box, { key: "Home" });
    expect(selected()).toHaveTextContent("a");
    fireEvent.keyDown(box, { key: "End" });
    expect(selected()).toHaveTextContent("c");
  });

  it("runs the primary action of the selected result on Enter", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send(batch(0, [item("a"), item("b")]));
    const box = screen.getByRole("searchbox");
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(host.runAction).toHaveBeenCalledExactlyOnceWith("b", "open");
  });

  it("does nothing on Enter without results", () => {
    const { host } = fakeHost();
    render(() => <App host={host} />);
    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Enter" });
    expect(host.runAction).not.toHaveBeenCalled();
  });

  it("picks the n-th result with Ctrl+n and Cmd+n", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send(batch(0, [item("a"), item("b"), item("c")]));
    const box = screen.getByRole("searchbox");
    fireEvent.keyDown(box, { key: "3", ctrlKey: true });
    expect(host.runAction).toHaveBeenLastCalledWith("c", "open");
    fireEvent.keyDown(box, { key: "1", metaKey: true });
    expect(host.runAction).toHaveBeenLastCalledWith("a", "open");
    // Out of range and without the modifier: nothing happens.
    fireEvent.keyDown(box, { key: "9", ctrlKey: true });
    fireEvent.keyDown(box, { key: "2" });
    expect(host.runAction).toHaveBeenCalledTimes(2);
  });

  it("merges later batches without moving the selection and replaces the list for a new query", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send(batch(0, [item("a"), item("b"), item("c")]));
    const box = screen.getByRole("searchbox");
    fireEvent.keyDown(box, { key: "End" });
    expect(selected()).toHaveTextContent("c");

    send(batch(1, [item("x")], "calc"));
    expect(titles()).toEqual(["a", "b", "c", "x"]);
    expect(selected()).toHaveTextContent("c");

    send(batch(0, [item("y"), item("z")]));
    expect(titles()).toEqual(["y", "z"]);
    expect(selected()).toHaveTextContent("y");
  });

  it("ignores batches of other sessions", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send({ ...batch(0, [item("a")]), session: 2 });
    expect(screen.queryAllByRole("option")).toHaveLength(0);
  });

  it("sizes the window for the rows shown, at most eight", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    expect(host.resize).toHaveBeenLastCalledWith(0);
    send(batch(0, [item("a"), item("b")]));
    expect(host.resize).toHaveBeenLastCalledWith(2);
    send(
      batch(
        0,
        Array.from({ length: 30 }, (_, i) => item(`i${i}`)),
      ),
    );
    expect(host.resize).toHaveBeenLastCalledWith(8);
  });

  it("renders only the visible rows of a long list and scrolls with the selection", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send(
      batch(
        0,
        Array.from({ length: 1000 }, (_, i) => item(`item${i}`)),
      ),
    );
    expect(screen.getAllByRole("option").length).toBeLessThanOrEqual(10);

    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "End" });
    expect(selected()).toHaveTextContent("item999");
    expect(screen.getAllByRole("option").length).toBeLessThanOrEqual(10);

    // Ctrl+1 now means the first visible row, not the first item.
    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "1", ctrlKey: true });
    expect(host.runAction).toHaveBeenLastCalledWith("item992", "open");
  });

  it("runs a result on click and selects on hover", () => {
    const { host, send } = fakeHost();
    render(() => <App host={host} />);
    send(batch(0, [item("a"), item("b")]));
    const rows = screen.getAllByRole("option");
    fireEvent.mouseMove(at(rows, 1));
    expect(selected()).toHaveTextContent("b");
    fireEvent.click(at(rows, 0));
    expect(host.runAction).toHaveBeenCalledExactlyOnceWith("a", "open");
  });
});
