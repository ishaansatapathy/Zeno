mod common;

use std::net::TcpListener;
use std::time::{Duration, Instant};

use common::*;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use zeno_core::jobs;
use zeno_worker::error::JobError;
use zeno_worker::{pipeline, runner};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

/// Runs the job that is due, which makes the first rendition: the video can be watched after it.
async fn run_first(ctx: &pipeline::Context) {
    let handled = runner::run_once(ctx, &CancellationToken::new())
        .await
        .unwrap();
    assert!(handled, "there should have been a job to run");
}

/// Runs everything that is due until nothing is: the first rendition, and then the rest of the ladder.
async fn run(ctx: &pipeline::Context) {
    run_first(ctx).await;
    while runner::run_once(ctx, &CancellationToken::new())
        .await
        .unwrap()
    {}
}

fn looks_like_plain_mpegts(bytes: &[u8]) -> bool {
    bytes.len() > 400 && bytes[0] == 0x47 && bytes[188] == 0x47 && bytes[376] == 0x47
}

db_test! {
    async fn a_720p_upload_becomes_encrypted_hls_that_plays_back_with_the_stored_key(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;

        run(&ctx).await;

        let video = video_state(&pool, id).await;
        assert_eq!(video.status, "ready", "{:?}", video.error_code);
        assert_eq!((video.width, video.height), (Some(1280), Some(720)));
        assert_eq!(video.duration, Some(3));
        assert_eq!(rendition_names(&video), ["144p", "240p", "360p", "480p", "720p", "audio"]);
        assert!(job_row(&pool).await.1, "the job is marked complete");
        assert!(dir_is_empty(&ctx.config.work_dir), "the scratch directory is cleaned up");

        let master = String::from_utf8(ctx.store.get_bytes(&format!("videos/{id}/master.m3u8"), 1 << 16).await.unwrap()).unwrap();
        let variants: Vec<&str> = master.lines().filter(|l| l.ends_with("/index.m3u8")).collect();
        assert_eq!(variants, ["144p/index.m3u8", "240p/index.m3u8", "360p/index.m3u8", "480p/index.m3u8", "720p/index.m3u8", "audio/index.m3u8"]);
        assert!(master.contains("RESOLUTION=1280x720"));
        assert!(master.contains("RESOLUTION=256x144"));
        assert!(master.lines().any(|l| l.contains("CODECS=\"mp4a.40.2\"") && !l.contains("RESOLUTION")), "audio-only variant is advertised");

        let key = stored_key(&pool, id).await;
        assert_eq!(key.len(), 16);
        let play = scratch_dir();
        for variant in ["144p", "720p", "audio"] {
            let playlist = ctx.store.get_bytes(&format!("videos/{id}/{variant}/index.m3u8"), 1 << 20).await.unwrap();
            let text = String::from_utf8(playlist).unwrap();
            assert!(text.contains("#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video\""), "{variant}: {text}");
            assert!(text.contains("#EXT-X-PLAYLIST-TYPE:VOD") && text.contains("#EXT-X-ENDLIST"));

            let first_segment = ctx.store.get_bytes(&format!("videos/{id}/{variant}/seg_00000.ts"), 64 << 20).await.unwrap();
            assert!(!looks_like_plain_mpegts(&first_segment), "{variant}: segments must be encrypted at rest");

            let local = fetch_variant(&ctx, id, variant, &key, play.path()).await;
            assert!(decodes_cleanly(&local), "{variant} must decrypt and decode with the stored key");
            let probe = probe_json(&local);
            match variant {
                "audio" => {
                    assert!(stream(&probe, "video").is_none());
                    assert_eq!(stream(&probe, "audio").unwrap()["codec_name"], "aac");
                }
                other => {
                    let height: i64 = other.trim_end_matches('p').parse().unwrap();
                    let v = stream(&probe, "video").unwrap();
                    assert_eq!((v["codec_name"].as_str(), v["height"].as_i64()), (Some("h264"), Some(height)));
                    assert_eq!(stream(&probe, "audio").unwrap()["codec_name"], "aac");
                }
            }
        }

        let wrong = scratch_dir();
        let local = fetch_variant(&ctx, id, "360p", &[0u8; 16], wrong.path()).await;
        assert!(!decodes_cleanly(&local), "a wrong key must not decrypt the segments");

        for never in ["key.bin", "keyinfo.txt", "source"] {
            assert_eq!(ctx.store.head(&format!("videos/{id}/{never}")).await.unwrap(), None, "{never} must never be published");
        }
    }
}

db_test! {
    async fn renditions_are_aligned_on_six_second_segments(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let spec = Spec { width: 640, height: 360, secs: 14, audio: true, fps: 25 };
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &spec)).await;
        run(&ctx).await;
        assert_eq!(video_state(&pool, id).await.status, "ready");

        let mut counts = vec![];
        for variant in ["144p", "240p", "360p"] {
            let playlist = String::from_utf8(ctx.store.get_bytes(&format!("videos/{id}/{variant}/index.m3u8"), 1 << 20).await.unwrap()).unwrap();
            let durations: Vec<f64> = playlist
                .lines()
                .filter_map(|l| l.strip_prefix("#EXTINF:"))
                .map(|l| l.trim_end_matches(',').parse().unwrap())
                .collect();
            assert!(durations[0] > 5.9 && durations[0] < 6.1, "{variant}: {durations:?}");
            counts.push(durations.len());
        }
        assert_eq!(counts, [3, 3, 3], "every rendition cuts at the same boundaries");
    }
}

db_test! {
    async fn small_sources_are_never_upscaled(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let spec = Spec { width: 320, height: 180, ..Spec::default() };
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &spec)).await;
        run(&ctx).await;
        let video = video_state(&pool, id).await;
        assert_eq!(video.status, "ready");
        assert_eq!(rendition_names(&video), ["144p", "audio"]);
    }
}

db_test! {
    async fn a_video_without_sound_gets_no_audio_variant(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let spec = Spec { width: 640, height: 360, audio: false, ..Spec::default() };
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &spec)).await;
        run(&ctx).await;
        let video = video_state(&pool, id).await;
        assert_eq!(video.status, "ready");
        assert_eq!(rendition_names(&video), ["144p", "240p", "360p"]);
        let master = String::from_utf8(ctx.store.get_bytes(&format!("videos/{id}/master.m3u8"), 1 << 16).await.unwrap()).unwrap();
        assert!(!master.contains("audio/index.m3u8") && master.contains("CODECS=\"avc1.4d401f\""));
        let local = fetch_variant(&ctx, id, "240p", &stored_key(&pool, id).await, scratch.path()).await;
        assert!(stream(&probe_json(&local), "audio").is_none());
        assert!(decodes_cleanly(&local));
    }
}

db_test! {
    async fn portrait_phone_video_is_laddered_by_its_displayed_size(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let spec = Spec { width: 640, height: 360, ..Spec::default() };
        let id = seed(&ctx, make_rotated(scratch.path(), "portrait.mp4", &spec, 90)).await;
        run(&ctx).await;
        let video = video_state(&pool, id).await;
        assert_eq!(video.status, "ready", "{:?}", video.error_code);
        assert_eq!((video.width, video.height), (Some(360), Some(640)), "stored 640x360 rotated is shown as 360x640");
        assert_eq!(rendition_names(&video), ["144p", "240p", "360p", "480p", "audio"]);

        let local = fetch_variant(&ctx, id, "480p", &stored_key(&pool, id).await, scratch.path()).await;
        let probe = probe_json(&local);
        let v = stream(&probe, "video").unwrap();
        assert_eq!((v["width"].as_i64(), v["height"].as_i64()), (Some(270), Some(480)), "a 9:16 picture at 480p is 270 wide: portrait stays portrait");
    }
}

db_test! {
    async fn high_frame_rate_sources_are_capped_at_thirty(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let spec = Spec { width: 640, height: 360, fps: 60, ..Spec::default() };
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &spec)).await;
        run(&ctx).await;
        assert_eq!(video_state(&pool, id).await.status, "ready");
        let local = fetch_variant(&ctx, id, "360p", &stored_key(&pool, id).await, scratch.path()).await;
        let rate = stream(&probe_json(&local), "video").unwrap()["avg_frame_rate"].as_str().unwrap().to_owned();
        assert_eq!(rate, "30/1");
    }
}

db_test! {
    async fn audio_only_and_garbage_uploads_are_rejected_for_good_without_retries(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();

        let podcast = seed(&ctx, make_audio_only(scratch.path(), "pod.m4a")).await;
        run(&ctx).await;
        let video = video_state(&pool, podcast).await;
        assert_eq!((video.status.as_str(), video.error_code.as_deref()), ("failed", Some("no_video_stream")));
        let (attempts, completed, failed, _) = job_row(&pool).await;
        assert_eq!((attempts, completed, failed), (1, false, true), "a bad file is never retried");

        let noise: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        let junk = seed(&ctx, noise).await;
        run(&ctx).await;
        let video = video_state(&pool, junk).await;
        assert_eq!((video.status.as_str(), video.error_code.as_deref()), ("failed", Some("unsupported_media")));
    }
}

db_test! {
    async fn playlists_and_scripts_that_point_elsewhere_are_refused_and_never_followed(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        // A listener the worker must never connect to.
        let trap = TcpListener::bind("127.0.0.1:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        let port = trap.local_addr().unwrap().port();

        let hls = format!("#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:5,\nhttp://127.0.0.1:{port}/secret.ts\n#EXT-X-ENDLIST\n");
        let concat = "ffconcat version 1.0\nfile '/etc/hosts'\nfile 'C:/Windows/win.ini'\n";
        let sdp = format!("v=0\no=- 0 0 IN IP4 127.0.0.1\ns=x\nc=IN IP4 127.0.0.1\nt=0 0\nm=video {port} RTP/AVP 96\n");
        for evil in [hls, concat.to_owned(), sdp] {
            let id = seed(&ctx, evil.clone().into_bytes()).await;
            run(&ctx).await;
            let video = video_state(&pool, id).await;
            assert_eq!(video.status, "failed", "{evil}");
            assert!(matches!(video.error_code.as_deref(), Some("unsupported_media")), "{evil}: {:?}", video.error_code);
        }
        assert!(trap.accept().is_err(), "the worker must never open a connection named by an upload");
    }
}

db_test! {
    async fn a_perfectly_good_video_in_an_unlisted_container_is_still_refused(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        // The same bytes in an accepted container transcode fine; the container is what is judged.
        let accepted = seed(&ctx, make_in_container(scratch.path(), "ok.mkv", "matroska")).await;
        let refused = seed(&ctx, make_in_container(scratch.path(), "no.nut", "nut")).await;
        run(&ctx).await;
        assert_eq!(video_state(&pool, accepted).await.status, "ready");
        let video = video_state(&pool, refused).await;
        assert_eq!((video.status.as_str(), video.error_code.as_deref()), ("failed", Some("unsupported_media")));
    }
}

db_test! {
    async fn videos_over_the_duration_or_resolution_limits_are_rejected(pool: PgPool) {
        let scratch = scratch_dir();
        let source = make_video(scratch.path(), "in.mp4", &Spec::default());

        let short = context_with(pool.clone(), |c| c.limits.max_duration_secs = 2.0).await;
        let id = seed(&short, source.clone()).await;
        run(&short).await;
        assert_eq!(video_state(&pool, id).await.error_code.as_deref(), Some("too_long"));

        let small = context_with(pool.clone(), |c| c.limits.max_dimension = 640).await;
        let id = seed(&small, source).await;
        run(&small).await;
        assert_eq!(video_state(&pool, id).await.error_code.as_deref(), Some("resolution_too_high"));
    }
}

db_test! {
    async fn a_video_deleted_while_queued_is_a_quiet_no_op(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;
        sqlx::query("delete from videos where id = $1").bind(id).execute(&pool).await.unwrap();

        run(&ctx).await;

        assert!(job_row(&pool).await.1, "the job is closed, not retried forever");
        assert_eq!(ctx.store.head(&format!("videos/{id}/master.m3u8")).await.unwrap(), None);
    }
}

db_test! {
    async fn a_storage_outage_is_retried_and_only_fails_the_video_after_the_last_attempt(pool: PgPool) {
        let healthy = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&healthy, make_video(scratch.path(), "in.mp4", &Spec::default())).await;

        let broken = broken_storage_context(pool.clone());
        run(&broken).await;

        let video = video_state(&pool, id).await;
        assert_eq!((video.status.as_str(), video.error_code.as_deref()), ("processing", None));
        let (attempts, completed, failed, error) = job_row(&pool).await;
        assert_eq!((attempts, completed, failed), (1, false, false));
        assert!(error.unwrap().contains("storage"));

        // The retry is scheduled for later; once it is due the healthy worker finishes the job.
        sqlx::query("update jobs set run_at = now()").execute(&pool).await.unwrap();
        run(&healthy).await;
        assert_eq!(video_state(&pool, id).await.status, "ready");
    }
}

db_test! {
    async fn when_every_attempt_hits_an_outage_the_video_is_marked_failed(pool: PgPool) {
        let healthy = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&healthy, make_video(scratch.path(), "in.mp4", &Spec::default())).await;
        sqlx::query("update jobs set max_attempts = 1").execute(&pool).await.unwrap();

        let broken = broken_storage_context(pool.clone());
        run(&broken).await;

        let video = video_state(&pool, id).await;
        assert_eq!((video.status.as_str(), video.error_code.as_deref()), ("failed", Some("processing_failed")));
        assert!(job_row(&pool).await.2);
    }
}

db_test! {
    async fn an_encoder_that_keeps_failing_is_retried_twice_then_abandoned(pool: PgPool) {
        let ctx = context_with(pool.clone(), |c| c.encode_timeout = Some(Duration::from_millis(1))).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;

        for expected_attempt in 1..=2 {
            run(&ctx).await;
            let video = video_state(&pool, id).await;
            assert_eq!(video.status, "processing", "attempt {expected_attempt} is retried");
            assert_eq!(job_row(&pool).await.0, expected_attempt);
            sqlx::query("update jobs set run_at = now()").execute(&pool).await.unwrap();
        }
        run(&ctx).await;
        let video = video_state(&pool, id).await;
        assert_eq!((video.status.as_str(), video.error_code.as_deref()), ("failed", Some("processing_failed")));
        assert!(dir_is_empty(&ctx.config.work_dir));
    }
}

db_test! {
    async fn a_job_that_keeps_killing_its_worker_is_eventually_failed_by_the_next_one(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;
        sqlx::query("update jobs set max_attempts = 2").execute(&pool).await.unwrap();

        // Two workers claim it and die without reporting; each lease then lapses.
        for _ in 0..2 {
            jobs::claim(&pool, &["transcode"], Duration::from_secs(60)).await.unwrap().unwrap();
            sqlx::query("update jobs set locked_until = now() - interval '1 second'").execute(&pool).await.unwrap();
        }
        run(&ctx).await;

        let video = video_state(&pool, id).await;
        assert_eq!((video.status.as_str(), video.error_code.as_deref()), ("failed", Some("processing_failed")));
        assert!(job_row(&pool).await.2);
    }
}

db_test! {
    async fn reprocessing_publishes_a_new_key_and_everything_still_decrypts(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec { width: 640, height: 360, ..Spec::default() })).await;
        run(&ctx).await;
        let first_key = stored_key(&pool, id).await;

        sqlx::query("update videos set status = 'processing', ready_at = null where id = $1").bind(id).execute(&pool).await.unwrap();
        jobs::enqueue(&pool, "transcode", &serde_json::json!({ "video_id": id })).await.unwrap().unwrap();
        run(&ctx).await;

        let second_key = stored_key(&pool, id).await;
        assert_ne!(first_key, second_key, "each encode uses a fresh key");
        let play = scratch_dir();
        for variant in ["144p", "360p", "audio"] {
            let local = fetch_variant(&ctx, id, variant, &second_key, play.path()).await;
            assert!(decodes_cleanly(&local), "{variant} decrypts with the new key");
        }
        let stale = scratch_dir();
        let local = fetch_variant(&ctx, id, "144p", &first_key, stale.path()).await;
        assert!(!decodes_cleanly(&local), "the previous key no longer opens anything");
    }
}

db_test! {
    async fn cancelling_a_job_kills_ffmpeg_promptly_and_leaves_the_job_for_someone_else(pool: PgPool) {
        let ctx = context_with(pool.clone(), |c| c.encode_threads = 1).await;
        let scratch = scratch_dir();
        let spec = Spec { width: 1280, height: 720, secs: 90, audio: true, fps: 25 };
        let id = seed(&ctx, make_video(scratch.path(), "long.mp4", &spec)).await;
        let job = jobs::claim(&pool, &["transcode"], Duration::from_secs(120)).await.unwrap().unwrap();

        let cancel = CancellationToken::new();
        let canceller = {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                cancel.cancel();
            })
        };
        let started = Instant::now();
        let result = pipeline::process(&ctx, &job, &cancel).await;
        canceller.await.unwrap();

        assert!(matches!(result, Err(JobError::Cancelled)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(20), "cancellation must not wait for the encode to finish");
        assert_eq!(video_state(&pool, id).await.status, "processing");
        assert!(!job_row(&pool).await.1);
        assert_eq!(ctx.store.head(&format!("videos/{id}/master.m3u8")).await.unwrap(), None, "nothing was published");
    }
}

db_test! {
    async fn a_video_marked_deleted_while_queued_is_never_downloaded_or_encoded(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;
        sqlx::query("update videos set deleted_at = now() where id = $1").bind(id).execute(&pool).await.unwrap();

        run(&ctx).await;

        assert!(job_row(&pool).await.1, "the job is closed, not retried forever");
        assert_eq!(video_state(&pool, id).await.status, "processing");
        assert_eq!(ctx.store.list_prefix(&format!("videos/{id}/")).await.unwrap().len(), 0);
    }
}

db_test! {
    async fn a_video_deleted_during_the_encode_is_not_marked_ready_and_gets_no_key(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;
        // Deterministically "delete" the video at the worst moment: after the worker has
        // started committing its result but before the video is marked ready.
        sqlx::raw_sql(
            "create function soft_delete_on_key() returns trigger language plpgsql as $$ \
             begin update videos set deleted_at = now() where id = new.video_id; return new; end $$; \
             create trigger soft_delete_on_key before insert on video_keys \
             for each row execute function soft_delete_on_key();",
        )
        .execute(&pool)
        .await
        .unwrap();

        run(&ctx).await;

        assert_eq!(video_state(&pool, id).await.status, "processing", "a deleted video must not become ready");
        let keys: i64 = sqlx::query_scalar("select count(*) from video_keys").fetch_one(&pool).await.unwrap();
        assert_eq!(keys, 0, "the key insert is rolled back with everything else");
        assert!(job_row(&pool).await.1, "the job still ends cleanly");
    }
}

db_test! {
    async fn the_content_key_is_stored_wrapped_and_only_opens_for_its_own_video(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;
        run(&ctx).await;

        let blob = stored_blob(&pool, id).await;
        let key = stored_key(&pool, id).await;
        assert_eq!(blob.len(), zeno_core::keyvault::WRAPPED_LEN);
        assert_eq!(key.len(), 16);
        assert!(!blob.windows(16).any(|w| w == &key[..]), "the raw key must not appear in the database");
        assert!(test_vault().open(uuid::Uuid::new_v4(), &blob).is_err(), "bound to its own video");
    }
}

// ---- first play: one rendition, then the rest ---------------------------------------------------------------

async fn live_jobs(pool: &PgPool, kind: &str) -> i64 {
    sqlx::query_scalar(
        "select count(*) from jobs where kind = $1 and completed_at is null and failed_at is null",
    )
    .bind(kind)
    .fetch_one(pool)
    .await
    .unwrap()
}

db_test! {
    async fn the_video_plays_after_the_first_rendition_while_the_rest_waits_in_its_own_job(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;

        run_first(&ctx).await;

        // Ready, with exactly one rendition: the largest up to 360p.
        let video = video_state(&pool, id).await;
        assert_eq!(video.status, "ready", "{:?}", video.error_code);
        assert_eq!(rendition_names(&video), ["360p"]);
        assert_eq!((video.width, video.height, video.duration), (Some(1280), Some(720), Some(3)));
        let has_ready_at: bool = sqlx::query_scalar("select ready_at is not null from videos where id = $1").bind(id).fetch_one(&pool).await.unwrap();
        assert!(has_ready_at);

        // The master lists only what is there, and it decrypts with the key that is stored.
        let master = String::from_utf8(ctx.store.get_bytes(&format!("videos/{id}/master.m3u8"), 1 << 16).await.unwrap()).unwrap();
        assert_eq!(master.lines().filter(|l| l.ends_with("/index.m3u8")).collect::<Vec<_>>(), ["360p/index.m3u8"]);
        let key = stored_key(&pool, id).await;
        let play = scratch_dir();
        assert!(decodes_cleanly(&fetch_variant(&ctx, id, "360p", &key, play.path()).await));
        assert_eq!(ctx.store.head(&format!("videos/{id}/720p/index.m3u8")).await.unwrap(), None, "nothing else is published yet");

        // The rest is queued, once, and the first job is done.
        assert_eq!(live_jobs(&pool, "transcode").await, 0);
        assert_eq!(live_jobs(&pool, "transcode_rest").await, 1);
        assert!(
            jobs::enqueue(&pool, "transcode", &serde_json::json!({ "video_id": id })).await.unwrap().is_none(),
            "a video has one live job at a time, of either kind"
        );

        // The rest arrives, under the same key, and the ladder is the whole ladder.
        run(&ctx).await;
        let video = video_state(&pool, id).await;
        assert_eq!(rendition_names(&video), ["144p", "240p", "360p", "480p", "720p", "audio"]);
        assert_eq!(live_jobs(&pool, "transcode_rest").await, 0);
        assert_eq!(stored_key(&pool, id).await, key, "the key did not change");
        let master = String::from_utf8(ctx.store.get_bytes(&format!("videos/{id}/master.m3u8"), 1 << 16).await.unwrap()).unwrap();
        assert_eq!(master.lines().filter(|l| l.ends_with("/index.m3u8")).count(), 6);
        for variant in ["144p", "720p", "audio"] {
            let local = fetch_variant(&ctx, id, variant, &key, play.path()).await;
            assert!(decodes_cleanly(&local), "{variant} decrypts with the key the first rendition used");
        }
        assert!(dir_is_empty(&ctx.config.work_dir), "both jobs clean up after themselves");
    }
}

db_test! {
    async fn viewers_see_a_consistent_ladder_at_every_moment_while_the_rest_is_added(pool: PgPool) {
        let ctx = context_with(pool.clone(), |c| c.encode_threads = 1).await;
        let scratch = scratch_dir();
        let spec = Spec { secs: 6, ..Spec::default() };
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &spec)).await;
        run_first(&ctx).await;

        let rest = {
            let ctx = ctx.clone();
            tokio::spawn(async move { run(&ctx).await })
        };
        // Whenever somebody looks, every rendition the master names is one the row names (the API refuses a
        // master that names more), and every playlist it names is already in storage. The master is read
        // first: the row can only have grown by the time it is read.
        let mut seen = std::collections::BTreeSet::new();
        while !rest.is_finished() {
            let master = String::from_utf8(ctx.store.get_bytes(&format!("videos/{id}/master.m3u8"), 1 << 16).await.unwrap()).unwrap();
            let named: Vec<String> = master.lines().filter_map(|l| l.strip_suffix("/index.m3u8")).map(str::to_owned).collect();
            let row = rendition_names(&video_state(&pool, id).await);
            for name in &named {
                assert!(row.contains(name), "the master names {name}, the row has {row:?}");
                assert!(ctx.store.head(&format!("videos/{id}/{name}/index.m3u8")).await.unwrap().is_some(), "{name} is named before it is there");
            }
            seen.insert(named.len());
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        rest.await.unwrap();
        assert!(seen.len() >= 3, "the ladder was seen growing, not appearing at once: {seen:?}");
        assert_eq!(rendition_names(&video_state(&pool, id).await).len(), 6);
    }
}

db_test! {
    async fn nothing_more_is_queued_for_a_video_with_nothing_more_to_make(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let spec = Spec { width: 256, height: 144, audio: false, ..Spec::default() };
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &spec)).await;
        run_first(&ctx).await;
        assert_eq!(rendition_names(&video_state(&pool, id).await), ["144p"]);
        assert_eq!(live_jobs(&pool, "transcode_rest").await, 0);
    }
}

db_test! {
    async fn a_video_with_one_picture_rung_and_sound_still_gets_its_audio_only_rendition(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let spec = Spec { width: 256, height: 144, ..Spec::default() };
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &spec)).await;
        run_first(&ctx).await;
        assert_eq!(live_jobs(&pool, "transcode_rest").await, 1);
        run(&ctx).await;
        assert_eq!(rendition_names(&video_state(&pool, id).await), ["144p", "audio"]);
    }
}

db_test! {
    async fn a_video_deleted_before_the_rest_is_made_gets_no_more_renditions(pool: PgPool) {
        let ctx = context(pool.clone()).await;
        let scratch = scratch_dir();
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &Spec::default())).await;
        run_first(&ctx).await;
        sqlx::query("update videos set deleted_at = now() where id = $1").bind(id).execute(&pool).await.unwrap();
        run(&ctx).await;
        assert_eq!(rendition_names(&video_state(&pool, id).await), ["360p"]);
        assert_eq!(live_jobs(&pool, "transcode_rest").await, 0, "the job ends quietly");
        assert_eq!(ctx.store.head(&format!("videos/{id}/720p/index.m3u8")).await.unwrap(), None);
    }
}

db_test! {
    async fn a_video_encoded_again_during_the_rest_is_not_given_renditions_under_the_old_key(pool: PgPool) {
        let ctx = context_with(pool.clone(), |c| c.encode_threads = 1).await;
        let scratch = scratch_dir();
        let spec = Spec { secs: 8, ..Spec::default() };
        let id = seed(&ctx, make_video(scratch.path(), "in.mp4", &spec)).await;
        run_first(&ctx).await;
        let old_key = stored_blob(&pool, id).await;

        let rest = {
            let ctx = ctx.clone();
            tokio::spawn(async move { run(&ctx).await })
        };
        // As soon as one more rendition is in, the video gets a new key (what encoding it again does).
        loop {
            if rendition_names(&video_state(&pool, id).await).len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let other = test_vault().seal(id, &[9u8; 16]).unwrap();
        sqlx::query("update video_keys set wrapped_key = $2 where video_id = $1").bind(id).bind(&other).execute(&pool).await.unwrap();
        rest.await.unwrap();

        assert_ne!(stored_blob(&pool, id).await, old_key);
        assert!(rendition_names(&video_state(&pool, id).await).len() < 6, "it stopped instead of adding more");
        assert_eq!(live_jobs(&pool, "transcode_rest").await, 0, "and its job is finished, not retried forever");
    }
}
