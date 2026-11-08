-- Ordered course lessons. Deleted lessons keep their original position for analytics, while
-- their live `position` moves negative so active positions remain dense and reusable.
alter table lessons
    add column title text not null default 'Untitled lesson',
    add column description text not null default '',
    add column is_free_preview boolean not null default false,
    add column deleted_at timestamptz,
    add column deleted_position integer,
    add column updated_at timestamptz not null default now();

-- Once a video is referenced by lesson history its row cannot be hard-deleted. The storage sweep
-- records completion here so it does not repeatedly delete the same already-empty prefix.
alter table videos add column storage_purged_at timestamptz;

alter table lessons
    add constraint lessons_title_length check (char_length(title) between 1 and 120),
    add constraint lessons_description_length check (char_length(description) <= 5000),
    add constraint lessons_position_state check (
        (deleted_at is null and position > 0 and deleted_position is null)
        or
        (deleted_at is not null and position < 0 and deleted_position > 0)
    ),
    add constraint lessons_course_position_unique
        unique (course_id, position) deferrable initially immediate;

create index lessons_active_order_idx
    on lessons (course_id, position)
    where deleted_at is null;
