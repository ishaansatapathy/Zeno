import fs from "node:fs";

import { API_LOG } from "../playwright.config";

/** How many requests the API has logged for exactly this path, from its JSON log. */
export function apiRequestsTo(path: string): number {
  let count = 0;
  for (const line of fs.readFileSync(API_LOG, "utf8").split("\n")) {
    if (!line.startsWith("{")) continue;
    try {
      const entry = JSON.parse(line) as { fields?: { message?: string }; span?: { path?: string } };
      if (entry.span?.path === path && entry.fields?.message === "finished processing request") count += 1;
    } catch {
      /* not a log line we care about */
    }
  }
  return count;
}
