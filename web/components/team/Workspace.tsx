"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";

import { Notice, TextField, focusField } from "@/components/auth/controls";
import { describeFailure } from "@/components/auth/messages";
import { checkEmail, checkName } from "@/components/auth/validate";
import { api, type ApiFailure, type Schemas } from "@/lib/api/client";
import { ROLE_LABEL, canManage, canRemove, inviteRoles, isOwner, type Role } from "@/lib/team";

type Workspace = Schemas["WorkspaceResponse"];
type Member = Schemas["MemberResponse"];
type Invite = Schemas["InviteResponse"];

type State =
  | { phase: "loading" }
  | { phase: "missing" }
  | { phase: "failed"; message: string }
  | { phase: "ready"; workspace: Workspace; members: Member[]; invites: Invite[] };

const button = "caps fillup fillup-ink h-12 bg-silver px-8 text-ink disabled:cursor-wait disabled:opacity-70";
const quiet = "caps h-10 border border-line-hi px-4 text-paper/80 transition-colors hover:bg-raise hover:text-paper disabled:cursor-wait disabled:opacity-60";

function Panel({ id, title, intro, children }: { id: string; title: string; intro?: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="border border-line bg-panel/60 p-6 sm:p-8">
      <h2 id={id} className="label text-paper">
        {title}
      </h2>
      {intro ? <p className="mt-3 max-w-prose text-[0.9rem] leading-relaxed text-mute">{intro}</p> : null}
      <div className="mt-6">{children}</div>
    </section>
  );
}

const day = (iso: string) => new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });

/** One workspace: its people, what each is, who is invited, and (for those who may) the changes. */
export function WorkspaceView({ id }: { id: string }) {
  const router = useRouter();
  const [state, setState] = useState<State>({ phase: "loading" });
  const [message, setMessage] = useState("");
  const [alert, setAlert] = useState<string | null>(null);
  const clear = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => void (clear.current && clearTimeout(clear.current)), []);

  const say = useCallback((text: string) => {
    setAlert(null);
    setMessage(text);
    if (clear.current) clearTimeout(clear.current);
    clear.current = setTimeout(() => setMessage(""), 6000);
  }, []);

  const load = useCallback(async () => {
    const detail = await api.workspaces.get(id);
    if (!detail.ok) {
      setState(detail.error.status === 404 ? { phase: "missing" } : { phase: "failed", message: describeFailure(detail.error) });
      return;
    }
    const { workspace, members } = detail.data;
    let invites: Invite[] = [];
    if (canManage(workspace.role)) {
      const listed = await api.workspaces.invites(id);
      if (listed.ok) invites = listed.data.invites;
    }
    setState({ phase: "ready", workspace, members, invites });
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  /** Runs a change; on success says so and reloads, on failure says why. */
  async function change(run: () => Promise<{ ok: true } | { ok: false; error: ApiFailure }>, done: string) {
    setAlert(null);
    const result = await run();
    if (result.ok) {
      say(done);
      await load();
      return true;
    }
    setAlert(describeFailure(result.error));
    return false;
  }

  if (state.phase === "loading") {
    return (
      <div role="status">
        <span className="sr-only">Loading the workspace</span>
        <div aria-hidden="true" className="h-8 w-1/3 bg-raise" />
      </div>
    );
  }
  if (state.phase === "missing") {
    return (
      <div className="space-y-4">
        <h1 className="display text-[clamp(2rem,5vw,3.5rem)]">Not found.</h1>
        <p className="max-w-prose text-[0.95rem] text-mute">This workspace does not exist, or you are not in it. If you were sent an invitation, open its link.</p>
        <Link href="/team" className="text-paper underline decoration-line-hi underline-offset-4 hover:decoration-silver">
          Your workspaces
        </Link>
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

  const { workspace, members, invites } = state;
  const me = workspace.role;

  return (
    <>
      <p className="label">
        <Link href="/team" className="underline decoration-line-hi underline-offset-4 hover:decoration-silver">
          Workspaces
        </Link>{" "}
        · You are {ROLE_LABEL[me].toLowerCase()}
      </p>
      <h1 className="display mt-6 text-[clamp(2.2rem,5.5vw,4rem)] [overflow-wrap:anywhere]">{workspace.name}</h1>
      <p role="status" aria-live="polite" className="label mt-4 min-h-5 text-paper">
        {message}
      </p>
      {alert ? (
        <div className="mt-4">
          <Notice tone="error">{alert}</Notice>
        </div>
      ) : null}

      <div className="mt-8 space-y-6">
        <Panel id="people" title={`People · ${members.length}`}>
          <ul className="border-t border-line">
            {members.map((member) => (
              <li key={member.user_id} className="flex flex-wrap items-center justify-between gap-3 border-b border-line py-4">
                <div className="min-w-0">
                  <p className="truncate text-[1rem] text-paper">
                    {member.name}
                    {member.you ? <span className="label ml-2">You</span> : null}
                  </p>
                  {member.email ? <p className="truncate text-[0.8125rem] text-mute">{member.email}</p> : null}
                </div>
                <div className="flex flex-wrap items-center gap-3">
                  {isOwner(me) ? (
                    <select
                      aria-label={`Role of ${member.name}`}
                      value={member.role}
                      onChange={(event) => void change(() => api.workspaces.setRole(id, member.user_id, event.target.value as Role), `${member.name} is now ${ROLE_LABEL[event.target.value as Role].toLowerCase()}.`)}
                      className="h-10 border border-line-hi bg-ink px-3 text-[0.9rem] text-paper"
                    >
                      {(["owner", "admin", "member"] as const).map((role) => (
                        <option key={role} value={role}>
                          {ROLE_LABEL[role]}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <span className="label">{ROLE_LABEL[member.role]}</span>
                  )}
                  {member.you ? (
                    <button
                      type="button"
                      className={quiet}
                      onClick={async () => {
                        if (await change(() => api.workspaces.removeMember(id, member.user_id), "You left.")) router.replace("/team");
                      }}
                    >
                      Leave
                    </button>
                  ) : canRemove(me, member.role) ? (
                    <button
                      type="button"
                      aria-label={`Remove ${member.name}`}
                      className={quiet}
                      onClick={() => void change(() => api.workspaces.removeMember(id, member.user_id), `${member.name} was removed.`)}
                    >
                      Remove
                    </button>
                  ) : null}
                </div>
              </li>
            ))}
          </ul>
        </Panel>

        {canManage(me) ? <Invitations id={id} role={me} invites={invites} change={change} say={say} reload={load} /> : null}
        {canManage(me) ? <Rename id={id} current={workspace.name} change={change} /> : null}
        {isOwner(me) ? <Danger id={id} name={workspace.name} /> : null}
      </div>
    </>
  );
}

function Invitations({
  id,
  role,
  invites,
  change,
  say,
  reload,
}: {
  id: string;
  role: Role;
  invites: Invite[];
  change: (run: () => Promise<{ ok: true } | { ok: false; error: ApiFailure }>, done: string) => Promise<boolean>;
  say: (text: string) => void;
  reload: () => Promise<void>;
}) {
  const roles = inviteRoles(role);
  const [email, setEmail] = useState("");
  const [as, setAs] = useState<Role>(roles[0] ?? "member");
  const [problem, setProblem] = useState<string | undefined>();
  const [alert, setAlert] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function send(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    setAlert(null);
    const found = checkEmail(email);
    setProblem(found ?? undefined);
    if (found) return focusField("invite-email");
    setPending(true);
    const result = await api.workspaces.invite(id, email.trim(), as);
    setPending(false);
    if (result.ok) {
      say(`Invitation sent to ${result.data.invite.email}.`);
      setEmail("");
      await reload();
      return;
    }
    if (result.error.fields.email) {
      setProblem(result.error.fields.email);
      focusField("invite-email");
    } else {
      setAlert(describeFailure(result.error));
    }
  }

  return (
    <Panel id="invitations" title="Invite people" intro="They get an email with a link that works for seven days, and only for the account that has that address.">
      <form onSubmit={send} noValidate className="max-w-sm space-y-5">
        {alert ? <Notice tone="error">{alert}</Notice> : null}
        <TextField id="invite-email" label="Email address" type="email" value={email} onChange={setEmail} error={problem} autoComplete="off" />
        {roles.length > 1 ? (
          <div>
            <label htmlFor="invite-role" className="label mb-2 block">
              As
            </label>
            <select id="invite-role" value={as} onChange={(event) => setAs(event.target.value as Role)} className="h-12 w-full border border-line-hi bg-ink px-4 text-[0.95rem] text-paper">
              {roles.map((option) => (
                <option key={option} value={option}>
                  {ROLE_LABEL[option]}
                </option>
              ))}
            </select>
          </div>
        ) : null}
        <button type="submit" disabled={pending} aria-busy={pending} className={button}>
          {pending ? "One moment…" : "Send invitation"}
        </button>
      </form>

      {invites.length > 0 ? (
        <div className="mt-8">
          <h3 className="label">Waiting for an answer</h3>
          <ul className="mt-3 border-t border-line">
            {invites.map((invite) => (
              <li key={invite.id} className="flex flex-wrap items-center justify-between gap-3 border-b border-line py-3">
                <div className="min-w-0">
                  <p className="truncate text-[0.95rem] text-paper">{invite.email}</p>
                  <p className="text-[0.8125rem] text-mute">
                    {ROLE_LABEL[invite.role]} · until {day(invite.expires_at)}
                  </p>
                </div>
                <button
                  type="button"
                  aria-label={`Revoke invitation to ${invite.email}`}
                  className={quiet}
                  onClick={() => void change(() => api.workspaces.revokeInvite(id, invite.id), `The invitation to ${invite.email} was taken back.`)}
                >
                  Revoke
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </Panel>
  );
}

function Rename({ id, current, change }: { id: string; current: string; change: (run: () => Promise<{ ok: true } | { ok: false; error: ApiFailure }>, done: string) => Promise<boolean> }) {
  const [name, setName] = useState(current);
  const [problem, setProblem] = useState<string | undefined>();
  const [pending, setPending] = useState(false);

  async function save(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    const found = checkName(name);
    setProblem(found ?? undefined);
    if (found) return focusField("workspace-rename");
    if (name.trim() === current) return;
    setPending(true);
    await change(() => api.workspaces.rename(id, name), "Renamed.");
    setPending(false);
  }

  return (
    <Panel id="rename" title="Name">
      <form onSubmit={save} noValidate className="max-w-sm space-y-5">
        <TextField id="workspace-rename" label="Workspace name" value={name} onChange={setName} error={problem} maxLength={80} />
        <button type="submit" disabled={pending} aria-busy={pending} className={button}>
          {pending ? "One moment…" : "Save name"}
        </button>
      </form>
    </Panel>
  );
}

/** Ending it takes two presses, and the second says what it does. */
function Danger({ id, name }: { id: string; name: string }) {
  const router = useRouter();
  const [asking, setAsking] = useState(false);
  const [pending, setPending] = useState(false);
  const [alert, setAlert] = useState<string | null>(null);

  async function end() {
    setPending(true);
    const result = await api.workspaces.remove(id);
    if (result.ok) {
      router.replace("/team");
      return;
    }
    setPending(false);
    setAlert(describeFailure(result.error));
  }

  return (
    <Panel id="end" title="End the workspace" intro="Everyone in it loses access at once and its open invitations stop working. What people made on their own is not touched.">
      {alert ? <Notice tone="error">{alert}</Notice> : null}
      {asking ? (
        <div role="group" aria-label={`Delete ${name}?`} className="flex flex-wrap items-center gap-3">
          <p className="min-w-0 text-[0.9rem] text-paper [overflow-wrap:anywhere]">Delete “{name}” for everyone?</p>
          <button type="button" disabled={pending} aria-label={`Delete for everyone: ${name}`} onClick={() => void end()} className={quiet}>
            Delete for everyone
          </button>
          <button type="button" disabled={pending} onClick={() => setAsking(false)} className={quiet}>
            Keep it
          </button>
        </div>
      ) : (
        <button type="button" aria-label={`Delete workspace ${name}`} onClick={() => setAsking(true)} className={quiet}>
          Delete workspace
        </button>
      )}
    </Panel>
  );
}
