import { fireEvent, render, screen } from "@solidjs/testing-library";
import { SearchField } from "./SearchField";

describe("SearchField", () => {
  it("renders an empty search box before any data is loaded", () => {
    render(() => <SearchField />);
    expect(screen.getByRole("searchbox")).toHaveValue("");
  });

  it("keeps what the user types", () => {
    render(() => <SearchField />);
    const box = screen.getByRole("searchbox");
    fireEvent.input(box, { target: { value: "firefox" } });
    expect(box).toHaveValue("firefox");
  });
});
