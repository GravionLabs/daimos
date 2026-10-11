/** WCAG 2 contrast helpers, used to keep the OS accent readable and to test the theme tokens. */

export function parseHex(hex: string): [number, number, number] | null {
  const match = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match?.[1]) return null;
  const value = Number.parseInt(match[1], 16);
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}

export function toHex([red, green, blue]: [number, number, number]): string {
  const part = (channel: number) => Math.round(channel).toString(16).padStart(2, "0");
  return `#${part(red)}${part(green)}${part(blue)}`;
}

function luminance([red, green, blue]: [number, number, number]): number {
  const linear = (channel: number) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
}

/** The contrast ratio of two `#rrggbb` colors, from 1 to 21. */
export function contrast(a: string, b: string): number {
  const first = parseHex(a);
  const second = parseHex(b);
  if (!first || !second) throw new Error(`not a #rrggbb color: ${a} ${b}`);
  const [light, dark] = [luminance(first), luminance(second)].sort((x, y) => y - x);
  return ((light ?? 0) + 0.05) / ((dark ?? 0) + 0.05);
}

/**
 * Moves `color` towards black (on a light background) or white (on a dark one) until it has at
 * least `minimum` contrast with every background; returns it unchanged when it already has.
 */
export function ensureContrast(color: string, backgrounds: string[], minimum = 4.5): string {
  const rgb = parseHex(color);
  if (!rgb) return color;
  const ok = (candidate: string) => backgrounds.every((bg) => contrast(candidate, bg) >= minimum);
  if (ok(color)) return color;
  const first = backgrounds[0];
  const towardsWhite = first ? (parseHex(first) ?? [0, 0, 0]).every((c) => c < 128) : false;
  const target = towardsWhite ? 255 : 0;
  for (let step = 1; step <= 20; step++) {
    const amount = step / 20;
    const candidate = toHex(rgb.map((c) => c + (target - c) * amount) as [number, number, number]);
    if (ok(candidate)) return candidate;
  }
  return toHex([target, target, target]);
}
