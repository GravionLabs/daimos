import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { contrast } from "./color";
import { SCHEME } from "./theme";

// Vitest runs in apps/ui (`pnpm test`).
const css = readFileSync(resolve("src/styles.css"), "utf8");

/** The `--name: #rrggbb` declarations of the first block that starts with `selector`. */
function tokens(selector: string): Record<string, string> {
  const start = css.indexOf(selector);
  if (start < 0) throw new Error(`no ${selector} in styles.css`);
  const body = css.slice(css.indexOf("{", start) + 1, css.indexOf("}", start));
  const found: Record<string, string> = {};
  for (const match of body.matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})/gi)) {
    if (match[1] && match[2]) found[match[1]] = match[2];
  }
  return found;
}

const schemes = {
  light: tokens(":root {"),
  dark: tokens(':root[data-theme="dark"]'),
};

// What is drawn on what: text on the page, text on the selected row, accent text (the matched
// characters), the error text under a field, and the toast (page color on the text color).
const pairs: [string, string, string][] = [
  ["fg", "bg", "text"],
  ["muted", "bg", "secondary text"],
  ["accent", "bg", "match highlight"],
  ["error", "bg", "error text"],
  ["fg", "selected", "text on the selected row"],
  ["muted", "selected", "subtitle on the selected row"],
  ["accent", "selected", "match highlight on the selected row"],
  ["bg", "fg", "toast"],
];

describe.each(Object.keys(schemes) as (keyof typeof schemes)[])("%s scheme", (name) => {
  const t = schemes[name];

  it.each(pairs)("%s on %s meets WCAG AA (4.5:1): %s", (foreground, background) => {
    const fg = t[foreground];
    const bg = t[background];
    expect(fg, `--${foreground}`).toBeDefined();
    expect(bg, `--${background}`).toBeDefined();
    expect(contrast(fg as string, bg as string)).toBeGreaterThanOrEqual(4.5);
  });

  it("agrees with the backgrounds the accent adjustment uses", () => {
    expect(t.bg).toBe(SCHEME[name].bg);
    expect(t.selected).toBe(SCHEME[name].selected);
  });
});

describe("dark scheme without a forced theme", () => {
  it("uses the same values as the forced dark scheme", () => {
    expect(tokens(':root:not([data-theme="light"])')).toEqual(schemes.dark);
  });
});

describe("reduced motion", () => {
  it("switches off transitions and animations", () => {
    const block = css.slice(css.indexOf("@media (prefers-reduced-motion: reduce)"));
    expect(block).toMatch(/transition:\s*none\s*!important/);
    expect(block).toMatch(/animation:\s*none\s*!important/);
  });

  it("only declares transitions where motion is allowed", () => {
    const outside = css.replace(
      /@media \(prefers-reduced-motion: no-preference\)\s*\{[\s\S]*?\n\}\n/,
      "",
    );
    const withoutReduce = outside.replace(/@media \(prefers-reduced-motion: reduce\)[\s\S]*$/, "");
    expect(withoutReduce).not.toMatch(/transition:/);
  });
});
