-- The hourly sweep removes recordings that were deleted long ago. Without this it would look at
-- every recording to find them; with it, it looks only at the deleted ones, oldest first.
--
-- Expand-only. The index is partial, so it costs nothing for the live recordings that are almost
-- all of the table.
create index recordings_deleted_idx on recordings (deleted_at) where deleted_at is not null;
