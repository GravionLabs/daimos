import { acceleratorFromEvent, keyName } from "./hotkey";

function press(code: string, modifiers: Partial<KeyboardEventInit> = {}) {
  return new KeyboardEvent("keydown", { code, ...modifiers });
}

describe("keyName", () => {
  it("names letters, digits, function keys and a few named keys", () => {
    expect(keyName("KeyK")).toBe("K");
    expect(keyName("Digit7")).toBe("7");
    expect(keyName("F12")).toBe("F12");
    expect(keyName("F24")).toBe("F24");
    expect(keyName("Space")).toBe("Space");
    expect(keyName("ArrowLeft")).toBe("Left");
  });

  it("refuses keys that cannot be bound", () => {
    for (const code of [
      "ShiftLeft",
      "ControlRight",
      "MetaLeft",
      "Escape",
      "Comma",
      "F25",
      "F0",
      "Numpad1",
      "",
    ]) {
      expect(keyName(code), code).toBeNull();
    }
  });
});

describe("acceleratorFromEvent", () => {
  it("joins the modifiers in a fixed order with the key", () => {
    expect(acceleratorFromEvent(press("Space", { altKey: true }))).toBe("Alt+Space");
    expect(
      acceleratorFromEvent(press("KeyK", { shiftKey: true, ctrlKey: true, altKey: true })),
    ).toBe("Ctrl+Alt+Shift+K");
    expect(acceleratorFromEvent(press("KeyP", { metaKey: true }))).toBe("Super+P");
  });

  it("uses the macOS names for Option and Command", () => {
    expect(acceleratorFromEvent(press("Space", { altKey: true }), true)).toBe("Option+Space");
    expect(acceleratorFromEvent(press("KeyP", { metaKey: true }), true)).toBe("Cmd+P");
  });

  it("waits while only a modifier is down or the key has no modifier", () => {
    expect(acceleratorFromEvent(press("ControlLeft", { ctrlKey: true }))).toBeNull();
    expect(acceleratorFromEvent(press("KeyA"))).toBeNull();
    expect(acceleratorFromEvent(press("Space"))).toBeNull();
  });

  it("lets a function key stand alone", () => {
    expect(acceleratorFromEvent(press("F9"))).toBe("F9");
  });
});
