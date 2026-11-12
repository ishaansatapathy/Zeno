import type { Config } from "tailwindcss";

/**
 * Graphite, nearly monochrome. Text is off-white; the second half of a headline steps down to
 * gray. One cool accent, frost, marks only what is live or locked. A single light section, paper
 * on bone, breaks the dark once.
 */
const config: Config = {
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#09090b",
        panel: "#0f1012",
        raise: "#16171a",
        line: "#1d1e21",
        "line-hi": "#2b2d31",
        paper: "#ededed",
        mute: "#8a8d94",
        frost: { DEFAULT: "#b4c2d6", deep: "#7f95b6" },
        bone: { DEFAULT: "#ececea", line: "#d6d6d2", mute: "#55585e" },
      },
      fontFamily: {
        sans: ["var(--font-sans)", "system-ui", "sans-serif"],
        mono: ["var(--font-mono)", "ui-monospace", "monospace"],
      },
      maxWidth: { page: "80rem" },
      keyframes: {
        marquee: { from: { transform: "translateX(0)" }, to: { transform: "translateX(-50%)" } },
        blink: { "50%": { opacity: "0.2" } },
      },
      animation: {
        marquee: "marquee 60s linear infinite",
        blink: "blink 1.4s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};

export default config;
