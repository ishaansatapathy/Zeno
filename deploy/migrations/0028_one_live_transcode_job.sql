-- A video's transcoding now happens in two jobs: `transcode` makes the first rendition and marks the video
-- playable, then `transcode_rest` adds the others while people can already watch. The two must never run
-- (or be queued) at the same time for one video: a re-encode takes a new content key, and a second job
-- still writing renditions under the old one would leave a mix nobody could decrypt.
drop index jobs_one_live_transcode_per_video;
create unique index jobs_one_live_transcode_per_video
    on jobs ((payload ->> 'video_id'))
    where kind in ('transcode', 'transcode_rest') and completed_at is null and failed_at is null;
