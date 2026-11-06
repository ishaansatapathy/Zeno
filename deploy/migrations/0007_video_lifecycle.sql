-- Deleting a video is two steps: the row is marked deleted (so it stops being served at once),
-- and a sweep removes its objects and then the row, but only once no transcode job can still be
-- writing output for it.
alter table videos
    add column deleted_at timestamptz,
    add column source_purged_at timestamptz;

create index videos_deleted_idx on videos (deleted_at) where deleted_at is not null;
-- Finished videos whose original upload has not been removed from storage yet.
create index videos_source_retained_idx on videos ((coalesce(ready_at, updated_at)))
    where source_purged_at is null and status in ('ready', 'failed');
