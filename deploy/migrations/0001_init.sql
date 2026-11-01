create extension if not exists "uuid-ossp";

create type user_role as enum ('student', 'creator', 'admin');

create table users (
    id uuid primary key default uuid_generate_v4(),
    name text not null,
    phone text unique,
    email text unique,
    password_hash text not null,
    role user_role not null default 'student',
    created_at timestamptz not null default now()
);

create type course_status as enum ('draft', 'published');

create table courses (
    id uuid primary key default uuid_generate_v4(),
    creator_id uuid not null references users(id),
    title text not null,
    description text not null default '',
    price_paise bigint not null default 0,
    status course_status not null default 'draft',
    created_at timestamptz not null default now()
);

create type video_status as enum ('uploading', 'processing', 'ready', 'failed');

create table videos (
    id uuid primary key default uuid_generate_v4(),
    status video_status not null default 'uploading',
    duration_seconds integer,
    renditions jsonb not null default '[]',
    key_ref text,
    created_at timestamptz not null default now()
);

create type live_state as enum ('scheduled', 'live', 'ended');

create table live_sessions (
    id uuid primary key default uuid_generate_v4(),
    course_id uuid not null references courses(id),
    stream_key_hash text not null,
    scheduled_at timestamptz not null,
    state live_state not null default 'scheduled',
    created_at timestamptz not null default now()
);

create table lessons (
    id uuid primary key default uuid_generate_v4(),
    course_id uuid not null references courses(id),
    position integer not null,
    video_id uuid references videos(id),
    live_session_id uuid references live_sessions(id),
    created_at timestamptz not null default now(),
    constraint lesson_source_check check (
        (video_id is not null and live_session_id is null)
        or (video_id is null and live_session_id is not null)
    )
);

create type payment_status as enum ('pending', 'confirmed', 'failed', 'refunded');

create table payments (
    id uuid primary key default uuid_generate_v4(),
    provider_ref text not null unique,
    amount_paise bigint not null,
    status payment_status not null default 'pending',
    raw_event jsonb not null,
    created_at timestamptz not null default now()
);

create table enrollments (
    id uuid primary key default uuid_generate_v4(),
    user_id uuid not null references users(id),
    course_id uuid not null references courses(id),
    payment_id uuid not null references payments(id),
    expires_at timestamptz,
    created_at timestamptz not null default now(),
    unique (user_id, course_id)
);

create table playback_sessions (
    id uuid primary key default uuid_generate_v4(),
    user_id uuid not null references users(id),
    video_id uuid not null references videos(id),
    device text not null,
    ip inet not null,
    started_at timestamptz not null default now(),
    last_position_seconds integer not null default 0
);

create index on lessons (course_id);
create index on enrollments (user_id);
create index on enrollments (course_id);
create index on playback_sessions (user_id, video_id);
