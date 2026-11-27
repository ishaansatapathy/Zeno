import path from "node:path";
import { fileURLToPath } from "node:url";

// Where the API listens. The browser never sees this: it calls /v1 on the page's own origin, and
// this server forwards those calls. In production the edge routes /v1 to the API directly, and
// this file is not involved.
const api = (process.env.API_URL ?? "http://127.0.0.1:8080").replace(/\/+$/, "");

/** @type {import('next').NextConfig} */
const nextConfig = {
  poweredByHeader: false,
  reactStrictMode: true,
  images: { formats: ["image/avif", "image/webp"] },
  // The app lives in web/, with its own lockfile. Say so, rather than let Next guess from
  // whatever lockfiles happen to sit further up the disk.
  turbopack: { root: path.dirname(fileURLToPath(import.meta.url)) },

  async rewrites() {
    // The session cookies are set by the API and belong to the page's origin because of this, so
    // there is no CORS to configure and the API's origin check sees the page's own origin.
    return [{ source: "/v1/:path*", destination: `${api}/v1/:path*` }];
  },

  async headers() {
    // A link in an email carries a one-time token in its path. Keep it out of the Referer of
    // anything the page loads or links to, and out of every cache.
    const private_ = [
      { key: "Referrer-Policy", value: "no-referrer" },
      { key: "Cache-Control", value: "no-store" },
    ];
    // A share link is a capability: whoever holds the address may watch. It is kept out of the
    // Referer of anything the page loads, and out of search results. (Rendered pages are reused for
    // 15 seconds by Next itself, so no Cache-Control is set here.)
    const capability = [
      { key: "Referrer-Policy", value: "no-referrer" },
      { key: "X-Robots-Tag", value: "noindex, nofollow" },
    ];
    return [
      { source: "/verify-email/:token", headers: private_ },
      { source: "/reset-password/:token", headers: private_ },
      { source: "/invite/:token", headers: private_ },
      { source: "/s/:code", headers: capability },
      { source: "/s/:code/:path*", headers: capability },
    ];
  },
};

export default nextConfig;
