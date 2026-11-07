//! Playback through the API against real Postgres and MinIO: starting sessions, the
//! concurrent-screen limit, per-viewer playlists, heartbeats and resume, and every way a
//! playlist request is refused.

mod common;

use axum::http::{header, Method, StatusCode};
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::playback::{PlaybackClaims, PlaybackSigner};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

const MIB: usize = 1024 * 1024;
const DURATION: i32 = 600;
const RENDITIONS: [&str; 2] = ["144p", "audio"];

fn segment_bytes(rendition: &str, n: usize) -> Vec<u8> {
    format!("encrypted segment {n} of {rendition}").into_bytes()
}

/// A video as the worker leaves it: uploaded, then `ready` with a master playlist, one media
/// playlist per rendition (with the key placeholder) and its segments in storage.
async fn ready_video(app: &App, jar: &Jar, pool: &PgPool) -> Uuid {
    let created = app
        .post("/v1/videos")
        .jar(jar)
        .json(json!({"filename": "Lecture.mp4", "size_bytes": MIB, "content_type": "video/mp4"}))
        .send()
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let id = Uuid::parse_str(created.body["video"]["id"].as_str().unwrap()).unwrap();
    let url = created.body["upload"]["parts"][0]["url"].as_str().unwrap();
    assert!(browser_put(url, vec![1; MIB]).await.is_success());
    let done = app
        .post(&format!("/v1/videos/{id}/complete"))
        .jar(jar)
        .send()
        .await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);

    let store = &app.state.store;
    let master = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n\
        #EXT-X-STREAM-INF:BANDWIDTH=250000,RESOLUTION=256x144,CODECS=\"avc1.4d401f,mp4a.40.2\"\n144p/index.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=70000,CODECS=\"mp4a.40.2\"\naudio/index.m3u8\n";
    store
        .put_bytes(
            &format!("videos/{id}/master.m3u8"),
            master.into(),
            "application/vnd.apple.mpegurl",
        )
        .await
        .unwrap();
    for rendition in RENDITIONS {
        let media = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n\
            #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video\",IV=0x00000000000000000000000000000000\n\
            #EXTINF:6.000000,\nseg_00000.ts\n#EXTINF:6.000000,\nseg_00001.ts\n#EXT-X-ENDLIST\n";
        store
            .put_bytes(
                &format!("videos/{id}/{rendition}/index.m3u8"),
                media.into(),
                "application/vnd.apple.mpegurl",
            )
            .await
            .unwrap();
        for n in 0..2 {
            store
                .put_bytes(
                    &format!("videos/{id}/{rendition}/seg_0000{n}.ts"),
                    segment_bytes(rendition, n),
                    "video/mp2t",
                )
                .await
                .unwrap();
        }
    }
    let renditions: Value = RENDITIONS
        .iter()
        .map(|name| json!({"name": name, "playlist": format!("{name}/index.m3u8")}))
        .collect();
    sqlx::query("update videos set status = 'ready', ready_at = now(), duration_seconds = $2, renditions = $3 where id = $1")
        .bind(id).bind(DURATION).bind(renditions).execute(pool).await.unwrap();
    sqlx::query("insert into video_keys (video_id, wrapped_key) values ($1, $2)")
        .bind(id)
        .bind(vec![7u8; 45])
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("update jobs set completed_at = now()")
        .execute(pool)
        .await
        .unwrap();
    id
}

async fn play(app: &App, jar: &Jar, video: Uuid) -> Res {
    app.post(&format!("/v1/videos/{video}/play"))
        .jar(jar)
        .json(json!({}))
        .send()
        .await
}

/// Starts playback and returns (session id, master playlist path with token).
async fn started(app: &App, jar: &Jar, video: Uuid) -> (Uuid, String) {
    let res = play(app, jar, video).await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    let p = &res.body["playback"];
    (
        Uuid::parse_str(p["session_id"].as_str().unwrap()).unwrap(),
        p["playlist_url"].as_str().unwrap().to_owned(),
    )
}

fn token_of(playlist_url: &str) -> String {
    playlist_url.split("?t=").nth(1).unwrap().to_owned()
}

/// A player fetching a playlist: no cookies, no Origin needed.
async fn fetch(app: &App, path: &str) -> Res {
    app.get(path).no_origin().send().await
}

fn text(res: &Res) -> &str {
    res.body.as_str().unwrap_or("")
}

async fn heartbeat(app: &App, jar: &Jar, session: Uuid, position: Option<u32>) -> Res {
    let body = match position {
        Some(p) => json!({ "position_seconds": p }),
        None => json!({}),
    };
    app.post(&format!("/v1/play/{session}/heartbeat"))
        .jar(jar)
        .json(body)
        .send()
        .await
}

async fn stop(app: &App, jar: &Jar, session: Uuid) -> Res {
    app.req(Method::DELETE, &format!("/v1/play/{session}"))
        .jar(jar)
        .send()
        .await
}

db_test! {
    async fn the_owner_plays_a_ready_video_through_playlists_made_for_them(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "asha@x.in").await;
        let video = ready_video(&app, &jar, &pool).await;

        let res = play(&app, &jar, video).await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        let p = &res.body["playback"];
        let session = p["session_id"].as_str().unwrap();
        let code = &session.replace('-', "")[..8].to_uppercase();
        assert_eq!(p["watermark"]["session_code"], code.as_str());
        assert_eq!(p["watermark"]["text"], format!("Asha Rao · asha@x.in · {code}"));
        assert_eq!(p["heartbeat_interval_seconds"], 30);
        assert_eq!(p["resume_from_seconds"], 0);
        let url = p["playlist_url"].as_str().unwrap();
        assert!(url.starts_with(&format!("/v1/play/{session}/master.m3u8?t=v1.")), "{url}");

        let master = fetch(&app, url).await;
        assert_eq!(master.status, StatusCode::OK, "{:?}", master.body);
        assert_eq!(master.headers[header::CONTENT_TYPE], "application/vnd.apple.mpegurl");
        assert_eq!(master.headers[header::CACHE_CONTROL], "no-store, private");
        let token = token_of(url);
        let variants: Vec<&str> = text(&master).lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(variants, [format!("144p.m3u8?t={token}"), format!("audio.m3u8?t={token}")]);

        let media = fetch(&app, &format!("/v1/play/{session}/{}", variants[0])).await;
        assert_eq!(media.status, StatusCode::OK, "{:?}", media.body);
        let body = text(&media);
        assert!(!body.contains("zeno-key"), "{body}");
        assert!(body.contains(&format!("URI=\"https://keys.test/v1/keys/{video}?t={token}\"")), "{body}");
        let segments: Vec<&str> = body.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(segments.len(), 2);
        // Each segment URL works on its own, straight against storage, with no credentials.
        for (n, url) in segments.iter().enumerate() {
            assert!(url.starts_with(storage_config().public_endpoint.as_str()), "{url}");
            let got = reqwest::get(*url).await.unwrap();
            assert_eq!(got.status(), 200);
            assert_eq!(got.bytes().await.unwrap().to_vec(), segment_bytes("144p", n));
        }
    }
}

db_test! {
    async fn only_people_who_may_watch_can_press_play(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "owner@x.in").await;
        let rival = signup_creator(&app, "rival@x.in").await;
        let student = signup_verified(&app, "kid@x.in").await;
        signup_verified_as(&app, "boss@x.in", "creator").await;
        sqlx::query("update users set role = 'admin' where email = 'boss@x.in'").execute(&pool).await.unwrap();
        let admin = Jar::of(&login(&app, "boss@x.in", PASSWORD).await);
        let video = ready_video(&app, &owner, &pool).await;

        assert_eq!(play(&app, &rival, video).await.status, StatusCode::NOT_FOUND);
        // Students get access through course enrollment, which does not exist yet.
        assert_eq!(play(&app, &student, video).await.status, StatusCode::NOT_FOUND);
        assert_eq!(app.post(&format!("/v1/videos/{video}/play")).json(json!({})).send().await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(play(&app, &admin, video).await.status, StatusCode::CREATED);
        assert_eq!(app.post("/v1/videos/nope/play").jar(&owner).json(json!({})).send().await.status, StatusCode::NOT_FOUND);

        sqlx::query("update videos set status = 'processing' where id = $1").bind(video).execute(&pool).await.unwrap();
        let res = play(&app, &owner, video).await;
        assert_eq!((res.status, res.code()), (StatusCode::CONFLICT, "not_ready"));
        sqlx::query("update videos set status = 'ready', deleted_at = now() where id = $1").bind(video).execute(&pool).await.unwrap();
        assert_eq!(play(&app, &owner, video).await.status, StatusCode::NOT_FOUND);
    }
}

db_test! {
    async fn playlist_requests_need_this_sessions_valid_token(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let video = ready_video(&app, &jar, &pool).await;
        let (a, url_a) = started(&app, &jar, video).await;
        let (b, url_b) = started(&app, &jar, video).await;
        let (token_a, token_b) = (token_of(&url_a), token_of(&url_b));
        assert_eq!(fetch(&app, &url_a).await.status, StatusCode::OK);

        let denied = |res: Res| {
            assert_eq!((res.status, res.code()), (StatusCode::FORBIDDEN, "playback_denied"), "{:?}", res.body);
        };
        denied(fetch(&app, &format!("/v1/play/{a}/master.m3u8")).await);
        denied(fetch(&app, &format!("/v1/play/{a}/master.m3u8?t=")).await);
        denied(fetch(&app, &format!("/v1/play/{a}/master.m3u8?t={token_b}")).await);
        denied(fetch(&app, &format!("/v1/play/{b}/144p.m3u8?t={token_a}")).await);
        denied(fetch(&app, &format!("/v1/play/not-a-session/master.m3u8?t={token_a}")).await);
        let mut forged = token_a.clone();
        forged.replace_range(5..6, if &forged[5..6] == "A" { "B" } else { "A" });
        denied(fetch(&app, &format!("/v1/play/{a}/master.m3u8?t={forged}")).await);
        // Correctly signed by someone who does not hold our secret.
        let outsider = PlaybackSigner::new("an-outsiders-secret-that-is-long-enough").unwrap();
        let claims = PlaybackClaims {
            session_id: a,
            video_id: video,
            user_id: Uuid::new_v4(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        };
        denied(fetch(&app, &format!("/v1/play/{a}/master.m3u8?t={}", outsider.sign(&claims))).await);
        // Even correctly signed, a token must agree with the session it names on user and video.
        let ours = PlaybackSigner::new(PLAYBACK_SECRET).unwrap();
        denied(fetch(&app, &format!("/v1/play/{a}/master.m3u8?t={}", ours.sign(&claims))).await);
        let other_video = PlaybackClaims { video_id: Uuid::new_v4(), ..claims };
        let user: Uuid = sqlx::query_scalar("select user_id from playback_sessions where id = $1").bind(a).fetch_one(&pool).await.unwrap();
        let other_video = PlaybackClaims { user_id: user, ..other_video };
        denied(fetch(&app, &format!("/v1/play/{a}/master.m3u8?t={}", ours.sign(&other_video))).await);

        // Renditions are only the video's own, by name.
        for file in ["720p.m3u8", "..%2F..%2Fuploads%2Fsource.m3u8", "144p", "master.m3u8.bak"] {
            let res = fetch(&app, &format!("/v1/play/{a}/{file}?t={token_a}")).await;
            assert_eq!(res.status, StatusCode::NOT_FOUND, "{file}: {:?}", res.body);
        }

        // Stopping, expiry and deletion all take effect on the very next request.
        assert_eq!(stop(&app, &jar, a).await.status, StatusCode::NO_CONTENT);
        denied(fetch(&app, &url_a).await);
        sqlx::query("update playback_sessions set expires_at = now() - interval '1 second' where id = $1").bind(b).execute(&pool).await.unwrap();
        denied(fetch(&app, &url_b).await);
        let (_, url_c) = started(&app, &jar, video).await;
        let gone = app.req(Method::DELETE, &format!("/v1/videos/{video}")).jar(&jar).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        denied(fetch(&app, &url_c).await);
    }
}

db_test! {
    async fn one_account_can_only_watch_on_so_many_screens_at_once(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let video = ready_video(&app, &jar, &pool).await;
        let (first, _) = started(&app, &jar, video).await;
        let (second, _) = started(&app, &jar, video).await;

        let third = play(&app, &jar, video).await;
        assert_eq!((third.status, third.code()), (StatusCode::CONFLICT, "stream_limit"));
        assert!(third.body["error"]["message"].as_str().unwrap().contains("2 screens"));

        // Stopping one frees its slot.
        assert_eq!(stop(&app, &jar, first).await.status, StatusCode::NO_CONTENT);
        let (third, _) = started(&app, &jar, video).await;

        // A screen that went quiet (closed laptop) gives up its slot to the next one to press play,
        // and is told why when it comes back.
        sqlx::query("update playback_sessions set last_seen_at = now() - interval '10 minutes' where id = $1")
            .bind(second).execute(&pool).await.unwrap();
        let (_fourth, _) = started(&app, &jar, video).await;
        let back = heartbeat(&app, &jar, second, Some(10)).await;
        assert_eq!((back.status, back.code()), (StatusCode::CONFLICT, "session_ended"));
        assert!(back.body["error"]["message"].as_str().unwrap().contains("another screen"));
        assert_eq!(heartbeat(&app, &jar, third, Some(10)).await.status, StatusCode::NO_CONTENT);
        let reason: Option<String> = sqlx::query_scalar("select end_reason from playback_sessions where id = $1")
            .bind(second).fetch_one(&pool).await.unwrap();
        assert_eq!(reason.as_deref(), Some("idle"));
    }
}

db_test! {
    async fn simultaneous_presses_of_play_cannot_overshoot_the_limit(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let video = ready_video(&app, &jar, &pool).await;
        let attempts = (0..8).map(|_| play(&app, &jar, video));
        let results = futures_util::future::join_all(attempts).await;
        let ok = results.iter().filter(|r| r.status == StatusCode::CREATED).count();
        let refused = results.iter().filter(|r| r.code() == "stream_limit").count();
        assert_eq!((ok, refused), (MAX_STREAMS as usize, 8 - MAX_STREAMS as usize));
        assert_eq!(count(&pool, "select count(*) from playback_sessions where ended_at is null").await, i64::from(MAX_STREAMS));
    }
}

db_test! {
    async fn heartbeats_save_the_position_and_the_next_session_resumes_there(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let rival = signup_creator(&app, "rival@x.in").await;
        let video = ready_video(&app, &jar, &pool).await;
        let (session, _) = started(&app, &jar, video).await;

        assert_eq!(heartbeat(&app, &jar, session, Some(42)).await.status, StatusCode::NO_CONTENT);
        assert_eq!(heartbeat(&app, &jar, session, None).await.status, StatusCode::NO_CONTENT, "position is optional");
        // Someone else cannot report on (or stop) my session; it looks like it does not exist.
        assert_eq!(heartbeat(&app, &rival, session, Some(1)).await.status, StatusCode::NOT_FOUND);
        assert_eq!(stop(&app, &rival, session).await.status, StatusCode::NOT_FOUND);
        assert_eq!(heartbeat(&app, &jar, Uuid::new_v4(), Some(1)).await.status, StatusCode::NOT_FOUND);
        let bad = app.post(&format!("/v1/play/{session}/heartbeat")).jar(&jar).json(json!({"position_seconds": -5})).send().await;
        assert_eq!(bad.status.as_u16() / 100, 4);

        stop(&app, &jar, session).await;
        let res = play(&app, &jar, video).await;
        assert_eq!(res.body["playback"]["resume_from_seconds"], 42);

        // Positions past the end are clamped; finishing the video means the next play starts over.
        let next = Uuid::parse_str(res.body["playback"]["session_id"].as_str().unwrap()).unwrap();
        assert_eq!(heartbeat(&app, &jar, next, Some(99_999)).await.status, StatusCode::NO_CONTENT);
        let stored: i32 = sqlx::query_scalar("select last_position_seconds from playback_sessions where id = $1")
            .bind(next).fetch_one(&pool).await.unwrap();
        assert_eq!(stored, DURATION);
        stop(&app, &jar, next).await;
        assert_eq!(play(&app, &jar, video).await.body["playback"]["resume_from_seconds"], 0);

        // An ended session says so.
        let ended = heartbeat(&app, &jar, next, Some(1)).await;
        assert_eq!((ended.status, ended.code()), (StatusCode::CONFLICT, "session_ended"));
        assert_eq!(stop(&app, &jar, next).await.status, StatusCode::NO_CONTENT, "stopping twice is fine");
    }
}

db_test! {
    async fn a_password_reset_ends_every_playback_session_of_that_account(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "a@x.in").await;
        let video = ready_video(&app, &jar, &pool).await;
        let (session, url) = started(&app, &jar, video).await;

        app.post("/v1/auth/forgot-password").json(json!({"email": "a@x.in"})).send().await;
        let token = token_from(&app.mailer.wait_for(2).await[1]);
        let reset = app.post("/v1/auth/reset-password")
            .json(json!({"token": token, "new_password": "a-completely-new-passphrase"}))
            .send().await;
        assert_eq!(reset.status, StatusCode::OK, "{:?}", reset.body);

        assert_eq!(fetch(&app, &url).await.status, StatusCode::FORBIDDEN);
        let reason: Option<String> = sqlx::query_scalar("select end_reason from playback_sessions where id = $1")
            .bind(session).fetch_one(&pool).await.unwrap();
        assert_eq!(reason.as_deref(), Some("password_reset"));
    }
}

db_test! {
    async fn a_tampered_stored_playlist_is_never_passed_on(pool: PgPool) {
        let app = app(pool.clone());
        let jar = signup_creator(&app, "t@x.in").await;
        let video = ready_video(&app, &jar, &pool).await;
        let (session, url) = started(&app, &jar, video).await;
        let token = token_of(&url);
        let clear = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nseg_00000.ts\n#EXT-X-ENDLIST\n";
        app.state.store.put_bytes(&format!("videos/{video}/144p/index.m3u8"), clear.into(), "application/vnd.apple.mpegurl").await.unwrap();
        let res = fetch(&app, &format!("/v1/play/{session}/144p.m3u8?t={token}")).await;
        assert_eq!(res.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!text(&res).contains("seg_00000"));
    }
}
