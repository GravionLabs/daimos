import type { ThemeInfo } from "./bindings";
import { applyTheme, resolveScheme } from "./theme";

const info = (extra: Partial<ThemeInfo> = {}): ThemeInfo => ({
  setting: "system",
  system: "light",
  accent: null,
  ...extra,
});

describe("resolveScheme", () => {
  it("follows the OS while the setting is system", () => {
    expect(resolveScheme(info({ system: "dark" }))).toBe("dark");
    expect(resolveScheme(info({ system: "light" }))).toBe("light");
  });

  it("uses the forced scheme whatever the OS says", () => {
    expect(resolveScheme(info({ setting: "dark", system: "light" }))).toBe("dark");
    expect(resolveScheme(info({ setting: "light", system: "dark" }))).toBe("light");
  });
});

describe("applyTheme", () => {
  const root = () => document.documentElement;
  afterEach(() => {
    delete root().dataset.theme;
    root().style.removeProperty("--accent");
  });

  it("sets the scheme on the document", () => {
    applyTheme(info({ system: "dark" }));
    expect(root().dataset.theme).toBe("dark");
    applyTheme(info({ setting: "light", system: "dark" }));
    expect(root().dataset.theme).toBe("light");
  });

  it("takes the OS accent and keeps it readable, or falls back to the CSS accent", () => {
    applyTheme(info({ accent: "#0a84ff" }));
    expect(root().style.getPropertyValue("--accent")).toMatch(/^#[0-9a-f]{6}$/);
    applyTheme(info({ accent: null }));
    expect(root().style.getPropertyValue("--accent")).toBe("");
  });

  it("moves a pale accent so it stays readable on the scheme's backgrounds", () => {
    applyTheme(info({ accent: "#ffd60a" }));
    const light = root().style.getPropertyValue("--accent");
    expect(light).not.toBe("#ffd60a");
  });
});
