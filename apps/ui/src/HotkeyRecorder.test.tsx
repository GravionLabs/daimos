import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { HotkeyRecorder } from "./HotkeyRecorder";

function setup(result: string | null = null) {
  const onRecord = vi.fn(() => Promise.resolve(result));
  render(() => <HotkeyRecorder binding="Alt+Space" onRecord={onRecord} />);
  return { onRecord, button: screen.getByRole("button", { name: "Hotkey" }) };
}

describe("HotkeyRecorder", () => {
  it("shows the current binding", () => {
    expect(setup().button).toHaveTextContent("Alt+Space");
  });

  it("records the next complete shortcut and reports it", async () => {
    const { onRecord, button } = setup();
    fireEvent.click(button);
    expect(button).toHaveTextContent("Press the new shortcut");
    // A modifier alone and a plain key do not finish the recording.
    fireEvent.keyDown(button, { code: "ControlLeft", key: "Control", ctrlKey: true });
    fireEvent.keyDown(button, { code: "KeyA", key: "a" });
    expect(onRecord).not.toHaveBeenCalled();
    fireEvent.keyDown(button, { code: "KeyK", key: "k", ctrlKey: true, altKey: true });
    await waitFor(() => expect(onRecord).toHaveBeenCalledExactlyOnceWith("Ctrl+Alt+K"));
    expect(button).toHaveTextContent("Alt+Space");
  });

  it("shows the answer of the system when the shortcut is refused", async () => {
    const { button } = setup("Ctrl+Alt+K is already used by another program");
    fireEvent.click(button);
    fireEvent.keyDown(button, { code: "KeyK", key: "k", ctrlKey: true, altKey: true });
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("already used"));
    // Starting again clears the message.
    fireEvent.click(button);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("Esc cancels the recording without choosing anything", () => {
    const { onRecord, button } = setup();
    fireEvent.click(button);
    fireEvent.keyDown(button, { code: "Escape", key: "Escape" });
    expect(button).toHaveTextContent("Alt+Space");
    expect(onRecord).not.toHaveBeenCalled();
  });

  it("does not record while not recording", () => {
    const { onRecord, button } = setup();
    fireEvent.keyDown(button, { code: "KeyK", key: "k", ctrlKey: true });
    expect(onRecord).not.toHaveBeenCalled();
  });
});
