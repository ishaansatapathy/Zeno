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
        danger: token("danger"),
        "on-danger": token("on-danger"),
        success: token("success"),
        warning: token("warning"),
        focus: token("focus"),
      },
      borderRadius: { control: "0.625rem", card: "1rem" },
      fontFamily: {
        sans: ["system-ui", "-apple-system", "Segoe UI", "Roboto", "Noto Sans", "Noto Sans Devanagari", "sans-serif"],
      },
    },
  },
  plugins: [],
};

export default config;
