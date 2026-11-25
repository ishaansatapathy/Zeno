-- Segment keys that change as a video plays (Day 30).
--
-- `rotation_secs` is how many seconds of video one key covers. NULL means the video was made before keys
-- were rotated: its segments are locked with the content key itself, and its key is served whole. A value
-- means the segments of every `rotation_secs` of video are locked with a key derived from the content key
-- (zeno_core::segment_keys), so the key server can stop handing them out when a viewing ends.
--
-- Rolling deploy: nothing here changes a video that exists. The worker records the value in the same
-- statement that stores a new video's key; the key server reads it. A key server that does not know the
-- column yet would serve the whole key for a video made after the worker was updated, which would not
-- decrypt it, so deploy the key server (and the API) before the worker.

alter table video_keys
    add column rotation_secs integer
        check (rotation_secs is null or rotation_secs between 6 and 3600);
