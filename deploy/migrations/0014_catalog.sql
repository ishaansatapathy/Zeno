-- Public catalog: creator profiles and course search. Expand-only.
create extension if not exists pg_trgm;

-- What visitors see of a creator. A creator without a profile can still sell; the public page
-- then shows their account name and no handle.
create table creator_profiles (
    user_id uuid primary key references users(id) on delete cascade,
    handle text not null,
    display_name text not null,
    bio text not null default '',
    avatar_key text,
    links jsonb not null default '[]',
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    constraint creator_profiles_handle_format check (
        char_length(handle) between 3 and 30
        and handle ~ '^[a-z0-9][a-z0-9-]*[a-z0-9]$'
        and handle !~ '--'
    ),
    constraint creator_profiles_display_name_length check (
        char_length(display_name) between 2 and 60
    ),
    constraint creator_profiles_bio_length check (char_length(bio) <= 500),
    constraint creator_profiles_links_shape check (
        jsonb_typeof(links) = 'array' and jsonb_array_length(links) <= 5
    )
);

create unique index creator_profiles_handle_unique on creator_profiles (handle);

-- Weighted full-text vector: title matters most, then the short blurb, then the long text.
-- The 'simple' configuration does no stemming, which is what mixed Hindi and English text needs.
alter table courses add column search_vector tsvector generated always as (
    setweight(to_tsvector('simple', coalesce(title, '')), 'A')
    || setweight(to_tsvector('simple', coalesce(short_description, '')), 'B')
    || setweight(to_tsvector('simple', coalesce(description, '')), 'C')
) stored;

-- The catalog only ever reads published, undeleted courses, so the indexes cover only those.
create index courses_search_idx on courses using gin (search_vector)
    where status = 'published' and deleted_at is null;
create index courses_catalog_idx on courses (published_at desc, id desc)
    where status = 'published' and deleted_at is null;
create index courses_creator_public_idx on courses (creator_id, published_at desc, id desc)
    where status = 'published' and deleted_at is null;
