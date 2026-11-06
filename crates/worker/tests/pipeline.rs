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

async fn run(ctx: &pipeline::Context) {
    let handled = runner::run_once(ctx, &CancellationToken::new())
        .await
        .unwrap();
    assert!(handled, "there should have been a job to run");
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
