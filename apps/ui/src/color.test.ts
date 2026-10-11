import { contrast, ensureContrast, parseHex, toHex } from "./color";

describe("contrast", () => {
  it("matches the WCAG reference values", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 1);
    expect(contrast("#ffffff", "#ffffff")).toBeCloseTo(1, 5);
    // #767676 on white is the classic AA threshold (4.54:1).
    expect(contrast("#767676", "#ffffff")).toBeCloseTo(4.54, 1);
    expect(contrast("#ffffff", "#767676")).toBeCloseTo(contrast("#767676", "#ffffff"), 8);
  });

  it("rejects what is not #rrggbb", () => {
    expect(() => contrast("red", "#fff")).toThrow();
    expect(parseHex("#12345")).toBeNull();
    expect(parseHex("#ABCDEF")).toEqual([171, 205, 239]);
  });

  it("round-trips hex", () => {
    expect(toHex([0, 120, 215])).toBe("#0078d7");
    expect(toHex(parseHex("#0a0b0c") ?? [0, 0, 0])).toBe("#0a0b0c");
  });
});

describe("ensureContrast", () => {
  it("keeps a readable color unchanged", () => {
    expect(ensureContrast("#3f4fd8", ["#ffffff"])).toBe("#3f4fd8");
  });

  it("darkens a pale color on a light background until it reaches AA", () => {
    const fixed = ensureContrast("#ffd60a", ["#ffffff", "#e3e6ff"]);
    expect(fixed).not.toBe("#ffd60a");
    expect(contrast(fixed, "#ffffff")).toBeGreaterThanOrEqual(4.5);
    expect(contrast(fixed, "#e3e6ff")).toBeGreaterThanOrEqual(4.5);
  });

  it("lightens a dark color on a dark background", () => {
    const fixed = ensureContrast("#0a2a8a", ["#1e1e20", "#2c2f4a"]);
    expect(contrast(fixed, "#1e1e20")).toBeGreaterThanOrEqual(4.5);
    expect(contrast(fixed, "#2c2f4a")).toBeGreaterThanOrEqual(4.5);
  });

  it("leaves an unparsable color alone", () => {
    expect(ensureContrast("rebeccapurple", ["#ffffff"])).toBe("rebeccapurple");
  });
});
