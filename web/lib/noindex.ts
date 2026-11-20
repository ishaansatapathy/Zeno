import type { Metadata } from "next";

/** Account screens are not for search results, and the ones with a token must not be kept at all. */
export const noindex: Metadata["robots"] = { index: false, follow: false };
