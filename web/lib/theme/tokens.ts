/**
 * The design tokens: one source for the colours the app uses, in both themes.
 *
 * Components never name a colour; they name a role (`text-muted`, `bg-primary`). Tailwind maps
 * each role to a CSS variable, and `themeCss()` writes the variables, so a change here is the only
 * change a re-skin needs. `tokens.test.ts` holds every pair of colours that is ever put on top of
 * one another to WCAG AA.
 */

export const roles = [
  "bg",
  "surface",
  "surfaceStrong",
  "border",
  "borderStrong",
  "text",
  "textMuted",
  "primary",
  "primaryHover",
  "onPrimary",
  "accent",
  "onAccent",
  "danger",
  "onDanger",
  "success",
  "warning",
  "focus",
] as const;

export type Role = (typeof roles)[number];
export type Palette = Record<Role, string>;

/**
 * Black and silver. Light is the same idea in daylight: near-white paper, near-black ink. The
 * accent is silver itself, used as a fill or a sheen (never as small text on a light page); the
 * only other colours are functional: paid, live, warning.
 */
export const light: Palette = {
  bg: "#fafafa",
  surface: "#ffffff",
  surfaceStrong: "#f0f0f2",
  border: "#e4e4e7",
  borderStrong: "#71717a",
  text: "#0a0a0b",
  textMuted: "#52525b",
  primary: "#0a0a0b",
  primaryHover: "#27272a",
  onPrimary: "#fafafa",
  accent: "#c8c8ce",
  onAccent: "#0a0a0b",
  danger: "#b42318",
  onDanger: "#ffffff",
  success: "#1f7a3d",
  warning: "#8a5300",
  focus: "#3f3f46",
};

/** Night: true black, steel greys, silver-white text and a silver button. */
export const dark: Palette = {
  bg: "#050505",
  surface: "#0e0e10",
  surfaceStrong: "#17171a",
  border: "#232327",
  borderStrong: "#6e6e77",
  text: "#f4f4f5",
  textMuted: "#a1a1aa",
  primary: "#f4f4f5",
  primaryHover: "#d4d4d8",
  onPrimary: "#050505",
  accent: "#d4d4d8",
  onAccent: "#050505",
  danger: "#f97066",
  onDanger: "#050505",
  success: "#6ee7a0",
  warning: "#f5c46b",
  focus: "#e4e4e7",
};

/** `--text-muted`, from `textMuted`. */
export const cssName = (role: Role) => `--${role.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;

/** `#4338ca` to `67 56 202`, the form Tailwind needs to apply opacity to a variable. */
export function channels(hex: string): string {
  const match = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!match) throw new Error(`not a #rrggbb colour: ${hex}`);
  const value = parseInt(match[1], 16);
  return `${(value >> 16) & 255} ${(value >> 8) & 255} ${value & 255}`;
}

const declarations = (palette: Palette, scheme: "light" | "dark") =>
  `${roles.map((role) => `${cssName(role)}:${channels(palette[role])};`).join("")}color-scheme:${scheme};`;

/**
 * The CSS that defines every variable. Light by default; dark when the system says so, unless the
 * page was told `data-theme="light"`; and `data-theme="dark"` forces it.
 */
export function themeCss(): string {
  return [
    `:root{${declarations(light, "light")}}`,
    `@media (prefers-color-scheme: dark){:root:not([data-theme="light"]){${declarations(dark, "dark")}}}`,
    `:root[data-theme="dark"]{${declarations(dark, "dark")}}`,
    // A part of the page designed for one palette only asks for it by name: the public site is
    // always black and silver, whatever the visitor's theme.
    `[data-palette="light"]{${declarations(light, "light")}}`,
    `[data-palette="dark"]{${declarations(dark, "dark")}}`,
  ].join("");
}

// ---- contrast ----------------------------------------------------------------------------------

function luminance(hex: string): number {
  const [r, g, b] = channels(hex)
    .split(" ")
    .map((part) => {
      const c = Number(part) / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio of two colours, from 1 (identical) to 21 (black on white). */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}
