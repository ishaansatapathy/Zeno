import net from "node:net";

import { E2E_REDIS_URL } from "../playwright.config";

/** Empties the Redis database the API uses for this run, so earlier runs' rate limits are gone. */
export default async function globalSetup() {
  const url = new URL(E2E_REDIS_URL);
  const db = url.pathname.replace("/", "") || "0";
  if (db === "0") throw new Error("Refusing to flush Redis database 0; point E2E_REDIS_URL at a database used only for tests.");
  await new Promise<void>((resolve, reject) => {
    const socket = net.connect({ host: url.hostname, port: Number(url.port || 6379) }, () => {
      // RESP: SELECT <db>, then FLUSHDB. Only these two commands, so no client library is needed.
      const command = (...parts: string[]) => `*${parts.length}\r\n${parts.map((p) => `$${Buffer.byteLength(p)}\r\n${p}\r\n`).join("")}`;
      socket.write(command("SELECT", db) + command("FLUSHDB"));
    });
    let seen = "";
    socket.on("data", (chunk) => {
      seen += chunk.toString();
      if ((seen.match(/\+OK/g) ?? []).length >= 2) {
        socket.end();
        resolve();
      } else if (seen.includes("-ERR")) {
        socket.destroy();
        reject(new Error(`Redis refused: ${seen.trim()}`));
      }
    });
    socket.on("error", reject);
  });
}
