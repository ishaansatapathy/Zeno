/**
 * Every word on the public site. Claims here are things the backend actually does today, with the
 * numbers it actually uses by default: five qualities from 144p to 720p, two streams at a time,
 * uploads up to 10 GB, refunds capped at what was paid, each payment applied exactly once.
 * Nothing here is tied to one country or one currency.
 */

export const links = {
  start: "/sign-up",
  login: "/sign-in",
} as const;

export const nav = [
  { label: "How it works", href: "#how" },
  { label: "The deck", href: "#deck" },
  { label: "Proof", href: "#proof" },
  { label: "FAQ", href: "#faq" },
] as const;

export const hero = {
  tag: "video courses · live classes",
  line1: "Teach what you know.",
  line2: "Keep what you",
  sprayed: "earn",
  body: "Zeno is the school behind your lessons: streaming that is hard to rip, payments that are counted once, and a ledger that has to balance before you get paid.",
  cta: "Start teaching",
  plate: [
    { k: "Qualities", v: "5" },
    { k: "Streams", v: "2" },
    { k: "Charged", v: "1×" },
  ],
  plateFoot: "lesson 07 · locked",
} as const;

export const statement = {
  text: "Every lesson is cut into segments and every segment is locked. Every sale lands in a ledger that refuses to be out of balance. Every payout goes to your bank.",
  last: "You just teach.",
  aside:
    "Most course tools leave the hard parts to you: the leaked link, the payment counted twice, the refund nobody can explain. Zeno does them underneath, the same way every time.",
  chips: ["Encrypted video", "Double-entry ledger"],
  cta: { label: "See how it works", href: "#how" },
} as const;

export const process = {
  lead: "From the",
  struck: "classroom",
  mid: "to the",
  sprayed: "world",
  steps: [
    {
      n: "01",
      title: "Upload",
      body: "Straight from the browser, in parts. A dropped connection costs one part, not the whole file.",
      stats: [
        { k: "Up to", v: "10 GB" },
        { k: "Hours", v: "6" },
      ],
    },
    {
      n: "02",
      title: "Encode",
      body: "One file becomes five qualities, so a slow connection gets a picture instead of a spinner.",
      stats: [
        { k: "Qualities", v: "5" },
        { k: "From", v: "144p" },
      ],
    },
    {
      n: "03",
      title: "Lock",
      body: "Every segment is encrypted. Keys go only to students who paid, checked on every request.",
      stats: [
        { k: "AES", v: "128" },
        { k: "Streams", v: "2" },
      ],
    },
    {
      n: "04",
      title: "Get paid",
      body: "Each sale is written twice in a ledger that must balance. Then it moves to your payout.",
      stats: [
        { k: "Charged", v: "1×" },
        { k: "Off by", v: "0" },
      ],
    },
  ],
} as const;

export type DeckCard = {
  id: string;
  image: 1 | 2 | 3 | 4 | 5 | 6;
  name: string;
  feature: string;
  line: string;
  stat: { k: string; v: string };
  alt: string;
};

/** The six cards of the character, each standing for one thing Zeno does. */
export const deck = {
  lead: "Everything's in the",
  sprayed: "deck",
  body: "Six cards, six jobs Zeno does so you don't have to. Drag, click or use the arrow keys.",
  cards: [
    {
      id: "chill",
      image: 1,
      name: "The Chill",
      feature: "Adaptive video",
      line: "Quality follows the connection, 144p to 720p, without a spinner.",
      stat: { k: "Qualities", v: "5" },
      alt: "The Zeno robot, monitor for a face, sitting back on a giant video player.",
    },
    {
      id: "peace",
      image: 2,
      name: "The Peace",
      feature: "Fair refunds",
      line: "Refunds inside your window, never more than what was paid.",
      stat: { k: "Refund", v: "≤ paid" },
      alt: "The Zeno robot sitting cross-legged on a player bar, throwing a peace sign.",
    },
    {
      id: "lean",
      image: 3,
      name: "The Lean",
      feature: "Live classes",
      line: "Go live from the streaming app you already use. Students watch in the browser.",
      stat: { k: "Ingest", v: "RTMP" },
      alt: "The Zeno robot leaning on a video screen, hands in its hoodie pocket.",
    },
    {
      id: "skip",
      image: 4,
      name: "The Skip",
      feature: "Resume anywhere",
      line: "Every lesson remembers where each student stopped.",
      stat: { k: "Progress", v: "saved" },
      alt: "The Zeno robot on a player with skip buttons, flashing a peace sign.",
    },
    {
      id: "drop",
      image: 5,
      name: "The Drop",
      feature: "Resumable uploads",
      line: "Big files go up in parts and pick up where they stopped.",
      stat: { k: "Up to", v: "10 GB" },
      alt: "The Zeno robot holding up a glowing play button.",
    },
    {
      id: "wave",
      image: 6,
      name: "The Wave",
      feature: "Payouts",
      line: "Your share, counted in a double-entry ledger, paid out to your bank.",
      stat: { k: "Entries", v: "×2" },
      alt: "The Zeno robot waving from the top of a video player.",
    },
  ] satisfies DeckCard[],
} as const;

export const tape = ["Encrypted", "Encoded", "Counted once", "Paid out", "Resumed", "Live"] as const;

export const protect = {
  points: [
    "Every few seconds of video is its own encrypted segment. Without a key, a downloaded file is noise.",
    "Keys come from a separate key server that checks a signed, short-lived pass on every request.",
    "One account streams on two screens at a time. A shared password stops being a free course.",
  ],
  foot: { k: "Lesson 07", v: "720p · locked" },
  alt: "The Zeno robot leaning on a locked video screen.",
} as const;

export const proof = {
  lead: "The parts nobody",
  sprayed: "demos",
  tiles: [
    { k: "Payment applied", v: 1, suffix: "×", tone: "cream" },
    { k: "Qualities per video", v: 5, suffix: "", tone: "violet" },
    { k: "Streams at a time", v: 2, suffix: "", tone: "outline" },
  ],
  seal: { big: "0", ring: "double charges · by design · " },
  rows: [
    { at: "Event 01", what: "Payment confirmed. The provider sent it three times", ref: "webhook", stamp: "Applied once" },
    { at: "Event 02", what: "Provider fee taken from what the provider reported", ref: "ledger", stamp: "Posted" },
    { at: "Event 03", what: "Refund asked for more than was paid. Refused", ref: "refund", stamp: "Capped" },
    { at: "Event 04", what: "Refund window closed. Share moved to payout", ref: "payout", stamp: "Paid" },
  ],
} as const;

export const faq = {
  lead: "Ask us the",
  line2: "hard",
  sprayed: "stuff",
  note: "Short, honest answers. If one of them is not good enough for you, that is worth knowing early.",
  items: [
    {
      q: "Can students screen-record my lessons?",
      a: "Honestly, yes. Nobody can stop a phone pointed at a screen. What Zeno stops is the easy part: the files are encrypted, the keys only go to students who paid, and one account can only stream on two screens at once.",
    },
    {
      q: "What if a payment is sent twice?",
      a: "It is applied once. Payment providers retry their notifications; Zeno records each one exactly once, even if the database restarts halfway through. We test that by killing it on purpose.",
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

export const join = {
  lead: "Bring your first",
  sprayed: "course",
  body: "Zeno is opening to educators in small groups. Upload a lesson, set a price, share the link.",
  primary: "Start teaching",
  secondary: "Log in",
  foot: "Video courses · live classes · payouts",
  alt: "The Zeno robot waving hello.",
} as const;

export const footer = {
  tagline: "Video courses and live classes for independent educators.",
  made: "Made for teachers everywhere.",
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
} as const;
