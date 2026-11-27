"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState, type FormEvent } from "react";

import { Notice, TextField, focusField } from "@/components/auth/controls";
import { describeFailure } from "@/components/auth/messages";
import { checkName } from "@/components/auth/validate";
import { api, type Schemas } from "@/lib/api/client";
import { ROLE_LABEL } from "@/lib/team";

type State = { phase: "loading" } | { phase: "failed"; message: string } | { phase: "ready"; workspaces: Schemas["WorkspaceResponse"][] };

const button = "caps fillup fillup-ink h-12 bg-silver px-8 text-ink disabled:cursor-wait disabled:opacity-70";

/** The workspaces a person is in, and a way to start another. */
export function Teams() {
  const router = useRouter();
  const [state, setState] = useState<State>({ phase: "loading" });
  const [name, setName] = useState("");
  const [problem, setProblem] = useState<string | undefined>();
  const [alert, setAlert] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  const load = useCallback(async () => {
    setState({ phase: "loading" });
    const result = await api.workspaces.list();
    setState(result.ok ? { phase: "ready", workspaces: result.data.workspaces } : { phase: "failed", message: describeFailure(result.error) });
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function create(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    setAlert(null);
    const found = checkName(name);
    setProblem(found ?? undefined);
    if (found) return focusField("workspace-name");
    setPending(true);
    const result = await api.workspaces.create(name);
    if (result.ok) {
      router.push(`/team/${result.data.workspace.id}`);
      return;
    }
    setPending(false);
    if (result.error.fields.name) {
      setProblem(result.error.fields.name);
      focusField("workspace-name");
    } else {
      setAlert(describeFailure(result.error));
    }
  }

  return (
    <>
      <p className="label">Teams</p>
      <h1 className="display mt-6 text-[clamp(2.4rem,6vw,4.5rem)]">
        <span className="block">Your</span>
        <span className="block">
          <em className="metal">workspaces.</em>
        </span>
      </h1>

      <section aria-labelledby="yours" className="mt-12 border border-line bg-panel/60 p-6 sm:p-8">
        <h2 id="yours" className="label text-paper">
          The ones you are in
        </h2>
        <div className="mt-6">
          {state.phase === "loading" ? (
            <div role="status">
              <span className="sr-only">Loading your workspaces</span>
              <div aria-hidden="true" className="h-5 w-2/5 bg-raise" />
            </div>
          ) : null}
          {state.phase === "failed" ? (
            <div className="space-y-4">
              <Notice tone="error">{state.message}</Notice>
              <button type="button" onClick={() => void load()} className={button}>
                Try again
              </button>
            </div>
          ) : null}
          {state.phase === "ready" && state.workspaces.length === 0 ? (
            <p className="text-[0.95rem] text-mute">You are not in a workspace yet. Start one below, or open an invitation you were sent.</p>
          ) : null}
          {state.phase === "ready" && state.workspaces.length > 0 ? (
            <ul className="border-t border-line">
              {state.workspaces.map((workspace) => (
                <li key={workspace.id} className="flex flex-wrap items-baseline justify-between gap-3 border-b border-line py-4">
                  <Link href={`/team/${workspace.id}`} className="min-w-0 text-[1.05rem] text-paper underline decoration-line-hi underline-offset-4 hover:decoration-silver [overflow-wrap:anywhere]">
                    {workspace.name}
                  </Link>
                  <span className="label">
                    {ROLE_LABEL[workspace.role]} · {workspace.member_count === 1 ? "1 person" : `${workspace.member_count} people`}
                  </span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </section>

      <section aria-labelledby="start" className="mt-6 border border-line bg-panel/60 p-6 sm:p-8">
        <h2 id="start" className="label text-paper">
          Start one
        </h2>
        <p className="mt-3 max-w-prose text-[0.9rem] leading-relaxed text-mute">You will be its owner. You can invite people by email once it exists. An account may own five.</p>
        <form onSubmit={create} noValidate className="mt-6 max-w-sm space-y-5">
          {alert ? <Notice tone="error">{alert}</Notice> : null}
          <TextField id="workspace-name" label="Name" value={name} onChange={setName} error={problem} maxLength={80} hint="For example, the class or the team." />
          <button type="submit" disabled={pending} aria-busy={pending} className={button}>
            {pending ? "One moment…" : "Start workspace"}
          </button>
        </form>
      </section>
    </>
  );
}
