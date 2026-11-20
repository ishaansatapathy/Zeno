// Checks a form can make before asking the server, so a typo costs no round trip. The server stays
// the authority (it also refuses common passwords and ones built from the email address); these
// say the same thing in the same words for the rules a form can know.

import { limits } from "@/lib/api/limits";

export type Problems = Record<string, string>;

/** Characters as the server counts them (Unicode scalar values), not UTF-16 units. */
const length = (text: string) => [...text].length;

export function checkName(raw: string): string | null {
  const n = length(raw.trim());
  return n >= limits.nameMin && n <= limits.nameMax ? null : `Name must be ${limits.nameMin} to ${limits.nameMax} characters.`;
}

export function checkEmail(raw: string): string | null {
  const value = raw.trim();
  if (!value) return "Email is required.";
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value) ? null : "Enter a valid email address.";
}

export function checkNewPassword(raw: string): string | null {
  const n = length(raw);
  if (n < limits.passwordMin) return `Password must be at least ${limits.passwordMin} characters.`;
  if (n > limits.passwordMax) return `Password must be at most ${limits.passwordMax} characters.`;
  return null;
}

function collect(entries: [string, string | null][]): Problems {
  const problems: Problems = {};
  for (const [field, message] of entries) if (message) problems[field] = message;
  return problems;
}

export const checkRegister = (v: { name: string; email: string; password: string }): Problems =>
  collect([
    ["name", checkName(v.name)],
    ["email", checkEmail(v.email)],
    ["password", checkNewPassword(v.password)],
  ]);

export const checkLogin = (v: { email: string; password: string }): Problems =>
  collect([
    ["email", checkEmail(v.email)],
    ["password", v.password ? null : "Enter your password."],
  ]);

export const checkReset = (v: { password: string; confirm: string }): Problems =>
  collect([
    ["new_password", checkNewPassword(v.password)],
    ["confirm", v.password === v.confirm ? null : "The two passwords don't match."],
  ]);

/** The field to put the cursor in, given the order they appear on screen. */
export function firstProblem(problems: Problems, order: string[]): string | undefined {
  return order.find((field) => field in problems) ?? Object.keys(problems)[0];
}
