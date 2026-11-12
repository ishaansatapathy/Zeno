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
  "danger",
  "onDanger",
  "success",
  "warning",
  "focus",
] as const;

export type Role = (typeof roles)[number];
export type Palette = Record<Role, string>;

export const light: Palette = {
  bg: "#ffffff",
  surface: "#f6f7fb",
  surfaceStrong: "#eceef6",
  border: "#d9dce8",
  borderStrong: "#6b7088",
  text: "#14162a",
  textMuted: "#4a4f6a",
  primary: "#4338ca",
  primaryHover: "#3730a3",
  onPrimary: "#ffffff",
  danger: "#b91c1c",
  onDanger: "#ffffff",
  success: "#15803d",
  warning: "#b45309",
  focus: "#4f46e5",
};

export const dark: Palette = {
  bg: "#0d0f1a",
  surface: "#151828",
  surfaceStrong: "#1e2236",
  border: "#2a2f47",
  borderStrong: "#8a91b0",
  text: "#eceefb",
  textMuted: "#a6abc8",
  primary: "#818cf8",
  primaryHover: "#a5b4fc",
  onPrimary: "#0d0f1a",
  danger: "#f87171",
  onDanger: "#0d0f1a",
  success: "#4ade80",
  warning: "#fbbf24",
  focus: "#a5b4fc",
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
