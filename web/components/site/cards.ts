import type { StaticImageData } from "next/image";

import card1 from "@/assets/cards/card-1.webp";
import card2 from "@/assets/cards/card-2.webp";
import card3 from "@/assets/cards/card-3.webp";
import card4 from "@/assets/cards/card-4.webp";
import card5 from "@/assets/cards/card-5.webp";
import card6 from "@/assets/cards/card-6.webp";

/**
 * The character's six trading cards, cut from their photos by scripts/extract-cards.py: 900 by
 * 1200, corners rounded into transparency. Imported statically so Next knows their size and
 * serves each one resized to the space it fills.
 */
export const cards: Record<1 | 2 | 3 | 4 | 5 | 6, StaticImageData> = {
  1: card1,
  2: card2,
  3: card3,
  4: card4,
  5: card5,
  6: card6,
};
