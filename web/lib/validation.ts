/**
 * Checks a form can make before it asks the server: quick feedback, in the person's language.
 * The server checks everything again (these are a courtesy, not a defence), and its answer is the
 * truth if the two ever disagree. Each failure is a key in the `validation` messages.
 */

export const PASSWORD_MIN_CHARS = 10;
export const PASSWORD_MAX_CHARS = 128;
export const NAME_MIN_CHARS = 2;
export const NAME_MAX_CHARS = 80;

export type FieldKey = "required" | "email" | "passwordMin" | "passwordMax" | "nameLength";
export type Problems<Field extends string> = Partial<Record<Field, FieldKey>>;

/** Counts what a person would count: "🙂" and "क्षि" are one character each to the API, too. */
const length = (text: string) => [...text].length;

/** Something@something.something, with no spaces. The server decides what is really valid. */
export function looksLikeEmail(text: string): boolean {
  return text.length <= 254 && /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/.test(text);
}

export function validateSignIn(input: { email: string; password: string }): Problems<"email" | "password"> {
  const problems: Problems<"email" | "password"> = {};
  if (input.email.trim() === "") problems.email = "required";
  else if (!looksLikeEmail(input.email.trim())) problems.email = "email";
  if (input.password === "") problems.password = "required";
  return problems;
}

export function validateSignUp(input: {
  name: string;
  email: string;
  password: string;
}): Problems<"name" | "email" | "password"> {
  const problems: Problems<"name" | "email" | "password"> = {};
  const name = input.name.trim();
  if (name === "") problems.name = "required";
  else if (length(name) < NAME_MIN_CHARS || length(name) > NAME_MAX_CHARS) problems.name = "nameLength";
  if (input.email.trim() === "") problems.email = "required";
  else if (!looksLikeEmail(input.email.trim())) problems.email = "email";
  if (input.password === "") problems.password = "required";
  else if (length(input.password) < PASSWORD_MIN_CHARS) problems.password = "passwordMin";
  else if (length(input.password) > PASSWORD_MAX_CHARS) problems.password = "passwordMax";
  return problems;
}
