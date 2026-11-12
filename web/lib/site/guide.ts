import { createAvatar } from "@dicebear/core";
import { openPeeps } from "@dicebear/collection";

/**
 * The Zeno guide: one person, drawn from Open Peeps by Pablo Stanley (CC0), through DiceBear's
 * remix of it (artwork CC0, code MIT). Every option is fixed except the face, so it is always the
 * same character, only its expression changes from section to section.
 */
export const moods = ["smile", "explaining", "awe", "smileBig", "calm", "cute", "eyesClosed"] as const;
export type Mood = (typeof moods)[number];

export function guideSvg(mood: Mood): string {
  return createAvatar(openPeeps, {
    seed: "zeno-guide",
    face: [mood],
    head: ["bun2"],
    accessories: ["glasses2"],
    accessoriesProbability: 100,
    facialHairProbability: 0,
    maskProbability: 0,
    skinColor: ["d08b5b"],
    clothingColor: ["e8a33d"],
    headContrastColor: ["2c1b18"],
  }).toString();
}
