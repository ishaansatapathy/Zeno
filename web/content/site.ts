/**
 * Every word on the public site. Zeno is one place to record, stream, edit, host and share video, for
 * creators, teachers and the teams they work with (README.md, "Surfaces"). What the site states as fact
 * is built and measured: a link at stop in about 0.2 s, at most a second lost to a crash, five qualities
 * from 144p to 720p, a key that changes every 30 seconds of video so a revoked link ends a viewing within
 * about half a minute, uploads up to 10 GB and 6 hours, each payment applied exactly once. What is not built
 * yet (the desktop studio, live streaming to other platforms, the editor, live meetings) is always said
 * to be on the way, never as if it were here. Nothing is tied to one country or one currency.
 *
 * A title is [plain, accent]: the accent is its second half, set apart on the page.
 */

export type Title = readonly [string, string];

export const links = {
  start: "/register",
  login: "/login",
} as const;

/** The words on the buttons that lead into the app, wherever they appear. */
export const cta = {
  start: "Get started",
  how: "See how it works",
} as const;

export const nav = [
  { label: "How it works", href: "#bench" },
  { label: "Who it's for", href: "#sides" },
  { label: "Promises", href: "#promises" },
  { label: "FAQ", href: "#faq" },
] as const;

export const hero = {
  label: "Record · Share · Host · Stream",
  title: ["Record it once.", "Share it anywhere."] as Title,
  lines: [
    "Zeno records your screen and camera, has the link ready before you stop, and plays it on any connection.",
    "Share it with a link, sell it as a course, or keep it in your team's workspace.",
  ],
  accentLine: "Live streaming, the editor and meetings are on the way.",
  scroll: "Scroll",
} as const;

export const flip = {
  label: "One place for your video",
  title: ["Record, share,", "teach and stream."] as Title,
  body: "Record in the browser, send a link, let people comment on the exact moment, sell your best ones as a course, and work on it all with your team. Going live from the desktop studio to the platforms you choose is next.",
  primary: cta.start,
  secondary: "See it work",
  ticker: ["Screen and camera", "Link at stop", "Comments on the moment", "Encrypted playback", "Courses and payouts", "Team workspaces", "Desktop studio · soon", "Live meetings · soon"],
} as const;

export const sides = {
  label: "Who it's for",
  title: ["Made for creators.", "Built for teams."] as Title,
  aside: "The same library does both jobs: it gets a recording to the people who need it, quickly and privately, and it turns the ones worth keeping into something you can teach or sell.",
  panels: [
    {
      tag: "Creators and teachers",
      word: "Creators.",
      line: ["Record, share, sell.", "Get paid to your bank."],
      cta: { label: cta.start, href: links.start, external: true },
    },
    {
      tag: "Teams",
      word: "Teams.",
      line: ["Invite by email.", "Owners, admins and members."],
      cta: { label: cta.how, href: "#bench", external: false },
    },
  ],
  foot: { left: "One library", right: "Record · Share · Play · Sell" },
} as const;

export type Step = {
  id: string;
  n: string;
  name: string;
  claim: Title;
  desc: string;
  /** The step's own numbers, as the platform runs them. */
  stats: { k: string; v: string }[];
};

export type FlowNode = {
  id: string;
  /** The step that brings this node onto the canvas (0-based). */
  step: number;
  icon: "upload" | "layers" | "lock" | "card" | "ledger" | "bank";
  meta: string;
  title: string;
  line: string;
  chip: string;
  /** Position and width on a 1000 by 620 canvas; every node is 150 tall. */
  x: number;
  y: number;
  w: number;
};

export type FlowEdge = { from: string; to: string; step: number; badge: string };

/**
 * One recording's way through Zeno, drawn as a flow: what each step does, and the nodes and links it
 * adds to the canvas. The canvas is a simulation; the limits and numbers are the platform's own.
 */
export const bench = {
  label: "One recording, end to end",
  canvas: "Recording canvas",
  canvasMeta: "4 steps · 1 recording",
  steps: [
    {
      id: "record",
      n: "01",
      name: "Record",
      claim: ["Screen and camera,", "kept safe."],
      desc: "In Chrome, Edge or Brave. Screen and camera are kept as separate tracks and written to disk every second, so a crash costs at most a second. Pausing leaves no gap, and a 3-2-1 count starts it.",
      stats: [
        { k: "Tracks", v: "2" },
        { k: "A crash costs", v: "≤ 1 s" },
        { k: "A pause leaves", v: "no gap" },
      ],
    },
    {
      id: "share",
      n: "02",
      name: "Share",
      claim: ["The link exists", "before you stop."],
      desc: "It uploads while you record, so when you stop the link is already on your clipboard. Anyone with it can watch, with no account; add a password or an expiry if you want one.",
      stats: [
        { k: "Link at stop", v: "0.2 s" },
        { k: "Account to watch", v: "none" },
        { k: "Password", v: "optional" },
      ],
    },
    {
      id: "play",
      n: "03",
      name: "Play",
      claim: ["Five qualities,", "keys that change."],
      desc: "Encoded from 144p to 720p and playable seconds after you stop. Every segment is encrypted and the key changes every 30 seconds of video, so revoking a link ends a viewing within about half a minute.",
      stats: [
        { k: "Qualities", v: "5" },
        { k: "Key every", v: "30 s" },
        { k: "Revoke ends it", v: "≈ 30 s" },
      ],
    },
    {
      id: "sell",
      n: "04",
      name: "Sell",
      claim: ["Counted once,", "paid out."],
      desc: "Turn recordings into a course. A payment reported three times is applied once, written twice in a ledger that has to balance, and paid out to your bank.",
      stats: [
        { k: "Applied", v: "1×" },
        { k: "Entries", v: "2" },
        { k: "Off by", v: "0.00" },
      ],
    },
  ] satisfies Step[],
  nodes: [
    { id: "record", step: 0, icon: "upload", meta: "Record · 2 tracks", title: "screen.webm + camera.webm", line: "Written to disk every second", chip: "Paused, no gap", x: 80, y: 235, w: 230 },
    { id: "share", step: 1, icon: "layers", meta: "Share · link", title: "Link on the clipboard", line: "Uploaded while recording", chip: "0.2 s after stop", x: 395, y: 50, w: 240 },
    { id: "play", step: 2, icon: "lock", meta: "Play · AES-128", title: "144p → 720p", line: "A new key every 30 s of video", chip: "Revoke ends it", x: 395, y: 355, w: 240 },
    { id: "checkout", step: 3, icon: "card", meta: "Checkout · payment", title: "Received ×3, applied once", line: "Same payment, counted one time", chip: "Captured", x: 725, y: 25, w: 250 },
    { id: "ledger", step: 3, icon: "ledger", meta: "Ledger · double entry", title: "Sale, fee and share", line: "Every entry written twice", chip: "Off by 0.00", x: 725, y: 230, w: 250 },
    { id: "payout", step: 3, icon: "bank", meta: "Payout · your bank", title: "Ready to pay out", line: "Once the refund window closes", chip: "Ready to pay out", x: 725, y: 435, w: 250 },
  ] satisfies FlowNode[],
  edges: [
    { from: "record", to: "share", step: 1, badge: "1" },
    { from: "share", to: "play", step: 2, badge: "2" },
    { from: "play", to: "checkout", step: 3, badge: "3" },
    { from: "play", to: "ledger", step: 3, badge: "4" },
    { from: "play", to: "payout", step: 3, badge: "5" },
  ] satisfies FlowEdge[],
} as const;

export const pillars = {
  label: "Underneath",
  title: ["Three parts,", "one library."] as Title,
  aside: "What happens under every recording: how it is captured, how it reaches people, and how it is kept private.",
  items: [
    {
      n: "01 / 03",
      word: "Capture.",
      name: "Recording",
      body: "The browser extension records screen, camera and voice as separate tracks and uploads while it records. The desktop studio, for Windows first and then macOS, is on the way.",
    },
    {
      n: "02 / 03",
      word: "Share.",
      name: "Links and comments",
      body: "A link anyone can open, with a password or an expiry if you want one. Viewers leave comments and reactions pinned to the moment they are about.",
    },
    {
      n: "03 / 03",
      word: "Lock.",
      name: "Encrypted playback",
      body: "Every segment is encrypted. The key changes every 30 seconds of video and comes only from a key server that checks, each time, that the viewing is still allowed.",
    },
  ],
} as const;

export const promises = {
  label: "Why Zeno",
  title: ["Five promises,", "kept in code."] as Title,
  aside: "Not features. Rules the backend refuses to break, measured, and tests that break things on purpose to prove it.",
  rows: [
    {
      n: "01",
      tag: "Fast",
      word: "instant.",
      title: "A link before you stop",
      body: "The recording goes up while you make it, so the link is on your clipboard the moment you stop.",
      more: "Measured: about 0.2 seconds from stop to link.",
      big: "0.2",
      unit: "seconds",
    },
    {
      n: "02",
      tag: "Safe",
      word: "kept.",
      title: "A crash does not lose the recording",
      body: "Written to disk every second. A recording cut short is finished later, under the same link.",
      more: "At most the last second is lost.",
      big: "≤ 1",
      unit: "second lost",
    },
    {
      n: "03",
      tag: "Private",
      word: "revocable.",
      title: "Revoking a link means it",
      body: "New viewings stop at once. One already playing ends within about half a minute, because the key changes every 30 seconds of video.",
      more: "Measured in a real browser: 10 to 27 seconds.",
      big: "30",
      unit: "seconds",
    },
    {
      n: "04",
      tag: "Smooth",
      word: "adapt.",
      title: "Quality follows the connection",
      body: "Five qualities from 144p to 720p. A slow connection gets a picture, not a spinner.",
      more: "Audio only, when even 144p is too much.",
      big: "5",
      unit: "qualities",
    },
    {
      n: "05",
      tag: "Exact",
      word: "once.",
      title: "Every payment applied once",
      body: "A payment provider can send the same news three times. Zeno applies it once and writes it twice, in a ledger that has to balance.",
      more: "Tested by killing the database in the middle of a payment.",
      big: "1×",
      unit: "applied",
    },
  ],
} as const;

export const faq = {
  label: "Questions",
  title: ["Asked,", "answered."] as Title,
  aside: "Short, honest answers. If one of them is not good enough for you, better to know now.",
  items: [
    {
      q: "What is Zeno, exactly?",
      a: "One place to record, stream, edit, host and share video. Today: a browser recorder with a link at stop, hosting with share links and comments, courses with checkout and payouts, and team workspaces. On the way: the desktop studio for going live to the platforms you choose, an editor for clips, and live meetings.",
    },
    {
      q: "Do the people I share with need an account?",
      a: "No. Anyone with the link watches in a browser, with nothing to install. A link can have a password or an expiry, and you can revoke it at any time.",
    },
    {
      q: "Which browsers does the recorder work in?",
      a: "Chrome, Edge and Brave on a computer. It records the screen, the camera and the microphone, and keeps your own copy on your computer as well.",
    },
    {
      q: "Can people screen-record what I share?",
      a: "Honestly, yes. Nobody can stop a phone pointed at a screen. What Zeno stops is the easy part: the files are encrypted, the keys change as the video plays, and a revoked link stops giving them out.",
    },
    {
      q: "Can I sell what I record?",
      a: "Yes. Put recordings in a course and set a price. Each payment is applied once, refunds are capped at what was paid, and your share is paid out to your bank once the refund window closes.",
    },
    {
      q: "When can I go live with Zeno?",
      a: "The desktop studio is being built, Windows first and then macOS. It will stream to only the platforms you tick for that stream. Until it is ready, Zeno records, shares and hosts.",
    },
  ],
} as const;

export const finale = {
  title: ["Record something.", "Send the link."] as Title,
  ctas: [
    { tag: "01 · Start", label: cta.start, sub: "Make an account and your library is ready.", href: links.start },
    { tag: "02 · Account", label: "Log in", sub: "Pick up where you left off.", href: links.login },
  ],
} as const;

export const footer = {
  columns: [
    { title: "Product", links: nav },
    {
      title: "Account",
      links: [
        { label: cta.start, href: links.start },
        { label: "Log in", href: links.login },
      ],
    },
  ],
  rights: "Zeno",
  made: "Made for creators and teams everywhere",
  top: "Back to top",
} as const;
