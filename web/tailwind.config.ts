import type { Config } from "tailwindcss";

/**
 * Pure black, and bright polished silver for whatever is live, active or wanted next. One bright
 * silver section breaks the dark once.
 */
const config: Config = {
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#000000",
        panel: "#080808",
        raise: "#101010",
        line: "#1c1c1c",
        "line-hi": "#2e2e2e",
        paper: "#f2f3f5",
        mute: "#8e939a",
        silver: { DEFAULT: "#e4e7eb", hi: "#ffffff", lo: "#b9c0c9" },
        chrome: { DEFAULT: "#e7eaee", line: "#c3c8cf", mute: "#4d525a" },
      },
      fontFamily: {
        serif: ["var(--font-serif)", "Georgia", "serif"],
        sans: ["var(--font-sans)", "system-ui", "sans-serif"],
        mono: ["var(--font-mono)", "ui-monospace", "monospace"],
      },
      maxWidth: { page: "90rem" },
      keyframes: {
        marquee: { from: { transform: "translateX(0)" }, to: { transform: "translateX(-50%)" } },
        blink: { "50%": { opacity: "0.15" } },
      },
      animation: {
        marquee: "marquee 45s linear infinite",
        blink: "blink 1.2s steps(2) infinite",
      },
    },
  },
  plugins: [],
};

export default config;
