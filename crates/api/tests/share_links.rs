//! Share links: making and revoking them, what a visitor with no account can see and do, and the
//! guarantees that matter: a closed link stays closed, a viewing ends when its link does, and
//! nobody can guess, brute-force or replay their way in.

mod common;

use axum::http::{header, Method, StatusCode};
use common::media::ready_video;
use common::*;
use futures_util::future::join_all;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::playback::{PlaybackClaims, ANONYMOUS_VIEWER};

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

fn rid(res: &Res) -> Uuid {
    Uuid::parse_str(res.body["recording"]["id"].as_str().unwrap()).unwrap()
}

fn link_id(res: &Res) -> Uuid {
    Uuid::parse_str(res.body["link"]["id"].as_str().unwrap()).unwrap()
}

fn code(res: &Res) -> String {
    res.body["link"]["code"].as_str().unwrap().to_owned()
}

async fn recording(app: &App, jar: &Jar, video: Option<Uuid>) -> Uuid {
    let made = app
        .post("/v1/recordings")
        .jar(jar)
        .json(json!({"source": "extension", "title": "Week 3", "video_id": video}))
        .send()
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
    rid(&made)
}

async fn make_link(app: &App, jar: &Jar, recording: Uuid, body: Value) -> Res {
    app.post(&format!("/v1/recordings/{recording}/links"))
        .jar(jar)
        .json(body)
        .send()
        .await
}

async fn resolve(app: &App, code: &str) -> Res {
    app.get(&format!("/v1/s/{code}")).send().await
}

async fn play(app: &App, code: &str, body: Value) -> Res {
    app.post(&format!("/v1/s/{code}/play"))
        .json(body)
        .send()
        .await
}

async fn fetch(app: &App, url: &str) -> Res {
    app.get(url).send().await
}

fn token_of(playlist_url: &str) -> String {
    playlist_url.split("?t=").nth(1).unwrap().to_owned()
}

/// A shared recording of a real, playable video, with one open link.
struct Shared {
    app: App,
    pool: PgPool,
    owner: Jar,
    recording: Uuid,
    video: Uuid,
    link: Uuid,
    code: String,
}

async fn shared(pool: PgPool) -> Shared {
    let app = app(pool.clone());
    let owner = signup_creator(&app, "teacher@x.in").await;
    let video = ready_video(&app, &owner, &pool).await;
    let recording = recording(&app, &owner, Some(video)).await;
    let made = make_link(&app, &owner, recording, json!({})).await;
    assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
    Shared {
        link: link_id(&made),
        code: code(&made),
        app,
        pool,
        owner,
        recording,
        video,
    }
}

async fn session_end(pool: &PgPool, session: &str) -> (bool, Option<String>) {
    sqlx::query_as::<_, (bool, Option<String>)>(
        "select ended_at is not null, end_reason from playback_sessions where id = $1::uuid",
    )
    .bind(session)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Presses play on a link as a visitor; the session id and the playlist address.
async fn start_watching(app: &App, code: &str) -> (String, String) {
    let res = play(app, code, json!({})).await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    (
        res.body["playback"]["session_id"]
            .as_str()
            .unwrap()
            .to_owned(),
        res.body["playback"]["playlist_url"]
            .as_str()
            .unwrap()
            .to_owned(),
    )
}

async fn open_sessions(pool: &PgPool) -> i64 {
    sqlx::query_scalar("select count(*) from playback_sessions where ended_at is null")
        .fetch_one(pool)
        .await
        .unwrap()
}

db_test! {
    async fn a_lost_link_response_returns_the_same_link_once(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "retry@links.in").await;
        let rec = recording(&app, &owner, None).await;
        let id = Uuid::new_v4();
        let body = json!({"id": id});
        let first = make_link(&app, &owner, rec, body.clone()).await;
        let again = make_link(&app, &owner, rec, body).await;
        assert_eq!(first.status, StatusCode::CREATED, "{:?}", first.body);
        assert_eq!(again.status, StatusCode::CREATED, "{:?}", again.body);
        assert_eq!(link_id(&first), id);
        assert_eq!(first.body["link"]["code"], again.body["link"]["code"]);
        assert_eq!(again.body["recording_visibility"], "link");
        let count: i64 = sqlx::query_scalar("select count(*) from share_links where recording_id = $1")
            .bind(rec).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);

        let another = recording(&app, &owner, None).await;
        let conflict = make_link(&app, &owner, another, json!({"id": id})).await;
        assert_eq!(conflict.status, StatusCode::CONFLICT);
        assert_eq!(conflict.code(), "link_id_conflict");
        let changed = make_link(&app, &owner, rec, json!({"id": id, "password": "other-password"})).await;
        assert_eq!(changed.status, StatusCode::CONFLICT);
        assert_eq!(changed.code(), "link_id_conflict");

        let racing = Uuid::new_v4();
        let (left, right) = tokio::join!(
            make_link(&app, &owner, rec, json!({"id": racing})),
            make_link(&app, &owner, rec, json!({"id": racing})),
        );
        assert_eq!(left.status, StatusCode::CREATED, "{:?}", left.body);
        assert_eq!(right.status, StatusCode::CREATED, "{:?}", right.body);
        assert_eq!(left.body["link"]["code"], right.body["link"]["code"]);
        let count: i64 = sqlx::query_scalar("select count(*) from share_links where id = $1")
            .bind(racing).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);

        let protected_id = Uuid::new_v4();
        let protected = json!({"id": protected_id, "password": "same-secret"});
        let first = make_link(&app, &owner, rec, protected.clone()).await;
        let again = make_link(&app, &owner, rec, protected).await;
        assert_eq!(first.status, StatusCode::CREATED);
        assert_eq!(again.status, StatusCode::CREATED);
        assert_eq!(code(&first), code(&again));
        let wrong = make_link(&app, &owner, rec, json!({"id": protected_id, "password": "different-secret"})).await;
        assert_eq!(wrong.code(), "link_id_conflict");

        let revoked = app.req(Method::DELETE, &format!("/v1/recordings/{rec}/links/{id}")).jar(&owner).send().await;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);
        let after_revoke = make_link(&app, &owner, rec, json!({"id": id})).await;
        assert_eq!(after_revoke.code(), "link_id_conflict", "a retry cannot revive a revoked link");
    }
}

db_test! {
    async fn an_owner_makes_lists_and_revokes_links(pool: PgPool) {
        let app = app(pool);
        let owner = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let rec = recording(&app, &owner, None).await;
        let before = app.get(&format!("/v1/recordings/{rec}")).jar(&owner).send().await;
        assert_eq!(before.body["recording"]["visibility"], "private");

        let made = make_link(&app, &owner, rec, json!({})).await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
        let c = code(&made);
        assert_eq!(c.len(), 22);
        assert!(c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_eq!(made.body["link"]["url"], format!("{ORIGIN}/s/{c}"));
        assert_eq!(made.body["link"]["state"], "active");
        assert_eq!(made.body["link"]["has_password"], false);
        // A link to a private recording would be a trap: making one shares it.
        assert_eq!(made.body["recording_visibility"], "link");
        let after = app.get(&format!("/v1/recordings/{rec}")).jar(&owner).send().await;
        assert_eq!(after.body["recording"]["visibility"], "link");
        assert_eq!(after.body["recording"]["version"], 2);

        let listed = app.get(&format!("/v1/recordings/{rec}/links")).jar(&owner).send().await;
        assert_eq!(listed.status, StatusCode::OK);
        assert_eq!(listed.body["links"].as_array().unwrap().len(), 1);
        assert!(!listed.body.to_string().contains("password_hash"));

        let id = link_id(&made);
        let path = format!("/v1/recordings/{rec}/links/{id}");
        let revoke = |app: &App, jar: &Jar| app.req(Method::DELETE, &path).jar(jar).send();
        assert_eq!(revoke(&app, &owner).await.status, StatusCode::NO_CONTENT);
        assert_eq!(revoke(&app, &owner).await.status, StatusCode::NO_CONTENT, "revoking twice is fine");
        let relisted = app.get(&format!("/v1/recordings/{rec}/links")).jar(&owner).send().await;
        assert_eq!(relisted.body["links"][0]["state"], "revoked");
        assert!(!relisted.body["links"][0]["revoked_at"].is_null());

        let ghost = format!("/v1/recordings/{rec}/links/{}", Uuid::new_v4());
        assert_eq!(app.req(Method::DELETE, &ghost).jar(&owner).send().await.status, StatusCode::NOT_FOUND);
        let junk = format!("/v1/recordings/{rec}/links/not-a-uuid");
        assert_eq!(app.req(Method::DELETE, &junk).jar(&owner).send().await.status, StatusCode::NOT_FOUND);
    }
}

db_test! {
    async fn links_are_validated_hashed_and_capped(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_verified_as(&app, "teacher@x.in", "creator").await;
        let rec = recording(&app, &owner, None).await;
        let soon = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let far = (chrono::Utc::now() + chrono::Duration::days(4000)).to_rfc3339();
        for body in [
            json!({"password": "abc"}),
            json!({"password": "x".repeat(129)}),
            json!({"password": "tab\there"}),
            json!({"expires_at": past}),
            json!({"expires_at": far}),
            json!({"surprise": 1}),
        ] {
            let res = make_link(&app, &owner, rec, body.clone()).await;
            assert!(res.status.is_client_error(), "{body}: {:?}", res.body);
        }
        let none: i64 = sqlx::query_scalar("select count(*) from share_links").fetch_one(&pool).await.unwrap();
        assert_eq!(none, 0, "refused requests made nothing");
        let private: Value = app.get(&format!("/v1/recordings/{rec}")).jar(&owner).send().await.body;
        assert_eq!(private["recording"]["visibility"], "private", "a refused link must not share the recording");

        let good = make_link(&app, &owner, rec, json!({"password": "open sesame", "expires_at": soon})).await;
        assert_eq!(good.status, StatusCode::CREATED, "{:?}", good.body);
        assert_eq!(good.body["link"]["has_password"], true);
        let stored: String = sqlx::query_scalar("select password_hash from share_links where id = $1")
            .bind(link_id(&good)).fetch_one(&pool).await.unwrap();
        assert!(stored.starts_with("$argon2id$"), "{stored}");
        assert!(!stored.contains("open sesame"));
        assert!(!good.body.to_string().contains("argon2"));

        // Ten live links is the most a recording may have; revoking one makes room.
        let mut ids = vec![link_id(&good)];
        for _ in 0..9 {
            ids.push(link_id(&make_link(&app, &owner, rec, json!({})).await));
        }
        let eleventh = make_link(&app, &owner, rec, json!({})).await;
        assert_eq!((eleventh.status, eleventh.code()), (StatusCode::CONFLICT, "too_many_links"));
        let path = format!("/v1/recordings/{rec}/links/{}", ids[3]);
        assert_eq!(app.req(Method::DELETE, &path).jar(&owner).send().await.status, StatusCode::NO_CONTENT);
        assert_eq!(make_link(&app, &owner, rec, json!({})).await.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn only_the_owner_manages_a_recordings_links(pool: PgPool) {
        let app = app(pool);
        let owner = signup_verified_as(&app, "owner@x.in", "creator").await;
        let other = signup_verified_as(&app, "other@x.in", "creator").await;
        let student = signup_verified_as(&app, "student@x.in", "student").await;
        let rec = recording(&app, &owner, None).await;
        let made = make_link(&app, &owner, rec, json!({})).await;
        let path = format!("/v1/recordings/{rec}/links/{}", link_id(&made));

        assert_eq!(make_link(&app, &other, rec, json!({})).await.status, StatusCode::NOT_FOUND);
        assert_eq!(app.get(&format!("/v1/recordings/{rec}/links")).jar(&other).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(app.req(Method::DELETE, &path).jar(&other).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(make_link(&app, &student, rec, json!({})).await.status, StatusCode::FORBIDDEN);
        assert_eq!(app.get(&format!("/v1/recordings/{rec}/links")).jar(&student).send().await.status, StatusCode::FORBIDDEN);
        let anon = app.req(Method::DELETE, &path).send().await;
        assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
        // The other creator's attempt changed nothing.
        let still = resolve(&app, &code(&made)).await;
        assert_eq!(still.status, StatusCode::OK);
    }
}

db_test! {
    async fn a_visitor_with_no_account_gets_a_cacheable_page(pool: PgPool) {
        let s = shared(pool).await;
        let page = resolve(&s.app, &s.code).await;
        assert_eq!(page.status, StatusCode::OK, "{:?}", page.body);
        assert_eq!(
            page.body,
            json!({"share": {"code": s.code, "requires_password": false, "title": "Week 3",
                              "status": "ready", "duration_seconds": 600}})
        );
        let cache = page.headers[header::CACHE_CONTROL].to_str().unwrap();
        assert!(cache.starts_with("public, max-age=15"), "{cache}");
        assert!(page.headers.get(header::SET_COOKIE).is_none(), "a visit must not set cookies");
        let etag = page.headers[header::ETAG].to_str().unwrap().to_owned();

        let again = s.app.get(&format!("/v1/s/{}", s.code)).header("if-none-match", &etag).send().await;
        assert_eq!(again.status, StatusCode::NOT_MODIFIED);
        assert_eq!(again.headers[header::ETAG].to_str().unwrap(), etag);

        // The page follows the recording: a rename shows up (new validator) once it is fetched again.
        let rec = format!("/v1/recordings/{}", s.recording);
        let version = s.app.get(&rec).jar(&s.owner).send().await.body["recording"]["version"].clone();
        let renamed = s.app.req(Method::PATCH, &rec).jar(&s.owner)
            .json(json!({"version": version, "title": "Week 3, redone"})).send().await;
        assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
        let changed = s.app.get(&format!("/v1/s/{}", s.code)).header("if-none-match", &etag).send().await;
        assert_eq!(changed.status, StatusCode::OK);
        assert_eq!(changed.body["share"]["title"], "Week 3, redone");
    }
}

db_test! {
    async fn a_processing_video_shows_as_processing_and_cannot_be_played_yet(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let video = ready_video(&app, &owner, &pool).await;
        sqlx::query("update videos set status = 'processing' where id = $1").bind(video).execute(&pool).await.unwrap();
        let rec = recording(&app, &owner, Some(video)).await;
        let c = code(&make_link(&app, &owner, rec, json!({})).await);
        let waiting = resolve(&app, &c).await;
        assert_eq!(waiting.body["share"]["status"], "processing");
        // Somebody waiting for it to play must find out at once: the answer is not kept for long, and never served stale.
        assert_eq!(waiting.headers[header::CACHE_CONTROL], "public, max-age=2");
        let started = play(&app, &c, json!({})).await;
        assert_eq!((started.status, started.code()), (StatusCode::CONFLICT, "not_ready"));
        assert_eq!(open_sessions(&pool).await, 0);

        // The moment it is ready the page is kept for the usual fifteen seconds again.
        sqlx::query("update videos set status = 'ready' where id = $1").bind(video).execute(&pool).await.unwrap();
        let ready = resolve(&app, &c).await;
        assert_eq!(ready.body["share"]["status"], "ready");
        assert!(ready.headers[header::CACHE_CONTROL].to_str().unwrap().starts_with("public, max-age=15"));
    }
}

db_test! {
    async fn a_password_link_reveals_nothing_until_it_is_unlocked(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let video = ready_video(&app, &owner, &pool).await;
        let rec = recording(&app, &owner, Some(video)).await;
        let c = code(&make_link(&app, &owner, rec, json!({"password": "open sesame"})).await);

        let page = resolve(&app, &c).await;
        assert_eq!(page.status, StatusCode::OK);
        assert_eq!(page.body, json!({"share": {"code": c, "requires_password": true}}), "not even the title");

        let none = play(&app, &c, json!({})).await;
        assert_eq!((none.status, none.code()), (StatusCode::FORBIDDEN, "password_required"));
        let wrong = play(&app, &c, json!({"password": "open sesamE"})).await;
        assert_eq!((wrong.status, wrong.code()), (StatusCode::FORBIDDEN, "wrong_password"));
        let long = play(&app, &c, json!({"password": "x".repeat(5000)})).await;
        assert_eq!((long.status, long.code()), (StatusCode::FORBIDDEN, "wrong_password"));
        assert_eq!(open_sessions(&pool).await, 0, "no session without the password");

        let right = play(&app, &c, json!({"password": "open sesame"})).await;
        assert_eq!(right.status, StatusCode::CREATED, "{:?}", right.body);
        assert_eq!(open_sessions(&pool).await, 1);
    }
}

db_test! {
    async fn password_guessing_is_limited_per_link_and_cannot_be_outrun(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "teacher@x.in").await;
        let video = ready_video(&app, &owner, &pool).await;
        let rec = recording(&app, &owner, Some(video)).await;
        let locked = code(&make_link(&app, &owner, rec, json!({"password": "open sesame"})).await);
        let open = code(&make_link(&app, &owner, rec, json!({})).await);

        for n in 0..10 {
            let res = play(&app, &locked, json!({"password": format!("guess number {n}")})).await;
            assert_eq!(res.code(), "wrong_password", "attempt {n}");
        }
        let eleventh = play(&app, &locked, json!({"password": "guess number 10"})).await;
        assert_eq!(eleventh.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(eleventh.headers.get(header::RETRY_AFTER).is_some());
        // Even the right password is refused now: the budget is spent before anything is checked.
        let right = play(&app, &locked, json!({"password": "open sesame"})).await;
        assert_eq!(right.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(open_sessions(&pool).await, 0);
        // Another link from the same address is not held up by it.
        assert_eq!(play(&app, &open, json!({})).await.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn unknown_codes_are_not_found_and_closed_links_are_gone(pool: PgPool) {
        let s = shared(pool).await;
        // Never existed, in any shape: no hint, and the malformed ones never reach the database.
        for bad in ["abcdefghijklmnopqrstuv", "short", "abcdefghijklmnopqrstu+", "abcdefghijklmnopqrstuvw", "%C3%A9bcdefghijklmnopqrstu"] {
            let res = resolve(&s.app, bad).await;
            assert_eq!((res.status, res.code()), (StatusCode::NOT_FOUND, "not_found"), "{bad}");
            assert_eq!(play(&s.app, bad, json!({})).await.status, StatusCode::NOT_FOUND, "{bad}");
        }
        let live = resolve(&s.app, &s.code).await;
        assert_eq!(live.status, StatusCode::OK);

        let gone = |res: Res| (res.status, res.code().to_owned());
        let expected = (StatusCode::GONE, "link_unavailable".to_owned());

        // Expired.
        sqlx::query("update share_links set expires_at = now() - interval '1 second' where id = $1")
            .bind(s.link).execute(&s.pool).await.unwrap();
        assert_eq!(gone(resolve(&s.app, &s.code).await), expected, "expired");
        assert_eq!(gone(play(&s.app, &s.code, json!({})).await), expected, "expired play");
        sqlx::query("update share_links set expires_at = null where id = $1").bind(s.link).execute(&s.pool).await.unwrap();
        assert_eq!(resolve(&s.app, &s.code).await.status, StatusCode::OK);

        // Switched off by making the recording private, and back on again.
        let rec = format!("/v1/recordings/{}", s.recording);
        let version = s.app.get(&rec).jar(&s.owner).send().await.body["recording"]["version"].clone();
        let off = s.app.req(Method::PATCH, &rec).jar(&s.owner).json(json!({"version": version, "visibility": "private"})).send().await;
        assert_eq!(off.status, StatusCode::OK, "{:?}", off.body);
        assert_eq!(gone(resolve(&s.app, &s.code).await), expected, "private");
        let on = s.app.req(Method::PATCH, &rec).jar(&s.owner).json(json!({"version": off.body["recording"]["version"], "visibility": "link"})).send().await;
        assert_eq!(on.status, StatusCode::OK);
        assert_eq!(resolve(&s.app, &s.code).await.status, StatusCode::OK, "the kill switch is reversible");

        // Revoked: for good.
        let path = format!("/v1/recordings/{}/links/{}", s.recording, s.link);
        s.app.req(Method::DELETE, &path).jar(&s.owner).send().await;
        assert_eq!(gone(resolve(&s.app, &s.code).await), expected, "revoked");
        assert_eq!(gone(play(&s.app, &s.code, json!({})).await), expected, "revoked play");

        // A fresh link, then the recording itself deleted.
        let again = code(&make_link(&s.app, &s.owner, s.recording, json!({})).await);
        assert_eq!(resolve(&s.app, &again).await.status, StatusCode::OK);
        s.app.req(Method::DELETE, &rec).jar(&s.owner).send().await;
        assert_eq!(gone(resolve(&s.app, &again).await), expected, "recording deleted");
    }
}

db_test! {
    async fn a_viewing_plays_end_to_end_with_no_account_and_ends_when_the_link_is_revoked(pool: PgPool) {
        let s = shared(pool).await;
        let started = play(&s.app, &s.code, json!({})).await;
        assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
        let playback = &started.body["playback"];
        let session = playback["session_id"].as_str().unwrap().to_owned();
        let url = playback["playlist_url"].as_str().unwrap().to_owned();
        assert!(url.starts_with(&format!("/v1/play/{session}/master.m3u8?t=")), "{url}");
        assert!(playback["watermark"]["text"].as_str().unwrap().starts_with("Shared view"));
        assert!(playback.get("heartbeat_interval_seconds").is_none(), "no heartbeat for a visitor");
        let token = token_of(&url);

        // The row says whose it is: a link, no user.
        let (user, link): (Option<Uuid>, Option<Uuid>) =
            sqlx::query_as("select user_id, share_link_id from playback_sessions where id = $1::uuid")
                .bind(&session).fetch_one(&s.pool).await.unwrap();
        assert_eq!((user, link), (None, Some(s.link)));
        // And the token carries the anonymous viewer.
        let claims = s.app.state.config.playback.verify(&token, chrono::Utc::now()).unwrap();
        assert_eq!(claims.user_id, ANONYMOUS_VIEWER);
        assert_eq!(claims.account(), None);

        // No cookies anywhere: the master playlist, then a media playlist with a key URL.
        let master = fetch(&s.app, &url).await;
        assert_eq!(master.status, StatusCode::OK, "{:?}", master.body);
        assert!(master.body.as_str().unwrap().contains("144p.m3u8"));
        let media = fetch(&s.app, &format!("/v1/play/{session}/144p.m3u8?t={token}")).await;
        assert_eq!(media.status, StatusCode::OK, "{:?}", media.body);
        assert!(media.body.as_str().unwrap().contains(&format!("/v1/keys/{}?t=", s.video)));

        // Revoking ends the viewing at once, and the very next request is refused.
        let path = format!("/v1/recordings/{}/links/{}", s.recording, s.link);
        assert_eq!(s.app.req(Method::DELETE, &path).jar(&s.owner).send().await.status, StatusCode::NO_CONTENT);
        assert_eq!(session_end(&s.pool, &session).await, (true, Some("access_revoked".into())));
        let after = fetch(&s.app, &url).await;
        assert_eq!((after.status, after.code()), (StatusCode::FORBIDDEN, "playback_denied"));
        let media_after = fetch(&s.app, &format!("/v1/play/{session}/144p.m3u8?t={token}")).await;
        assert_eq!(media_after.status, StatusCode::FORBIDDEN);
    }
}

db_test! {
    async fn every_way_of_closing_a_link_ends_the_viewings_under_way(pool: PgPool) {
        let s = shared(pool).await;
        let rec = format!("/v1/recordings/{}", s.recording);
        // 1. The recording goes private.
        let (a, a_url) = start_watching(&s.app, &s.code).await;
        let v = s.app.get(&rec).jar(&s.owner).send().await.body["recording"]["version"].clone();
        s.app.req(Method::PATCH, &rec).jar(&s.owner).json(json!({"version": v, "visibility": "private"})).send().await;
        assert_eq!(session_end(&s.pool, &a).await, (true, Some("access_revoked".into())), "private");
        assert_eq!(fetch(&s.app, &a_url).await.status, StatusCode::FORBIDDEN);

        // 2. The link expires while someone is watching (checked on their next request).
        let v = s.app.get(&rec).jar(&s.owner).send().await.body["recording"]["version"].clone();
        s.app.req(Method::PATCH, &rec).jar(&s.owner).json(json!({"version": v, "visibility": "link"})).send().await;
        let (b, b_url) = start_watching(&s.app, &s.code).await;
        assert_eq!(fetch(&s.app, &b_url).await.status, StatusCode::OK);
        sqlx::query("update share_links set expires_at = now() - interval '1 second' where id = $1")
            .bind(s.link).execute(&s.pool).await.unwrap();
        let denied = fetch(&s.app, &b_url).await;
        assert_eq!((denied.status, denied.code()), (StatusCode::FORBIDDEN, "playback_denied"));
        assert_eq!(session_end(&s.pool, &b).await, (true, Some("expired".into())), "expired");

        // 3. The recording is deleted.
        sqlx::query("update share_links set expires_at = null where id = $1").bind(s.link).execute(&s.pool).await.unwrap();
        let (c, c_url) = start_watching(&s.app, &s.code).await;
        s.app.req(Method::DELETE, &rec).jar(&s.owner).send().await;
        assert!(session_end(&s.pool, &c).await.0, "deleted");
        assert_eq!(fetch(&s.app, &c_url).await.status, StatusCode::FORBIDDEN);
        assert_eq!(open_sessions(&s.pool).await, 0);
    }
}

db_test! {
    async fn a_link_token_is_good_for_nothing_else(pool: PgPool) {
        let s = shared(pool).await;
        let started = play(&s.app, &s.code, json!({})).await;
        let session = started.body["playback"]["session_id"].as_str().unwrap().to_owned();
        let token = token_of(started.body["playback"]["playlist_url"].as_str().unwrap());

        // Another session id with this token, a made-up one, and a tampered token.
        let other = Uuid::new_v4();
        assert_eq!(fetch(&s.app, &format!("/v1/play/{other}/master.m3u8?t={token}")).await.status, StatusCode::FORBIDDEN);
        let mut forged = token.clone();
        forged.push('x');
        assert_eq!(fetch(&s.app, &format!("/v1/play/{session}/master.m3u8?t={forged}")).await.status, StatusCode::FORBIDDEN);

        // A token for this very session that names a real account (not the anonymous viewer)
        // does not match the session's own (empty) viewer.
        let claims = s.app.state.config.playback.verify(&token, chrono::Utc::now()).unwrap();
        let as_user = s.app.state.config.playback.sign(&PlaybackClaims { user_id: Uuid::new_v4(), ..claims });
        assert_eq!(fetch(&s.app, &format!("/v1/play/{session}/master.m3u8?t={as_user}")).await.status, StatusCode::FORBIDDEN);
        // ...nor a token for another video.
        let other_video = s.app.state.config.playback.sign(&PlaybackClaims { video_id: Uuid::new_v4(), ..claims });
        assert_eq!(fetch(&s.app, &format!("/v1/play/{session}/master.m3u8?t={other_video}")).await.status, StatusCode::FORBIDDEN);

        // A signed-in account cannot heartbeat or stop a visitor's session.
        let teacher = &s.owner;
        let beat = s.app.post(&format!("/v1/play/{session}/heartbeat")).jar(teacher).json(json!({"position_seconds": 5})).send().await;
        assert_eq!(beat.status, StatusCode::NOT_FOUND);
        let stop = s.app.req(Method::DELETE, &format!("/v1/play/{session}")).jar(teacher).send().await;
        assert_eq!(stop.status, StatusCode::NOT_FOUND);
        assert_eq!(open_sessions(&s.pool).await, 1, "still open");
    }
}

db_test! {
    async fn a_link_cannot_play_a_video_it_does_not_share(pool: PgPool) {
        let s = shared(pool.clone()).await;
        // Another recording of the same owner, never shared.
        let video2 = ready_video(&s.app, &s.owner, &pool).await;
        recording(&s.app, &s.owner, Some(video2)).await;
        let started = play(&s.app, &s.code, json!({})).await;
        let session = started.body["playback"]["session_id"].as_str().unwrap().to_owned();
        let token = token_of(started.body["playback"]["playlist_url"].as_str().unwrap());
        // Re-point the live session at the other video, as an attacker who could write the row
        // would: the access rule notices the link no longer shows it.
        sqlx::query("update playback_sessions set video_id = $2 where id = $1::uuid").bind(&session).bind(video2).execute(&pool).await.unwrap();
        let claims = s.app.state.config.playback.verify(&token, chrono::Utc::now()).unwrap();
        let moved = s.app.state.config.playback.sign(&PlaybackClaims { video_id: video2, ..claims });
        let res = fetch(&s.app, &format!("/v1/play/{session}/master.m3u8?t={moved}")).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        assert_eq!(session_end(&pool, &session).await, (true, Some("access_revoked".into())));
    }
}

db_test! {
    async fn a_burst_of_visitors_on_one_link_all_get_in(pool: PgPool) {
        let s = shared(pool).await;
        let attempts = (0..25).map(|_| play(&s.app, &s.code, json!({})));
        let results = join_all(attempts).await;
        assert!(results.iter().all(|r| r.status == StatusCode::CREATED), "{:?}", results.iter().map(|r| r.status).collect::<Vec<_>>());
        let sessions: Vec<Value> = results.iter().map(|r| r.body["playback"]["session_id"].clone()).collect();
        let distinct: std::collections::HashSet<String> = sessions.iter().map(|v| v.to_string()).collect();
        assert_eq!(distinct.len(), 25);
        let linked: i64 = sqlx::query_scalar("select count(*) from playback_sessions where share_link_id = $1 and user_id is null")
            .bind(s.link).fetch_one(&s.pool).await.unwrap();
        assert_eq!(linked, 25);
        // Revoking ends all of them in one statement.
        let path = format!("/v1/recordings/{}/links/{}", s.recording, s.link);
        s.app.req(Method::DELETE, &path).jar(&s.owner).send().await;
        assert_eq!(open_sessions(&s.pool).await, 0);
    }
}

db_test! {
    async fn starting_to_watch_is_limited_per_address(pool: PgPool) {
        let s = shared(pool).await;
        let mut refused = 0;
        for _ in 0..40 {
            let res = play(&s.app, &s.code, json!({})).await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                refused += 1;
            } else {
                assert_eq!(res.status, StatusCode::CREATED);
            }
        }
        assert_eq!(refused, 10, "30 a minute, then refused");
        // Reading the page is a different, much larger budget.
        assert_eq!(resolve(&s.app, &s.code).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn old_link_viewings_are_forgotten_after_a_week_but_accounts_keep_their_history(pool: PgPool) {
        let s = shared(pool.clone()).await;
        let started = play(&s.app, &s.code, json!({})).await;
        let link_session = started.body["playback"]["session_id"].as_str().unwrap().to_owned();
        let me: Uuid = sqlx::query_scalar("select id from users where email = 'teacher@x.in'").fetch_one(&pool).await.unwrap();
        let account_session = Uuid::new_v4();
        sqlx::query("insert into playback_sessions (id, user_id, video_id, expires_at, started_at) values ($1,$2,$3, now() + interval '1 hour', now())")
            .bind(account_session).bind(me).bind(s.video).execute(&pool).await.unwrap();
        for id in [link_session.clone(), account_session.to_string()] {
            sqlx::query("update playback_sessions set ended_at = now() - interval '10 days', end_reason = 'stopped' where id = $1::uuid")
                .bind(id).execute(&pool).await.unwrap();
        }
        let (_, forgotten) = zeno_api::playback::repo::sweep(&pool, 90).await.unwrap();
        assert_eq!(forgotten, 1);
        let left: Vec<String> = sqlx::query_scalar("select id::text from playback_sessions").fetch_all(&pool).await.unwrap();
        assert_eq!(left, vec![account_session.to_string()]);
    }
}

db_test! {
    async fn the_access_rule_in_the_database_agrees_with_the_page(pool: PgPool) {
        let s = shared(pool.clone()).await;
        for (expires, revoked, visibility, deleted, expect) in [
            ("null", false, "link", false, "allowed"),
            ("null", false, "public", false, "allowed"),
            ("now() + interval '1 hour'", false, "link", false, "allowed"),
            ("now() - interval '1 hour'", false, "link", false, "expired"),
            ("null", true, "link", false, "access_revoked"),
            ("null", false, "private", false, "access_revoked"),
            ("null", false, "link", true, "access_revoked"),
        ] {
            let links = format!(
                "update share_links set expires_at = {expires}, revoked_at = {} where id = $1",
                if revoked { "now()" } else { "null" }
            );
            sqlx::query(&links).bind(s.link).execute(&pool).await.unwrap();
            sqlx::query("update recordings set visibility = $2, deleted_at = case when $3 then now() end where id = $1")
                .bind(s.recording).bind(visibility).bind(deleted).execute(&pool).await.unwrap();
            let got: String = sqlx::query_scalar("select playback_access_state(null, $1, $2)")
                .bind(s.video).bind(s.link).fetch_one(&pool).await.unwrap();
            assert_eq!(got, expect, "{expires} revoked={revoked} {visibility} deleted={deleted}");
        }
        // A link only ever shows the video of its own recording.
        sqlx::query("update share_links set expires_at = null, revoked_at = null").execute(&pool).await.unwrap();
        sqlx::query("update recordings set visibility = 'link', deleted_at = null where id = $1").bind(s.recording).execute(&pool).await.unwrap();
        let stranger: String = sqlx::query_scalar("select playback_access_state(null, $1, $2)")
            .bind(Uuid::new_v4()).bind(s.link).fetch_one(&pool).await.unwrap();
        assert_eq!(stranger, "access_revoked");
        // And the signed-in rule is exactly what it was.
        let owner: Uuid = sqlx::query_scalar("select id from users where email = 'teacher@x.in'").fetch_one(&pool).await.unwrap();
        let account: String = sqlx::query_scalar("select playback_access_state($1, $2, null)").bind(owner).bind(s.video).fetch_one(&pool).await.unwrap();
        assert_eq!(account, "allowed");
        let outsider: String = sqlx::query_scalar("select playback_access_state($1, $2, null)").bind(Uuid::new_v4()).bind(s.video).fetch_one(&pool).await.unwrap();
        assert_eq!(outsider, "access_revoked");
    }
}
