-- Stage-1 course model. Expand-only: old binaries continue to see the original columns.
alter table courses
    add column slug text,
    add column currency text not null default 'INR',
    add column thumbnail_key text,
    add column published_at timestamptz,
    add column updated_at timestamptz not null default now(),
    add column deleted_at timestamptz,
    add column language text,
    add column level text not null default 'all_levels',
    add column short_description text not null default '',
    add column version integer not null default 1;

-- There is no production course data yet. The fallback only makes the migration safe on local
-- databases created before slugs existed; application-created slugs are human-readable.
update courses set slug = 'course-' || replace(id::text, '-', '') where slug is null;
alter table courses alter column slug set not null;

alter table courses
    add constraint courses_slug_format check (
        char_length(slug) between 3 and 140
        and slug = lower(slug)
        and slug ~ '^[a-z0-9]+(?:-[a-z0-9]+)*$'
    ),
    add constraint courses_currency_check check (currency = 'INR'),
    add constraint courses_price_check check (
        price_paise = 0 or price_paise between 100 and 100000000
    ),
    add constraint courses_title_length check (char_length(title) between 3 and 120),
    add constraint courses_description_length check (char_length(description) <= 5000),
    add constraint courses_short_description_length check (char_length(short_description) <= 300),
    add constraint courses_language_length check (language is null or char_length(language) between 2 and 35),
    add constraint courses_level_check check (level in ('all_levels', 'beginner', 'intermediate', 'advanced')),
    add constraint courses_version_positive check (version > 0);

create unique index courses_slug_unique on courses (slug);
create index courses_creator_list_idx
    on courses (creator_id, created_at desc, id desc)
    where deleted_at is null;
