/**
 * Every word on the public site. Claims are things the backend does today, with the numbers it
 * uses by default: five qualities from 144p to 720p, two screens at a time, uploads up to 10 GB
 * and 6 hours, originals deleted 7 days after encoding, refunds capped at what was paid, each
 * payment applied exactly once. Nothing is tied to one country or one currency.
 *
 * A title is [plain, accent]: the accent is set in the serif's italic, in signal orange.
 */

export type Title = readonly [string, string];

export const links = {
  start: "/sign-up",
  login: "/sign-in",
} as const;

export const nav = [
  { label: "How it works", href: "#bench" },
  { label: "For teachers", href: "#sides" },
  { label: "Promises", href: "#promises" },
  { label: "FAQ", href: "#faq" },
] as const;

export const hero = {
  title: ["Teach what you know.", "Keep what you earn."] as Title,
  lines: [
    "For teachers, Zeno turns a video into a course that is hard to steal.",
    "For students, it plays on any connection.",
  ],
  accentLine: "Both on one ledger that has to balance.",
  scroll: "Scroll",
} as const;

export const flip = {
  label: "The school behind your lessons",
  title: ["Every lesson locked.", "Every sale counted."] as Title,
  body: "Upload once. Zeno encodes it, locks it segment by segment, sells it, and pays you out, the same way every time.",
  primary: "Start teaching",
  secondary: "See it work",
  ticker: ["Video courses", "Live classes", "Five qualities", "Applied once", "Refunds capped", "Paid to your bank"],
} as const;

export const sides = {
  label: "Two sides, one school",
  title: ["Made for teachers.", "Fair to students."] as Title,
  aside: "The same platform does two jobs: it protects and pays the person teaching, and it plays smoothly and refunds honestly for the person learning.",
  panels: [
    {
      tag: "For teachers",
      word: "Teachers.",
      line: ["Upload once. Set a price.", "Get paid to your bank."],
      cta: { label: "Start teaching", href: links.start, external: true },
    },
    {
      tag: "For students",
      word: "Students.",
      line: ["Any phone, any connection.", "Pick up where you stopped."],
      cta: { label: "See how it plays", href: "#bench", external: false },
    },
  ],
  foot: { left: "One platform", right: "Upload · Encode · Lock · Pay out" },
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
 * One lesson's way through Zeno, drawn as a flow: what each step does, and the nodes and links it
 * adds to the canvas. The canvas is a simulation; the limits and numbers are the platform's own.
 */
export const bench = {
  label: "One lesson, end to end",
  canvas: "Lesson canvas",
  canvasMeta: "4 steps · 1 lesson",
  steps: [
    {
      id: "upload",
      n: "01",
      name: "Upload",
      claim: ["Up from the browser,", "in parts."],
      desc: "A dropped connection costs one part, not the whole file, and the upload picks up where it stopped. Up to 10 GB, up to 6 hours.",
      stats: [
        { k: "Up to", v: "10 GB" },
        { k: "Length", v: "6 h" },
        { k: "A drop costs", v: "1 part" },
      ],
    },
    {
      id: "encode",
      n: "02",
      name: "Encode",
      claim: ["One file,", "five qualities."],
      desc: "144p to 720p, never upscaled, with audio alone to fall back on, so a slow connection gets a picture instead of a spinner.",
      stats: [
        { k: "Qualities", v: "5" },
        { k: "Lowest", v: "144p" },
        { k: "Highest", v: "720p" },
      ],
    },
    {
      id: "lock",
      n: "03",
      name: "Lock",
      claim: ["Every segment,", "sealed."],
      desc: "Each few seconds is encrypted on its own. Keys come only from the key server, and only for a student who paid.",
      stats: [
        { k: "Cipher", v: "AES-128" },
        { k: "Screens", v: "2" },
        { k: "Keys for", v: "paid" },
      ],
    },
    {
      id: "paid",
      n: "04",
      name: "Get paid",
      claim: ["Counted once,", "paid out."],
      desc: "A payment reported three times is applied once, written twice in a ledger that has to balance, and paid out to your bank.",
      stats: [
        { k: "Applied", v: "1×" },
        { k: "Entries", v: "2" },
        { k: "Off by", v: "0.00" },
      ],
    },
  ] satisfies Step[],
  nodes: [
    { id: "upload", step: 0, icon: "upload", meta: "Upload · resumable", title: "lesson-07.mp4", line: "1.24 GB in 24 parts", chip: "Part 13 retried", x: 80, y: 235, w: 230 },
    { id: "encode", step: 1, icon: "layers", meta: "Encode · 5 qualities", title: "144p → 720p", line: "Never upscaled · audio fallback", chip: "Ladder ready", x: 395, y: 50, w: 240 },
    { id: "lock", step: 2, icon: "lock", meta: "Lock · AES-128", title: "24 segments sealed", line: "Key wrapped, never stored plain", chip: "Keys for paid students", x: 395, y: 355, w: 240 },
    { id: "checkout", step: 3, icon: "card", meta: "Checkout · payment", title: "Received ×3, applied once", line: "Same payment, counted one time", chip: "Captured", x: 725, y: 25, w: 250 },
    { id: "ledger", step: 3, icon: "ledger", meta: "Ledger · double entry", title: "Sale, fee and share", line: "Every entry written twice", chip: "Off by 0.00", x: 725, y: 230, w: 250 },
    { id: "payout", step: 3, icon: "bank", meta: "Payout · your bank", title: "Ready to pay out", line: "Once the refund window closes", chip: "Ready to pay out", x: 725, y: 435, w: 250 },
  ] satisfies FlowNode[],
  edges: [
    { from: "upload", to: "encode", step: 1, badge: "1" },
    { from: "encode", to: "lock", step: 2, badge: "2" },
    { from: "lock", to: "checkout", step: 3, badge: "3" },
    { from: "lock", to: "ledger", step: 3, badge: "4" },
    { from: "lock", to: "payout", step: 3, badge: "5" },
  ] satisfies FlowEdge[],
} as const;

export const pillars = {
  label: "Underneath",
  title: ["Three parts,", "one promise."] as Title,
  aside: "What happens under every lesson: how it plays, how it is locked, and how the money is counted.",
  items: [
    {
      n: "01 / 03",
      word: "Stream.",
      name: "Adaptive streaming",
      body: "Five qualities from 144p to 720p. The player picks what the connection can carry, a few seconds at a time.",
    },
    {
      n: "02 / 03",
      word: "Lock.",
      name: "Encrypted segments",
      body: "Every few seconds is its own encrypted segment. Keys come from a separate key server, and only for a student who paid.",
    },
    {
      n: "03 / 03",
      word: "Count.",
      name: "Double-entry ledger",
      body: "Every sale, fee, refund and payout is written twice. A payment is applied once, however many times it is reported.",
    },
  ],
} as const;

export const promises = {
  label: "Why Zeno",
  title: ["Five promises,", "kept in code."] as Title,
  aside: "Not features. Rules the backend refuses to break, and tests that break things on purpose to prove it.",
  rows: [
    {
      n: "01",
      tag: "Exact",
      word: "once.",
      title: "Every payment applied once",
      body: "A payment provider can send the same news three times. Zeno applies it once, even if the database restarts halfway.",
      more: "Tested by killing the database in the middle of a payment.",
      big: "1×",
      unit: "applied",
    },
    {
      n: "02",
      tag: "Fair",
      word: "capped.",
      title: "Refunds never exceed the sale",
      body: "A refund can never be bigger than what was paid, however it is asked for.",
      more: "Checked by the app, and again by the database itself.",
      big: "≤",
      unit: "what was paid",
    },
    {
      n: "03",
      tag: "Smooth",
      word: "adapt.",
      title: "Quality follows the connection",
      body: "Five qualities from 144p to 720p. A slow connection gets a picture, not a spinner.",
      more: "Audio only, when even 144p is too much.",
      big: "5",
      unit: "qualities",
    },
    {
      n: "04",
      tag: "Private",
      word: "locked.",
      title: "Keys only for students who paid",
      body: "Every segment is encrypted. One account streams on two screens at a time, so a shared password stops being a free course.",
      more: "A silent screen gives up its slot after three minutes.",
      big: "2",
      unit: "screens",
    },
    {
      n: "05",
      tag: "Exact",
      word: "balanced.",
      title: "A ledger that has to add up",
      body: "Every sale, fee, refund and payout is written twice, in a double-entry ledger.",
      more: "Integrity checks flag anything that does not add up.",
      big: "0",
      unit: "off",
    },
  ],
} as const;

export const faq = {
  label: "Questions",
  title: ["Asked,", "answered."] as Title,
  aside: "Short, honest answers. If one of them is not good enough for you, better to know now.",
  items: [
    {
      q: "Can students screen-record my lessons?",
      a: "Honestly, yes. Nobody can stop a phone pointed at a screen. What Zeno stops is the easy part: the files are encrypted, the keys only go to students who paid, and one account can only stream on two screens at once.",
    },
    {
      q: "What if a payment is sent twice?",
      a: "It is applied once. Payment providers retry their notifications; Zeno records each one exactly once, even if the database restarts halfway through.",
    },
    {
      q: "Can a refund be bigger than the payment?",
      a: "No. A refund is capped at what was paid. The app checks it, and the database checks it again on its own.",
    },
    {
      q: "What do my students need?",
      a: "A browser. Nothing to install. Video adjusts between 144p and 720p to fit their connection, and every lesson resumes where they stopped.",
    },
    {
      q: "When do I get paid?",
      a: "Once a sale's refund window closes, your share moves to your payout balance. Past the minimum, you request a payout to your bank.",
    },
  ],
} as const;

export const finale = {
  title: ["Teach what you know.", "Start with one lesson."] as Title,
  ctas: [
    { tag: "01 · Start", label: "Start teaching", sub: "Upload your first lesson.", href: links.start },
    { tag: "02 · Account", label: "Log in", sub: "Pick up where you left off.", href: links.login },
  ],
} as const;

export const footer = {
  columns: [
    { title: "Product", links: nav },
    {
      title: "Account",
      links: [
        { label: "Start teaching", href: links.start },
        { label: "Log in", href: links.login },
      ],
    },
  ],
  rights: "Zeno",
  made: "Made for teachers everywhere",
  top: "Back to top",
} as const;
