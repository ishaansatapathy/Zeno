-- Uploads of a recording that is still being made.
--
-- A file picked from disk has a size, and the plan for its parts (and the exact length signed into each
-- part's URL) follows from it. A recording has none until it stops. So an upload can be marked
-- `streaming`: its size is unknown (0) while it is uploading, parts are asked for one at a time as
-- they fill up, and the size is stated when the recording ends, from which the same part plan is
-- derived and checked against what storage holds before anything is assembled.
--
-- Expand-only. An old binary still inserts a size above zero, which the relaxed check allows.
alter table videos add column streaming boolean not null default false;

alter table videos drop constraint videos_size_bytes_check;
alter table videos
    add constraint videos_size_known check (size_bytes > 0 or (streaming and status = 'uploading'));
