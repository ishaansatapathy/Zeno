import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { developmentExtensionId, extensionIdOf } from "../../scripts/extension-id.mjs";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const read = (file: string) => fs.readFileSync(path.join(repo, file), "utf8");

describe("the extension has the same id wherever it is loaded from", () => {
  const id: string = developmentExtensionId();

  it("is 32 letters from a to p, worked out from the key in the manifest, and does not change by accident", () => {
    expect(id).toMatch(/^[a-p]{32}$/);
    // Changing the key changes the id, and then the API and storage would stop trusting the extension.
    expect(id).toBe("fbfpfadnhlgdhefbdmphfdikmpkjmade");
    expect(extensionIdOf("AAAA")).not.toBe(extensionIdOf("AAAB"));
  });

  it("is the one the API trusts as the person (.env.example) and storage allows (compose)", () => {
    expect(read(".env.example")).toContain(`EXTENSION_ORIGINS=chrome-extension://${id}`);
    expect(read("deploy/docker-compose.yml")).toContain(`chrome-extension://${id}`);
  });

  it("is the one the CI job lets storage and the API trust", () => {
    expect(read(".github/workflows/ci.yml")).toContain(`chrome-extension://${id}`);
  });
});
