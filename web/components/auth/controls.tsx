"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";

import { api } from "@/lib/api/client";

import { describeFailure } from "./messages";

/** Puts the cursor in a field by id, once the page has had a chance to show its error. */
export function focusField(id: string | undefined) {
  if (id) requestAnimationFrame(() => document.getElementById(id)?.focus());
}

interface FieldProps {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  error?: string;
  hint?: ReactNode;
  type?: "text" | "email" | "password";
  autoComplete?: string;
  autoFocus?: boolean;
  maxLength?: number;
  disabled?: boolean;
  /** Something set inside the field's right edge (the show/hide switch). */
  adornment?: ReactNode;
}

/**
 * A labelled input with its error and hint tied to it, so a screen reader reads them with the
 * field. The error is in the DOM only when there is one.
 */
export function TextField({ id, label, value, onChange, error, hint, type = "text", autoComplete, autoFocus, maxLength, disabled, adornment }: FieldProps) {
  const describedBy = [error ? `${id}-error` : null, hint ? `${id}-hint` : null].filter(Boolean).join(" ") || undefined;
  return (
    <div>
      <label htmlFor={id} className="label mb-2 block">
        {label}
      </label>
      <div className="relative">
        <input
          id={id}
          name={id}
          type={type}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          autoComplete={autoComplete}
          autoFocus={autoFocus}
          maxLength={maxLength}
          disabled={disabled}
          aria-invalid={error ? true : undefined}
          aria-describedby={describedBy}
          spellCheck={false}
          autoCapitalize="off"
          className={`h-12 w-full border bg-ink px-4 text-[0.95rem] text-paper placeholder:text-mute/60 transition-colors disabled:opacity-60 ${
            error ? "border-fault" : "border-line-hi focus:border-silver"
          } ${adornment ? "pr-20" : ""}`}
        />
        {adornment ? <div className="absolute inset-y-0 right-0 flex items-center pr-1">{adornment}</div> : null}
      </div>
      {error ? (
        <p id={`${id}-error`} className="mt-2 text-[0.8125rem] text-fault">
          {error}
        </p>
      ) : null}
      {hint ? (
        <p id={`${id}-hint`} className="mt-2 text-[0.8125rem] text-mute">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

/** A password field with a switch to see what was typed. */
export function PasswordField(props: Omit<FieldProps, "type" | "adornment"> & { current?: boolean }) {
  const [shown, setShown] = useState(false);
  const { current, autoComplete, ...rest } = props;
  return (
    <TextField
      {...rest}
      type={shown ? "text" : "password"}
      autoComplete={autoComplete ?? (current ? "current-password" : "new-password")}
      adornment={
        <button
          type="button"
          onClick={() => setShown((value) => !value)}
          aria-pressed={shown}
          aria-controls={props.id}
          className="caps h-10 px-3 text-mute transition-colors hover:text-paper"
        >
          {shown ? "Hide" : "Show"}
        </button>
      }
    />
  );
}

/** The one box that sends a form: silver, filling with black on hover, honest while it waits. */
export function Submit({ pending, children }: { pending: boolean; children: ReactNode }) {
  return (
    <button
      type="submit"
      disabled={pending}
      aria-busy={pending}
      className="caps fillup fillup-ink h-12 w-full bg-silver text-ink disabled:cursor-wait disabled:opacity-70"
    >
      {pending ? "One moment…" : children}
    </button>
  );
}

/**
 * A message above the form. An error is an alert, announced at once; anything else is a status,
 * announced politely.
 */
export function Notice({ tone = "info", children }: { tone?: "info" | "error"; children: ReactNode }) {
  return (
    <div
      role={tone === "error" ? "alert" : "status"}
      className={`mb-6 border px-4 py-3 text-[0.875rem] leading-relaxed ${tone === "error" ? "border-fault/50 text-fault" : "border-line-hi text-paper"}`}
    >
      {children}
    </div>
  );
}

/** A quiet button that looks like the footer links. */
export function TextButton({ onClick, disabled, children }: { onClick: () => void; disabled?: boolean; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="text-paper underline decoration-line-hi underline-offset-4 transition-colors hover:decoration-silver disabled:cursor-not-allowed disabled:text-mute disabled:no-underline"
    >
      {children}
    </button>
  );
}

const RESEND_COOLDOWN_SECONDS = 30;

/**
 * "Send it again": for a link that did not arrive. The same answer comes back whether or not an
 * account exists, so this says nothing about who has one. It waits 30 seconds between sends so a
 * double click is not two emails.
 */
export function ResendLink({ email, intro = "Didn't get it?" }: { email: string; intro?: string }) {
  const [wait, setWait] = useState(0);
  const [note, setNote] = useState<{ tone: "info" | "error"; text: string } | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => () => void (timer.current && clearInterval(timer.current)), []);

  async function send() {
    setNote(null);
    setWait(RESEND_COOLDOWN_SECONDS);
    timer.current = setInterval(() => {
      setWait((left) => {
        if (left <= 1 && timer.current) clearInterval(timer.current);
        return Math.max(0, left - 1);
      });
    }, 1000);
    const result = await api.auth.resendVerification(email);
    setNote(result.ok ? { tone: "info", text: "Sent again. It can take a minute." } : { tone: "error", text: describeFailure(result.error) });
  }

  return (
    <div className="text-[0.875rem] text-mute">
      <p>
        {intro}{" "}
        <TextButton onClick={send} disabled={wait > 0}>
          {wait > 0 ? `Send again in ${wait}s` : "Send it again"}
        </TextButton>
      </p>
      {note ? (
        <p role={note.tone === "error" ? "alert" : "status"} className={`mt-2 ${note.tone === "error" ? "text-fault" : "text-paper"}`}>
          {note.text}
        </p>
      ) : null}
    </div>
  );
}
