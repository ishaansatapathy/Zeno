alter table users
    add column email_verified_at timestamptz,
    add column updated_at timestamptz not null default now(),
    add constraint users_email_lowercase check (email is null or email = lower(email)),
    add constraint users_email_length check (email is null or char_length(email) <= 254);

-- One row per refresh token. Each refresh rotates: the old row gets rotated_at and a new row
-- joins the same family. Presenting an already-rotated token revokes the whole family.
create table sessions (
    id uuid primary key default uuid_generate_v4(),
    user_id uuid not null references users(id) on delete cascade,
    family_id uuid not null,
    refresh_token_hash bytea not null unique,
    user_agent text,
    ip inet,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    absolute_expires_at timestamptz not null,
    rotated_at timestamptz,
    revoked_at timestamptz
);

create index sessions_user_id_idx on sessions (user_id);
create index sessions_family_id_idx on sessions (family_id);

create type auth_token_purpose as enum ('email_verification', 'password_reset');

-- Single-use emailed tokens. Only the SHA-256 of the token is stored.
create table auth_tokens (
    id uuid primary key default uuid_generate_v4(),
    user_id uuid not null references users(id) on delete cascade,
    purpose auth_token_purpose not null,
    token_hash bytea not null unique,
    expires_at timestamptz not null,
    used_at timestamptz,
    created_at timestamptz not null default now()
);

create index auth_tokens_user_purpose_idx on auth_tokens (user_id, purpose);
