import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { App } from "./App";
import type { Action, Item } from "./bindings";
import { batch, fakeHost, item } from "./test-host";

const open: Action = { id: "open", title: "Open", shortcut: "Enter", kind: "open", payload: null };
const copy: Action = {
  id: "copy",
  title: "Copy Command",
  shortcut: null,
  kind: "copy",
  payload: "firefox --new",
};
const more: Action = {
  id: "more",
  title: "More",
  shortcut: "Ctrl+M",
  kind: "push",
  payload: "details",
};

function app(extra: Partial<Item> = {}) {
  const fake = fakeHost();
  render(() => <App host={fake.host} />);
  fake.send(batch(0, [item("a", { actions: [open, copy, more], ...extra }), item("b")]));
  return { ...fake, box: screen.getByRole("searchbox") };
}

describe("action panel", () => {
  it("opens with Ctrl+K for the selected item and lists its actions with shortcuts", () => {
    const { box } = app();
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.keyDown(box, { key: "k", ctrlKey: true });
    const entries = screen.getAllByRole("menuitem").map((e) => e.textContent);
    expect(entries).toEqual(["OpenEnter", "Copy Command", "MoreCtrl+M"]);
    fireEvent.keyDown(box, { key: "k", metaKey: true });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes with Esc without clearing the query or hiding the window", () => {
    const { host, box } = app();
    fireEvent.input(box, { target: { value: "x" } });
    fireEvent.keyDown(box, { key: "k", ctrlKey: true });
    fireEvent.keyDown(box, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(box).toHaveValue("x");
    expect(host.hide).not.toHaveBeenCalled();
  });

  it("moves with the arrow keys and runs the chosen action on Enter", () => {
    const { host, box } = app();
    fireEvent.keyDown(box, { key: "k", ctrlKey: true });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    const selected = screen.getAllByRole("menuitem").filter((e) => e.dataset.selected === "true");
    expect(selected.map((e) => e.textContent)).toEqual(["MoreCtrl+M"]);
    fireEvent.keyDown(box, { key: "ArrowUp" });
    fireEvent.keyDown(box, { key: "ArrowUp" });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(host.runAction).toHaveBeenCalledExactlyOnceWith("a", "open");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("does not open without a selected item", () => {
    const fake = fakeHost();
    render(() => <App host={fake.host} />);
    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "k", ctrlKey: true });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("runs an action on click", () => {
    const { host, box } = app();
    fireEvent.keyDown(box, { key: "k", ctrlKey: true });
    fireEvent.click(screen.getAllByRole("menuitem")[0] as HTMLElement);
    expect(host.runAction).toHaveBeenCalledExactlyOnceWith("a", "open");
  });
});

describe("copy and the toast", () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
  afterEach(() => vi.useRealTimers());

  it("copies the payload and shows Copied for a moment", async () => {
    const { host, box } = app();
    fireEvent.keyDown(box, { key: "k", ctrlKey: true });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(host.copy).toHaveBeenCalledExactlyOnceWith("firefox --new");
    expect(host.runAction).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByText("Copied")).toBeInTheDocument());
    vi.advanceTimersByTime(2000);
    await waitFor(() => expect(screen.queryByText("Copied")).toBeNull());
  });

  it("says so when the clipboard refuses", async () => {
    const { host, box } = app();
    vi.mocked(host.copy).mockRejectedValueOnce(new Error("denied"));
    fireEvent.keyDown(box, { key: "k", ctrlKey: true });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(screen.getByText("Could not copy")).toBeInTheDocument());
  });
});

describe("detail pane", () => {
  it("toggles with Tab and shows key/value metadata", () => {
    const { box } = app({
      detail: {
        type: "metadata",
        entries: [
          ["Command", "firefox"],
          ["Kind", "Browser"],
        ],
      },
    });
    expect(screen.queryByLabelText("Detail")).toBeNull();
    fireEvent.keyDown(box, { key: "Tab" });
    const pane = screen.getByLabelText("Detail");
    expect(pane).toHaveTextContent("Commandfirefox");
    expect(pane).toHaveTextContent("KindBrowser");
    fireEvent.keyDown(box, { key: "Tab" });
    expect(screen.queryByLabelText("Detail")).toBeNull();
  });

  it("renders Markdown and follows the selection", () => {
    const fake = fakeHost();
    render(() => <App host={fake.host} />);
    fake.send(
      batch(0, [
        item("a", {
          detail: { type: "markdown", text: "# Hello\n\nSome **bold** text\n\n- one\n- two" },
        }),
        item("b"),
      ]),
    );
    const box = screen.getByRole("searchbox");
    fireEvent.keyDown(box, { key: "Tab" });
    const pane = screen.getByLabelText("Detail");
    expect(pane.querySelector(".md-heading")).toHaveTextContent("Hello");
    expect(pane.querySelector("strong")).toHaveTextContent("bold");
    expect(pane.querySelectorAll("li")).toHaveLength(2);
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(screen.getByLabelText("Detail")).toHaveTextContent("No detail");
  });

  it("closes with Esc before the query is cleared", () => {
    const { host, box } = app();
    fireEvent.input(box, { target: { value: "x" } });
    fireEvent.keyDown(box, { key: "Tab" });
    fireEvent.keyDown(box, { key: "Escape" });
    expect(screen.queryByLabelText("Detail")).toBeNull();
    expect(box).toHaveValue("x");
    expect(host.hide).not.toHaveBeenCalled();
  });
});

describe("navigation stack", () => {
  type LoadedView = Awaited<ReturnType<ReturnType<typeof fakeHost>["host"]["loadView"]>>;

  /** Opens the panel, runs "More" (a push action) and answers the loads with `views` in order. */
  function push(...views: LoadedView[]) {
    const { host, box, ...rest } = app();
    for (const view of views) vi.mocked(host.loadView).mockResolvedValueOnce(view);
    fireEvent.keyDown(box, { key: "k", ctrlKey: true });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter" });
    return { host, box, ...rest };
  }

  it("pushes a list view and Esc goes back to the search results", async () => {
    const { host, box } = push({ type: "list", items: [item("x1"), item("x2")] });
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    expect(host.loadView).toHaveBeenCalledExactlyOnceWith("apps", "details");
    expect(screen.getByText("More")).toBeInTheDocument(); // the breadcrumb
    expect(screen.queryByRole("searchbox")).toBeNull();
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("x1");

    fireEvent.keyDown(screen.getByRole("main"), { key: "Escape" });
    await waitFor(() => expect(screen.getByRole("searchbox")).toBeInTheDocument());
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("a");
    expect(host.hide).not.toHaveBeenCalled();
    expect(box).toBeTruthy();
  });

  it("the keyboard works inside a pushed list", async () => {
    const { host } = push({ type: "list", items: [item("x1"), item("x2")] });
    const main = await screen.findByRole("main");
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    fireEvent.keyDown(main, { key: "ArrowDown" });
    fireEvent.keyDown(main, { key: "Enter" });
    expect(host.runAction).toHaveBeenCalledExactlyOnceWith("x2", "open");
  });

  it("pushes a detail view and goes back with the back button", async () => {
    push({ type: "detail", detail: { type: "markdown", text: "Just text" } });
    await waitFor(() => expect(screen.getByLabelText("Detail")).toHaveTextContent("Just text"));
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await waitFor(() => expect(screen.getByRole("searchbox")).toBeInTheDocument());
  });

  it("follows a push view to the next view", async () => {
    const { host } = push({ type: "push", viewId: "next" });
    vi.mocked(host.loadView).mockResolvedValueOnce({
      type: "detail",
      detail: { type: "markdown", text: "Next one" },
    });
    await waitFor(() => expect(screen.getByLabelText("Detail")).toHaveTextContent("Next one"));
    expect(vi.mocked(host.loadView).mock.calls.map((c) => c[1])).toEqual(["details", "next"]);
  });

  it("tells the user when a view cannot be loaded", async () => {
    const { host, box } = app();
    fireEvent.keyDown(box, { key: "k", ctrlKey: true });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(screen.getByText("no view")).toBeInTheDocument());
    expect(host.loadView).toHaveBeenCalled();
    expect(screen.getByRole("searchbox")).toBeInTheDocument();
  });

  it("a form view validates, submits to the provider and returns", async () => {
    const { host } = push({
      type: "form",
      fields: [{ id: "name", label: "Name", required: true }],
    });
    const field = await screen.findByLabelText("Name");
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Required");
    expect(host.submitForm).not.toHaveBeenCalled();

    fireEvent.input(field, { target: { value: "Ada" } });
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(host.submitForm).toHaveBeenCalledExactlyOnceWith("apps", "details", { name: "Ada" });
    await waitFor(() => expect(screen.getByRole("searchbox")).toBeInTheDocument());
    expect(screen.getByText("Done")).toBeInTheDocument();
  });

  it("a form keeps its place and shows the error when the provider refuses it", async () => {
    const { host } = push({
      type: "form",
      fields: [{ id: "name", label: "Name", required: true }],
    });
    vi.mocked(host.submitForm).mockRejectedValueOnce(new Error("name taken"));
    fireEvent.input(await screen.findByLabelText("Name"), { target: { value: "Ada" } });
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    await waitFor(() => expect(screen.getByText("name taken")).toBeInTheDocument());
    expect(screen.getByLabelText("Name")).toBeInTheDocument();
  });
});

describe("paste", () => {
  const paste: Action = {
    id: "paste",
    title: "Paste",
    shortcut: "Enter",
    kind: "paste",
    payload: "hello",
  };

  function pasting() {
    const fake = fakeHost();
    render(() => <App host={fake.host} />);
    fake.send(batch(0, [item("a", { actions: [paste] })]));
    return { ...fake, box: screen.getByRole("searchbox") };
  }

  it("copies the text, then asks the shell to paste it", async () => {
    const { host, box } = pasting();
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(host.paste).toHaveBeenCalledOnce());
    expect(host.copy).toHaveBeenCalledExactlyOnceWith("hello");
    expect(host.runAction).not.toHaveBeenCalled();
    expect(screen.queryByText(/^Copied/)).toBeNull();
  });

  it("says why when it could only copy", async () => {
    const { host, box } = pasting();
    vi.mocked(host.paste).mockResolvedValueOnce({ pasted: false, reason: "install wtype" });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(await screen.findByText("Copied. install wtype")).toBeInTheDocument();
  });
});
