import type { Config } from "tailwindcss";

/** A role in the design tokens, as a colour that still accepts `/50` style opacity. */
const token = (name: string) => `rgb(var(--${name}) / <alpha-value>)`;

const config: Config = {
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        bg: token("bg"),
        surface: token("surface"),
        "surface-strong": token("surface-strong"),
        border: token("border"),
        "border-strong": token("border-strong"),
        text: token("text"),
        "text-muted": token("text-muted"),
        primary: token("primary"),
        "primary-hover": token("primary-hover"),
        "on-primary": token("on-primary"),
        accent: token("accent"),
        "on-accent": token("on-accent"),
        danger: token("danger"),
        "on-danger": token("on-danger"),
        success: token("success"),
        warning: token("warning"),
        focus: token("focus"),
      },
      borderRadius: { control: "0.75rem", card: "1.25rem" },
      fontFamily: {
        sans: ["var(--font-geist-sans)", "system-ui", "-apple-system", "Segoe UI", "Roboto", "sans-serif"],
        serif: ["var(--font-display)", "Georgia", "Times New Roman", "serif"],
        mono: ["var(--font-geist-mono)", "ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
      letterSpacing: { label: "0.14em" },
      maxWidth: { page: "76rem" },
    },
  },
  plugins: [],
};

export default config;
