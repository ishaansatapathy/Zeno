import type { Config } from "tailwindcss";

/**
 * Colours taken from the character cards: the near-black of the card, the cream of its border,
 * the violet and the yellow of the spray paint. Nothing else.
 */
const config: Config = {
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#0c0a0f",
        coal: "#15121a",
        smoke: "#221d29",
        line: "#2e2836",
        cream: "#f1e6cf",
        dust: "#b9ad97",
        violet: { DEFAULT: "#8b5cf6", hi: "#b9a2ff", deep: "#5b34c9" },
        spray: "#ffc21a",
      },
      fontFamily: {
        sans: ["var(--font-body)", "system-ui", "sans-serif"],
        cond: ["var(--font-cond)", "Arial Narrow", "sans-serif"],
        tag: ["var(--font-tag)", "cursive"],
      },
      maxWidth: { page: "80rem" },
      boxShadow: {
        hard: "5px 5px 0 0 #8b5cf6",
        "hard-sm": "3px 3px 0 0 #8b5cf6",
        "hard-cream": "5px 5px 0 0 #f1e6cf",
      },
      keyframes: {
        marquee: { from: { transform: "translateX(0)" }, to: { transform: "translateX(-50%)" } },
        spin: { to: { transform: "rotate(360deg)" } },
      },
      animation: {
        marquee: "marquee 28s linear infinite",
        "marquee-rev": "marquee 34s linear infinite reverse",
        "spin-slow": "spin 22s linear infinite",
      },
    },
  },
  plugins: [],
};

export default config;
