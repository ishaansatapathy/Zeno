import type { ApiFailure } from "@/lib/api/client";

/** "2 minutes", "1 hour": a wait, as a person would say it. Always rounds up, never promises early. */
export function waitFor(seconds: number | null): string {
  if (seconds === null || seconds <= 0) return "a little while";
  if (seconds < 60) return seconds === 1 ? "1 second" : `${Math.ceil(seconds)} seconds`;
  if (seconds < 3600) {
    const minutes = Math.ceil(seconds / 60);
    return minutes === 1 ? "1 minute" : `${minutes} minutes`;
  }
  const hours = Math.ceil(seconds / 3600);
  return hours === 1 ? "1 hour" : `${hours} hours`;
}

/**
 * What to tell someone when a request failed in a way no field explains. The server's own message
 * is used when it is already good; only the ones that need a number or a different voice are
 * rewritten.
 */
export function describeFailure(failure: ApiFailure): string {
  switch (failure.code) {
    case "network":
      return failure.message;
    case "too_many_requests":
      return `Too many attempts. Try again in ${waitFor(failure.retryAfter)}.`;
    case "unavailable":
      return "Zeno is busy right now. Try again in a moment.";
    case "internal_error":
      return "Something went wrong on our side. Try again in a moment.";
    case "invalid_credentials":
      return "That email and password don't match. Check them and try again.";
    default:
      return failure.message;
  }
}
