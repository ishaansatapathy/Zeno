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
 * Paper and ink. A warm cream page, near-black ink for text and for the main button, one saffron
 * accent used as a fill (never as text: it is too light to read on cream), and a deep green that
 * means "paid". No purple, no blue-violet gradients.
 */
export const light: Palette = {
  bg: "#f4efe6",
  surface: "#fbf8f2",
  surfaceStrong: "#eae2d3",
  border: "#dcd3c4",
  borderStrong: "#7d7363",
  text: "#17140f",
  textMuted: "#5c5449",
  primary: "#17140f",
  primaryHover: "#2e2a23",
  onPrimary: "#f4efe6",
  accent: "#e8a33d",
  onAccent: "#17140f",
  danger: "#a1281c",
  onDanger: "#fbf8f2",
  success: "#2d6a3a",
  warning: "#865500",
  focus: "#a3560a",
};

/** The same roles at night: warm charcoal, not blue-black, so the brand still feels like paper. */
export const dark: Palette = {
  bg: "#13110d",
  surface: "#1b1813",
  surfaceStrong: "#26221b",
  border: "#36312a",
  borderStrong: "#8d8474",
  text: "#f2ece1",
  textMuted: "#b4aa99",
  primary: "#f2ece1",
  primaryHover: "#e2dacb",
  onPrimary: "#13110d",
  accent: "#e8a33d",
  onAccent: "#13110d",
  danger: "#f0897c",
  onDanger: "#13110d",
  success: "#7fc48d",
  warning: "#e3b160",
  focus: "#e8a33d",
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
    // A part of the page that is designed for paper only (the public site) asks for it by name.
    `[data-palette="light"]{${declarations(light, "light")}}`,
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
