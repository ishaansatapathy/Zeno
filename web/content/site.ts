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

export type Stage = { name: string; time: string };
export type Step = {
  id: string;
  n: string;
  name: string;
  blurb: string;
  log: string;
  stages: Stage[];
  claimLabel: string;
  claim: Title;
  status: string;
  side: { k: string; v: string };
  facts: { k: string; v: string; accent?: boolean }[];
};

export const bench = {
  label: "See it work",
  title: ["One lesson,", "four steps."] as Title,
  aside: "Pick a step and watch what Zeno does with a single lesson. It is a simulation; the limits and numbers are the ones the platform runs with.",
  index: "Step index",
  steps: [
    {
      id: "upload",
      n: "01",
      name: "Upload",
      blurb: "Straight from the browser, in parts.",
      log: "upload lesson-07.mp4  size 1.24 GB  parts 24",
      stages: [
        { name: "Signed", time: "0.02s" },
        { name: "Parts 24/24", time: "41.6s" },
        { name: "Checked", time: "0.31s" },
      ],
      claimLabel: "Upload · resumable",
      claim: ["Lesson 07", "uploaded."],
      status: "Ready to encode",
      side: { k: "Retried", v: "part 13" },
      facts: [
        { k: "Up to", v: "10 GB" },
        { k: "Length", v: "6 hours" },
        { k: "Dropped connection", v: "costs one part", accent: true },
        { k: "Original", v: "deleted after 7 days" },
      ],
    },
    {
      id: "encode",
      n: "02",
      name: "Encode",
      blurb: "Five qualities from one file.",
      log: "encode lesson-07  ladder 144p → 720p  never upscaled",
      stages: [
        { name: "144p", time: "0.9s" },
        { name: "240p", time: "1.4s" },
        { name: "360p", time: "2.2s" },
        { name: "480p", time: "3.1s" },
        { name: "720p", time: "5.8s" },
      ],
      claimLabel: "Encode · adaptive",
      claim: ["Five", "qualities."],
      status: "Adaptive stream ready",
      side: { k: "Keyframes", v: "aligned" },
      facts: [
        { k: "Lowest", v: "144p" },
        { k: "Highest", v: "720p" },
        { k: "Slow connection", v: "gets a picture", accent: true },
        { k: "Fallback", v: "audio only" },
      ],
    },
    {
      id: "lock",
      n: "03",
      name: "Lock",
      blurb: "Every segment encrypted.",
      log: "lock lesson-07  aes-128  key wrapped, never stored plain",
      stages: [
        { name: "Key made", time: "0.01s" },
        { name: "Segments", time: "1.9s" },
        { name: "Key wrapped", time: "0.01s" },
      ],
      claimLabel: "Lock · per segment",
      claim: ["Every segment", "locked."],
      status: "Keys for paid students only",
      side: { k: "Screens", v: "2 at once" },
      facts: [
        { k: "Cipher", v: "AES-128" },
        { k: "Pass", v: "signed, short-lived" },
        { k: "Downloaded file", v: "is noise", accent: true },
        { k: "Idle screen", v: "frees its slot" },
      ],
    },
    {
      id: "paid",
      n: "04",
      name: "Get paid",
      blurb: "Counted once, paid out.",
      log: "payment.captured  received ×3  same payment",
      stages: [
        { name: "Received ×3", time: "0.00s" },
        { name: "Applied ×1", time: "0.04s" },
        { name: "Posted", time: "0.01s" },
      ],
      claimLabel: "Ledger · double entry",
      claim: ["Applied", "once."],
      status: "Ledger balanced",
      side: { k: "Off by", v: "0.00" },
      facts: [
        { k: "Entries", v: "two per sale" },
        { k: "Fee", v: "as the provider reports" },
        { k: "Refund", v: "never more than paid", accent: true },
        { k: "Payout", v: "to your bank" },
      ],
    },
  ] satisfies Step[],
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
