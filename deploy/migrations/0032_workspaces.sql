-- Workspaces: a team of people who work in Zeno together (Day 32).
--
-- A person belongs to any number of workspaces, each with a role: an `owner` (everything, and at least one
-- must always remain), an `admin` (looks after the members and the name) or a `member`. Joining is by
-- invitation to an email address: the invitation carries a one-time token, only the hash of which is kept,
-- and it can only be accepted by the account that has that address, verified.
--
-- This day is the people and their roles. Putting recordings in a workspace comes next, and will add its own
-- column; nothing here touches the library, the share links or playback.
--
-- Expand-only: nothing refers to these tables yet, and an old binary never meets them.
create type workspace_role as enum ('owner', 'admin', 'member');

create table workspaces (
    id uuid primary key,
    name text not null check (char_length(name) between 2 and 80),
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    -- Soft delete: the members' access ends at once, the rows are purged later.
    deleted_at timestamptz
);

create table workspace_members (
    workspace_id uuid not null references workspaces(id) on delete cascade,
    user_id uuid not null references users(id) on delete cascade,
    role workspace_role not null,
    joined_at timestamptz not null default now(),
    primary key (workspace_id, user_id)
);

-- "My workspaces" is read often: by person, not by workspace.
create index workspace_members_user_idx on workspace_members (user_id);
-- "How many owners" is asked under a lock on every change of the members, so it reads only owners.
create index workspace_members_owners_idx on workspace_members (workspace_id) where role = 'owner';

create table workspace_invites (
    id uuid primary key,
    workspace_id uuid not null references workspaces(id) on delete cascade,
    -- Lower case, as the account's own address is kept.
    email text not null check (email = lower(email) and char_length(email) <= 254),
    -- Nobody is invited as an owner: an owner makes an owner of someone who has joined.
    role workspace_role not null check (role <> 'owner'),
    -- SHA-256 of the token in the emailed link. The token itself is never stored.
    token_hash bytea not null unique,
    invited_by uuid references users(id) on delete set null,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    accepted_at timestamptz,
    revoked_at timestamptz
);

-- One invitation at a time for an address in a workspace: a second is refused until the first is revoked or spent.
create unique index workspace_invites_live_idx
    on workspace_invites (workspace_id, email)
    where accepted_at is null and revoked_at is null;
create index workspace_invites_workspace_idx on workspace_invites (workspace_id, created_at desc);
