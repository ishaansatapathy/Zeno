/** Where to go after signing in. */
export const AFTER_SIGN_IN = "/library";

/**
 * The page to return to after signing in, taken from `?next=`. Only a path on this site is
 * honoured: `//evil.example`, `/\evil.example`, `https://evil.example` and `javascript:` would all
 * turn a sign-in link into a redirect to somewhere else, so they fall back to the default.
 */
export function safeNext(raw: string | string[] | undefined): string {
  const value = Array.isArray(raw) ? raw[0] : raw;
  if (!value || value.length > 512) return AFTER_SIGN_IN;
  if (!value.startsWith("/") || value.startsWith("//") || value.includes("\\")) return AFTER_SIGN_IN;
  // Control characters (a newline, a tab) are stripped by some browsers and can undo the checks above.
  if (/[\u0000-\u001f\u007f]/.test(value)) return AFTER_SIGN_IN;
  // Signing in again is pointless from a page that only exists to sign in.
  if (/^\/(login|register|forgot-password|verify-email|reset-password)(\/|\?|$)/.test(value)) return AFTER_SIGN_IN;
  return value;
}
