import type { Config } from "tailwindcss";

/**
 * Black, paper-white, and one signal orange for whatever is live, active or wanted next. Grey is
 * only ever for secondary text and hairlines. Nothing else.
 */
const config: Config = {
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#050505",
        panel: "#0c0c0c",
        line: "#1f1f1f",
        "line-hi": "#333333",
        paper: "#f2efe9",
        mute: "#8f8f8f",
        signal: { DEFAULT: "#ff5b14", deep: "#1a0d06" },
      },
      fontFamily: {
        serif: ["var(--font-serif)", "Georgia", "serif"],
        sans: ["var(--font-sans)", "system-ui", "sans-serif"],
        cond: ["var(--font-cond)", "Impact", "sans-serif"],
        mono: ["var(--font-mono)", "ui-monospace", "monospace"],
      },
      maxWidth: { page: "90rem" },
      keyframes: {
        marquee: { from: { transform: "translateX(0)" }, to: { transform: "translateX(-50%)" } },
        blink: { "50%": { opacity: "0" } },
      },
      animation: {
        marquee: "marquee 40s linear infinite",
        blink: "blink 1.1s steps(1) infinite",
      },
    },
  },
  plugins: [],
};

export default config;
