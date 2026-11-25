-- How much a person may keep (Day 31).
--
-- What counts is the size of each video the person has uploaded and still has: the original, as it was
-- sent. NULL means the account has the default (DEFAULT_STORAGE_QUOTA_BYTES); a number is that account's own,
-- which an operator sets by hand for now. Deleted and failed videos do not count; one still uploading does
-- (its size is only known when it ends, so a recording being streamed counts from its parts, see the API).
--
-- Expand-only: an old binary ignores the column and enforces nothing.
alter table users
    add column storage_quota_bytes bigint
        check (storage_quota_bytes is null or storage_quota_bytes >= 0);

-- Adding up a person's videos reads this index only, however many they have.
create index videos_owner_usage on videos (owner_id) include (size_bytes)
    where deleted_at is null and status <> 'failed';
