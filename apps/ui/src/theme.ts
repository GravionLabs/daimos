import type { ThemeInfo } from "./bindings";
import { ensureContrast } from "./color";

/** The backgrounds of the schemes; `styles.css` must agree (a test checks it). */
export const SCHEME = {
  light: { bg: "#ffffff", selected: "#e3e6ff" },
  dark: { bg: "#1e1e20", selected: "#2c2f4a" },
} as const;

export type Scheme = keyof typeof SCHEME;

/** The scheme to show: the forced one, or the one of the OS. */
export function resolveScheme(info: Pick<ThemeInfo, "setting" | "system">): Scheme {
  if (info.setting === "light" || info.setting === "dark") return info.setting;
  return info.system === "dark" ? "dark" : "light";
}

/**
 * Applies the theme to the document: the scheme as `data-theme` and, where the OS has an accent
 * color, that color, moved just far enough to stay readable on both backgrounds of the scheme.
 */
export function applyTheme(info: ThemeInfo, root: HTMLElement = document.documentElement): Scheme {
  const scheme = resolveScheme(info);
  root.dataset.theme = scheme;
  if (info.accent) {
    const { bg, selected } = SCHEME[scheme];
    root.style.setProperty("--accent", ensureContrast(info.accent, [bg, selected]));
  } else {
    root.style.removeProperty("--accent");
  }
  return scheme;
}
