import { fireEvent, render, screen } from "@solidjs/testing-library";
import { FormView, validate } from "./FormView";

const fields = [
  { id: "name", label: "Name", required: true },
  { id: "note", label: "Note", required: false },
];

describe("validate", () => {
  it("flags required fields that are empty or only spaces", () => {
    expect(validate(fields, {})).toEqual({ name: "Required" });
    expect(validate(fields, { name: "  " })).toEqual({ name: "Required" });
    expect(validate(fields, { name: "Ada" })).toEqual({});
  });
});

describe("FormView", () => {
  it("shows the problem and does not submit while a required field is empty", () => {
    const onSubmit = vi.fn();
    render(() => <FormView title="New" fields={fields} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Required");
    expect(screen.getByLabelText("Name")).toHaveAttribute("aria-invalid", "true");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("submits the values once the form is valid and clears the error", () => {
    const onSubmit = vi.fn();
    render(() => <FormView title="New" fields={fields} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    fireEvent.input(screen.getByLabelText("Name"), { target: { value: "Ada" } });
    fireEvent.input(screen.getByLabelText("Note"), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(onSubmit).toHaveBeenCalledExactlyOnceWith({ name: "Ada", note: "hi" });
  });
});
