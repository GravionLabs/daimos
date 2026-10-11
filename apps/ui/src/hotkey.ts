/** Turns a key press into an accelerator such as `Ctrl+Alt+Space`, as the Rust side parses them. */

const NAMED: Record<string, string> = {
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Insert: "Insert",
  Delete: "Delete",
};

/** The name of the key in an accelerator, or null for keys that cannot be bound. */
export function keyName(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter?.[1]) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit?.[1]) return digit[1];
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  return NAMED[code] ?? null;
}

/**
 * The accelerator for a key press, or null while it is not complete: a modifier alone, a key that
 * cannot be bound, or a plain key without a modifier (function keys may stand alone).
 */
export function acceleratorFromEvent(event: KeyboardEvent, mac = false): string | null {
  const key = keyName(event.code);
  if (!key) return null;
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("Ctrl");
  if (event.altKey) modifiers.push(mac ? "Option" : "Alt");
  if (event.shiftKey) modifiers.push("Shift");
  if (event.metaKey) modifiers.push(mac ? "Cmd" : "Super");
  if (modifiers.length === 0 && !/^F\d+$/.test(key)) return null;
  return [...modifiers, key].join("+");
}

export const isMac = () => typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
