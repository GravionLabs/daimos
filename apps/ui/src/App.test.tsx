import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { App, mounts } from "./App";
import type { Host } from "./host";

function fakeHost() {
  let shown: (() => void) | undefined;
  let query: ((query: string) => void) | undefined;
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
    firstPaint: vi.fn(),
    hide: vi.fn(),
    resize: vi.fn(),
  };
  return { host, show: () => shown?.(), sendQuery: (text: string) => query?.(text) };
}

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
});
