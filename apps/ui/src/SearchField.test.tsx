import { fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { SearchField } from "./SearchField";

function Harness(props: { onEscape?: () => void }) {
  const [query, setQuery] = createSignal("");
  return <SearchField query={query()} onQuery={setQuery} onEscape={props.onEscape ?? (() => {})} />;
}

describe("SearchField", () => {
  it("renders an empty search box before any data is loaded", () => {
    render(() => <Harness />);
    expect(screen.getByRole("searchbox")).toHaveValue("");
  });

  it("keeps what the user types", () => {
    render(() => <Harness />);
    const box = screen.getByRole("searchbox");
    fireEvent.input(box, { target: { value: "firefox" } });
    expect(box).toHaveValue("firefox");
  });

  it("takes the focus when it appears", () => {
    render(() => <Harness />);
    expect(screen.getByRole("searchbox")).toHaveFocus();
  });

  it("reports Escape", () => {
    const onEscape = vi.fn();
    render(() => <Harness onEscape={onEscape} />);
    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Escape" });
    expect(onEscape).toHaveBeenCalledOnce();
  });
});
