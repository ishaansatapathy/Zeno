// The id Chrome gives an extension is fixed by its public key: the first 16 bytes of the key's SHA-256, written
// with the letters a to p. The manifest carries a key so that the extension loaded from dist/ always has the same
// id, which the API (EXTENSION_ORIGINS) and storage (CORS) are told to trust. This is the one place that works it out.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export function extensionIdOf(base64Key) {
  const digest = crypto.createHash("sha256").update(Buffer.from(base64Key, "base64")).digest().subarray(0, 16);
  return [...digest].map((byte) => String.fromCharCode(97 + (byte >> 4)) + String.fromCharCode(97 + (byte & 15))).join("");
}

/** The id of the extension built from src/manifest.json. */
export function developmentExtensionId() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const manifest = JSON.parse(fs.readFileSync(path.join(root, "src", "manifest.json"), "utf8"));
  if (typeof manifest.key !== "string") throw new Error("src/manifest.json has no key");
  return extensionIdOf(manifest.key);
}
