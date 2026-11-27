import fs from "node:fs";

import { API_LOG } from "../playwright.config";

interface LogLine {
  fields?: { message?: string; to?: string; subject?: string };
}

type Kind = "verify-email" | "reset-password" | "invite";

/** Every link of this kind the API has "emailed" to `email`, oldest first. */
function linksTo(email: string, kind: Kind): string[] {
  const pattern = new RegExp(`/${kind}/[A-Za-z0-9_-]+`);
  const found: string[] = [];
  for (const line of fs.readFileSync(API_LOG, "utf8").split("\n")) {
    if (!line.startsWith("{")) continue;
    let entry: LogLine;
    try {
      entry = JSON.parse(line) as LogLine;
    } catch {
      continue;
    }
    if (entry.fields?.to !== email) continue;
    const match = pattern.exec(entry.fields.message ?? "");
    if (match) found.push(match[0]);
  }
  return found;
}

/** How many links of this kind have been mailed to `email` so far. */
export const mailedCount = (email: string, kind: Kind) => linksTo(email, kind).length;

/**
 * The path of the newest link the API "emailed" to `email`: `/verify-email/<token>` or
 * `/reset-password/<token>`. The development transport prints each message to the API's log, so
 * this reads the log, waiting a little for the message to be sent after the request that caused
 * it. `after` is how many links were already there, when a new one is wanted.
 */
export async function mailedLink(email: string, kind: Kind, after = 0): Promise<string> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    const found = linksTo(email, kind);
    if (found.length > after) return found[found.length - 1];
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`No ${kind} email for ${email} in the API log`);
}
