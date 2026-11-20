"use client";

import { useState, type FormEvent } from "react";

import { api } from "@/lib/api/client";
import { limits } from "@/lib/api/limits";

import { Notice, PasswordField, ResendLink, Submit, TextField, focusField } from "./controls";
import { describeFailure } from "./messages";
import { checkRegister, firstProblem, type Problems } from "./validate";

const ORDER = ["name", "email", "password"];

type Role = "creator" | "student";

const ROLES: { value: Role; title: string; line: string }[] = [
  { value: "creator", title: "Record and share", line: "Make recordings, streams and courses." },
  { value: "student", title: "Watch and learn", line: "Follow what others share with you." },
];

export function RegisterForm() {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("creator");
  const [problems, setProblems] = useState<Problems>({});
  const [alert, setAlert] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [sentTo, setSentTo] = useState<string | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    const found = checkRegister({ name, email, password });
    setProblems(found);
    setAlert(null);
    if (Object.keys(found).length > 0) return focusField(firstProblem(found, ORDER));

    setPending(true);
    const result = await api.auth.register({ name: name.trim(), email: email.trim(), password, role });
    setPending(false);
    if (result.ok) return setSentTo(email.trim());

    const fields = result.error.fields;
    if (Object.keys(fields).length > 0) {
      setProblems(fields);
      focusField(firstProblem(fields, ORDER));
    } else {
      setAlert(describeFailure(result.error));
    }
  }

  // The same answer comes back whether or not the address already has an account, so this screen
  // cannot say which. It says what to do.
  if (sentTo) {
    return (
      <div className="space-y-6">
        <Notice>
          If <span className="text-paper">{sentTo}</span> can be used, a link is on its way. Open it to confirm the address and you're in.
        </Notice>
        <ResendLink email={sentTo} />
        <p className="text-[0.875rem] text-mute">
          Wrong address?{" "}
          <button type="button" onClick={() => setSentTo(null)} className="text-paper underline decoration-line-hi underline-offset-4 hover:decoration-silver">
            Go back
          </button>
        </p>
      </div>
    );
  }

  return (
    <form onSubmit={submit} noValidate className="space-y-5">
      {alert ? <Notice tone="error">{alert}</Notice> : null}
      <TextField id="name" label="Your name" value={name} onChange={setName} error={problems.name} autoComplete="name" autoFocus />
      <TextField id="email" label="Email" type="email" value={email} onChange={setEmail} error={problems.email} autoComplete="email" />
      <PasswordField id="password" label="Password" value={password} onChange={setPassword} error={problems.password} hint={`At least ${limits.passwordMin} characters. A phrase is easier to remember than a word.`} />
      <fieldset>
        <legend className="label mb-2">I'll mostly</legend>
        <div className="grid grid-cols-2 gap-px border border-line-hi bg-line-hi">
          {ROLES.map((option) => (
            <label
              key={option.value}
              className={`block cursor-pointer p-3 transition-colors has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-silver ${
                role === option.value ? "bg-silver text-ink" : "bg-ink text-paper hover:bg-raise"
              }`}
            >
              <input type="radio" name="role" value={option.value} checked={role === option.value} onChange={() => setRole(option.value)} className="sr-only" />
              <span className="caps block">{option.title}</span>
              <span className={`mt-1 block text-[0.78rem] leading-snug ${role === option.value ? "text-ink/70" : "text-mute"}`}>{option.line}</span>
            </label>
          ))}
        </div>
        {problems.role ? <p className="mt-2 text-[0.8125rem] text-fault">{problems.role}</p> : null}
      </fieldset>
      <Submit pending={pending}>Create account</Submit>
    </form>
  );
}
