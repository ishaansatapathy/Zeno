"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from "react";

import { Notice, PasswordField, TextField, focusField } from "@/components/auth/controls";
import { describeFailure } from "@/components/auth/messages";
import { checkName, checkNewPassword } from "@/components/auth/validate";
import { api, type Schemas } from "@/lib/api/client";
import { formatBytes, percentUsed } from "@/lib/bytes";
import { useSession } from "@/lib/session";

/** One box on the page: a heading, what it is for, and the thing itself. */
function Panel({ id, title, intro, children }: { id: string; title: string; intro: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="border border-line bg-panel/60 p-6 sm:p-8">
      <h2 id={id} className="label text-paper">
        {title}
      </h2>
      <p className="mt-3 max-w-prose text-[0.9rem] leading-relaxed text-mute">{intro}</p>
      <div className="mt-6">{children}</div>
    </section>
  );
}

const button = "caps fillup fillup-ink h-12 bg-silver px-8 text-ink disabled:cursor-wait disabled:opacity-70";

/** What the person is called. */
function NameForm() {
  const { session, setUser } = useSession();
  const current = session.status === "signed-in" ? session.user.name : "";
  const [name, setName] = useState(current);
  const [problem, setProblem] = useState<string | undefined>();
  const [alert, setAlert] = useState<string | null>(null);
  const [saved, setSaved] = useState("");
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    setSaved("");
    setAlert(null);
    const found = checkName(name);
    setProblem(found ?? undefined);
    if (found) return focusField("name");
    if (name.trim() === current) return setSaved("That is already your name.");

    setPending(true);
    const result = await api.auth.rename(name);
    setPending(false);
    if (result.ok) {
      setUser(result.data.user);
      setName(result.data.user.name);
      setSaved("Name saved.");
      return;
    }
    if (result.error.fields.name) {
      setProblem(result.error.fields.name);
      focusField("name");
    } else {
      setAlert(describeFailure(result.error));
    }
  }

  return (
    <form onSubmit={submit} noValidate className="max-w-sm space-y-5">
      {alert ? <Notice tone="error">{alert}</Notice> : null}
      <TextField id="name" label="Name" value={name} onChange={setName} error={problem} autoComplete="name" maxLength={80} hint="Shown beside your comments." />
      <div className="flex items-center gap-5">
        <button type="submit" disabled={pending} aria-busy={pending} className={button}>
          {pending ? "One moment…" : "Save name"}
        </button>
        <p role="status" aria-live="polite" className="label min-h-5 text-paper">
          {saved}
        </p>
      </div>
    </form>
  );
}

/** How much of the allowance is used, from the API, which is the one that counts. */
function Storage() {
  const [state, setState] = useState<{ phase: "loading" } | { phase: "failed"; message: string } | { phase: "ready"; storage: Schemas["StorageUsage"] }>({ phase: "loading" });

  const load = useCallback(async () => {
    setState({ phase: "loading" });
    const result = await api.account.storage();
    setState(result.ok ? { phase: "ready", storage: result.data.storage } : { phase: "failed", message: describeFailure(result.error) });
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (state.phase === "loading") {
    return (
      <div role="status">
        <span className="sr-only">Loading your storage</span>
        <div aria-hidden="true" className="h-3 w-full max-w-md bg-raise" />
      </div>
    );
  }
  if (state.phase === "failed") {
    return (
      <div className="space-y-4">
        <Notice tone="error">{state.message}</Notice>
        <button type="button" onClick={() => void load()} className={button}>
          Try again
        </button>
      </div>
    );
  }

  const { used_bytes: used, quota_bytes: quota, remaining_bytes: left, videos } = state.storage;
  const percent = percentUsed(used, quota);
  return (
    <div className="max-w-md">
      <p className="text-[1.05rem] text-paper">
        {formatBytes(used)} <span className="text-mute">of {formatBytes(quota)} used</span>
      </p>
      <div
        role="progressbar"
        aria-label="Storage used"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={`${formatBytes(used)} of ${formatBytes(quota)}`}
        className="mt-4 h-2 w-full border border-line-hi bg-ink"
      >
        <div className={`h-full ${percent >= 100 ? "bg-fault" : "bg-silver"}`} style={{ width: `${percent}%` }} />
      </div>
      <p className="mt-3 text-[0.85rem] text-mute">
        {formatBytes(left)} left · {videos === 1 ? "1 video" : `${videos} videos`} counted
      </p>
      {left === 0 ? (
        <p role="status" className="mt-5 text-[0.9rem] text-paper">
          Your storage is full, so new uploads are refused (a recording being made is still kept on your computer).{" "}
          <Link href="/library" className="underline decoration-line-hi underline-offset-4 hover:decoration-silver">
            Delete what you no longer need
          </Link>{" "}
          to make room.
        </p>
      ) : null}
    </div>
  );
}

/** A new password: it asks for the old one, and ends every session, this one too. */
function PasswordForm() {
  const router = useRouter();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [problems, setProblems] = useState<Record<string, string>>({});
  const [alert, setAlert] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    setAlert(null);
    const found: Record<string, string> = {};
    if (!current) found.current_password = "Enter your current password.";
    const weak = checkNewPassword(next);
    if (weak) found.new_password = weak;
    else if (next === current) found.new_password = "Choose a password you are not using now.";
    if (next !== confirm) found.confirm = "The two passwords don't match.";
    setProblems(found);
    const first = ["current_password", "new_password", "confirm"].find((field) => field in found);
    if (first) return focusField(first);

    setPending(true);
    const result = await api.auth.changePassword(current, next);
    if (result.ok) {
      // Every session is over, this one too: the way on is to sign in with the new password.
      router.replace("/login?arrival=password-changed");
      return;
    }
    setPending(false);
    const { code, fields } = result.error;
    if (code === "wrong_password") {
      setProblems({ current_password: "That is not your current password." });
      focusField("current_password");
    } else if (fields.new_password) {
      setProblems({ new_password: fields.new_password });
      focusField("new_password");
    } else {
      setAlert(describeFailure(result.error));
    }
  }

  return (
    <form onSubmit={submit} noValidate className="max-w-sm space-y-5">
      {alert ? <Notice tone="error">{alert}</Notice> : null}
      <PasswordField id="current_password" label="Current password" value={current} onChange={setCurrent} error={problems.current_password} current />
      <PasswordField id="new_password" label="New password" value={next} onChange={setNext} error={problems.new_password} hint="At least 10 characters." />
      <PasswordField id="confirm" label="Type the new one again" value={confirm} onChange={setConfirm} error={problems.confirm} />
      <button type="submit" disabled={pending} aria-busy={pending} className={button}>
        {pending ? "One moment…" : "Change password"}
      </button>
    </form>
  );
}

/** What a signed-in person can see and change about their own account. */
export function Settings() {
  const { session } = useSession();
  if (session.status !== "signed-in") return null;
  return (
    <>
      <p className="label">Signed in as {session.user.email}</p>
      <h1 className="display mt-6 text-[clamp(2.4rem,6vw,4.5rem)]">
        <span className="block">Your</span>
        <span className="block">
          <em className="metal">settings.</em>
        </span>
      </h1>
      <div className="mt-12 space-y-6">
        <Panel id="storage-heading" title="Storage" intro="What you have uploaded and still have, counted at the size you sent it. Deleting a recording gives its room back.">
          <Storage />
        </Panel>
        <Panel id="name-heading" title="Name" intro="What people see next to your comments.">
          <NameForm />
        </Panel>
        <Panel id="password-heading" title="Password" intro="Changing it signs you out everywhere, on every device, so anyone else who had it or had your session is out too. You sign in again with the new one.">
          <PasswordForm />
        </Panel>
      </div>
    </>
  );
}
