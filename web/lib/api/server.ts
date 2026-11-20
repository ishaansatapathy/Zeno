// The API as the web server sees it, for pages that are rendered before anyone's browser is
// involved (a link preview, a search crawler, the first paint). Not for use in the browser: that
// has `client.ts`, which calls the page's own origin.

import { cache } from "react";

import type { Schemas } from "./client";

/** Where the API listens, as the web server reaches it. */
const API = (process.env.API_URL ?? "http://127.0.0.1:8080").replace(/\/+$/, "");

/** A share code: 22 URL-safe characters. Anything else cannot be a link, and never reaches the API. */
export const SHARE_CODE = /^[A-Za-z0-9_-]{22}$/;

/** How long a rendered share page, and the answer behind it, is reused. The API says the same. */
export const SHARE_REVALIDATE_SECONDS = 15;

export type Resolved =
  | { kind: "found"; share: Schemas["ShareView"] }
  /** No such link. */
  | { kind: "missing" }
  /** It was a link, and has closed. */
  | { kind: "closed" };

/**
 * What a share link is, as of the last 15 seconds. The page, its metadata and its preview image all
 * ask for it in one render, and many renders within the window share one fetch, so the API hears
 * about a link about four times a minute however many people open it.
 *
 * An API that cannot be reached is an error, not an answer: Next then keeps serving the page it
 * made last time instead of caching a page that says the link is gone.
 */
export const resolveShare = cache(async (code: string): Promise<Resolved> => {
  if (!SHARE_CODE.test(code)) return { kind: "missing" };
  const response = await fetch(`${API}/v1/s/${code}`, {
    headers: { accept: "application/json" },
    next: { revalidate: SHARE_REVALIDATE_SECONDS },
  });
  if (response.status === 404) return { kind: "missing" };
  if (response.status === 410) return { kind: "closed" };
  if (!response.ok) throw new Error(`The API answered ${response.status} for a share link`);
  const body = (await response.json()) as { share: Schemas["ShareView"] };
  return { kind: "found", share: body.share };
});
