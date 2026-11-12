import { describe, expect, it } from "vitest";

import { channels, contrast, cssName, dark, light, roles, themeCss, type Palette } from "./tokens";

/** Pairs that are put on top of one another, and the contrast each must reach (WCAG 2.1 AA). */
const textPairs: [keyof Palette, keyof Palette][] = [
  ["text", "bg"],
  ["text", "surface"],
  ["text", "surfaceStrong"],
  ["textMuted", "bg"],
  ["textMuted", "surface"],
  ["textMuted", "surfaceStrong"],
  ["onPrimary", "primary"],
  ["onPrimary", "primaryHover"],
  ["onDanger", "danger"],
  ["primary", "bg"],
  ["primary", "surface"],
  ["danger", "bg"],
  ["danger", "surface"],
  ["success", "bg"],
  ["success", "surface"],
  ["warning", "bg"],
  ["warning", "surface"],
];
/** Borders and focus rings that identify a control must reach 3:1 against what is around them. */
const uiPairs: [keyof Palette, keyof Palette][] = [
  ["borderStrong", "bg"],
  ["borderStrong", "surface"],
  ["focus", "bg"],
  ["focus", "surface"],
  ["primary", "bg"],
];

describe.each([
  ["light", light],
  ["dark", dark],
])("the %s palette", (_, palette) => {
  it.each(textPairs)("%s text on %s is readable (4.5:1)", (fg, bg) => {
    expect(contrast(palette[fg], palette[bg])).toBeGreaterThanOrEqual(4.5);
  });

  it.each(uiPairs)("%s against %s is visible (3:1)", (fg, bg) => {
    expect(contrast(palette[fg], palette[bg])).toBeGreaterThanOrEqual(3);
  });

  it("names every role, in a form the CSS can use", () => {
    for (const role of roles) expect(palette[role]).toMatch(/^#[0-9a-f]{6}$/i);
  });
});

describe("the generated CSS", () => {
  const css = themeCss();

  it("defines every role in light and in dark, twice for dark (system and forced)", () => {
    for (const role of roles) {
      const name = cssName(role);
      expect(css.split(`${name}:${channels(light[role])};`).length - 1).toBe(1);
      expect(css.split(`${name}:${channels(dark[role])};`).length - 1).toBe(2);
    }
  });

  it("follows the system unless told otherwise, and can be forced", () => {
    expect(css).toContain('@media (prefers-color-scheme: dark){:root:not([data-theme="light"])');
    expect(css).toContain(':root[data-theme="dark"]');
    expect(css).toContain("color-scheme:light;");
    expect(css).toContain("color-scheme:dark;");
  });
});

describe("the helpers", () => {
  it("names variables after roles", () => {
    expect(cssName("textMuted")).toBe("--text-muted");
    expect(cssName("bg")).toBe("--bg");
    expect(cssName("onPrimary")).toBe("--on-primary");
  });

  it("splits a colour into channels, and refuses anything else", () => {
    expect(channels("#4338ca")).toBe("67 56 202");
    expect(channels("#FFFFFF")).toBe("255 255 255");
    expect(() => channels("#fff")).toThrow();
    expect(() => channels("red")).toThrow();
  });

  it("measures contrast the way WCAG does", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("#ffffff", "#ffffff")).toBeCloseTo(1, 5);
    expect(contrast("#777777", "#ffffff")).toBeCloseTo(4.48, 1);
    // Symmetric.
    expect(contrast("#4338ca", "#ffffff")).toBe(contrast("#ffffff", "#4338ca"));
  });
});
