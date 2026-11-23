//! Comments and reactions on a shared recording: who can read, who can leave one, who can remove one, and the
//! limits that keep one account or one video from taking over a page.

mod common;

use axum::http::{header, Method, StatusCode};
use common::media::ready_video;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

/// A shared recording of a real video of 600 seconds, with one open link.
struct Shared {
    app: App,
    pool: PgPool,
    owner: Jar,
    recording: Uuid,
    code: String,
}

async fn shared_with(pool: PgPool, link: Value) -> Shared {
    let app = app(pool.clone());
    let owner = signup_creator(&app, "teacher@x.in").await;
    let video = ready_video(&app, &owner, &pool).await;
    let made = app
        .post("/v1/recordings")
        .jar(&owner)
        .json(json!({"source": "extension", "title": "Week 3", "video_id": video}))
        .send()
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
    let recording = Uuid::parse_str(made.body["recording"]["id"].as_str().unwrap()).unwrap();
    let made = app
        .post(&format!("/v1/recordings/{recording}/links"))
        .jar(&owner)
        .json(link)
        .send()
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
    Shared {
        app,
        pool,
        owner,
        recording,
        code: made.body["link"]["code"].as_str().unwrap().to_owned(),
    }
}

async fn shared(pool: PgPool) -> Shared {
    shared_with(pool, json!({})).await
}

impl Shared {
    async fn read(&self, jar: Option<&Jar>) -> Res {
        let mut req = self.app.get(&format!("/v1/s/{}/comments", self.code));
        if let Some(jar) = jar {
            req = req.jar(jar);
        }
        req.send().await
    }

    async fn leave(&self, jar: &Jar, body: Value) -> Res {
        self.app
            .post(&format!("/v1/s/{}/comments", self.code))
            .jar(jar)
            .json(body)
            .send()
            .await
    }

    async fn remove(&self, jar: &Jar, id: &str) -> Res {
        self.app
            .req(Method::DELETE, &format!("/v1/comments/{id}"))
            .jar(jar)
            .send()
            .await
    }

    async fn owner_list(&self, jar: &Jar) -> Res {
        self.app
            .get(&format!("/v1/recordings/{}/comments", self.recording))
            .jar(jar)
            .send()
            .await
    }

    async fn count(&self) -> i64 {
        count(&self.pool, "select count(*) from recording_comments").await
    }
}

fn comment(body: &str, at_ms: Option<i64>) -> Value {
    json!({"kind": "comment", "body": body, "at_ms": at_ms})
}

fn reaction(emoji: &str, at_ms: i64) -> Value {
    json!({"kind": "reaction", "emoji": emoji, "at_ms": at_ms})
}

fn ids(res: &Res) -> Vec<String> {
    res.body["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect()
}

db_test! {
    async fn anyone_with_the_link_reads_and_a_signed_in_person_leaves_comments_and_reactions(pool: PgPool) {
        let s = shared(pool).await;
        let pupil = signup_verified_as(&s.app, "pupil@x.in", "student").await;

        let empty = s.read(None).await;
        assert_eq!(empty.status, StatusCode::OK, "{:?}", empty.body);
        assert_eq!(empty.body, json!({"comments": [], "next_cursor": null, "comments_enabled": true}));
        assert_eq!(empty.headers[header::CACHE_CONTROL], "private, no-store", "it carries what the asker may delete");

        let said = s.leave(&pupil, comment("The part at the start is the one I mean", Some(12_000))).await;
        assert_eq!(said.status, StatusCode::CREATED, "{:?}", said.body);
        let reacted = s.leave(&pupil, reaction("🔥", 45_500)).await;
        assert_eq!(reacted.status, StatusCode::CREATED, "{:?}", reacted.body);
        assert_eq!(said.body["comment"]["author"]["name"], "Asha Rao", "{:?}", said.body);
        assert_eq!((said.body["comment"]["mine"].clone(), said.body["comment"]["can_delete"].clone()), (json!(true), json!(true)));

        // A stranger with no account sees both, newest first, and nothing is theirs to remove.
        let seen = s.read(None).await;
        assert_eq!(seen.body["comments"].as_array().unwrap().len(), 2);
        assert_eq!(seen.body["comments"][0]["emoji"], "🔥");
        assert_eq!(seen.body["comments"][1]["body"], "The part at the start is the one I mean");
        assert_eq!(seen.body["comments"][1]["at_ms"], 12_000);
        assert_eq!(seen.body["comments"][0]["can_delete"], false);
        assert!(!seen.body.to_string().contains("pupil@x.in"), "an email is never shown");

        // The author sees them as theirs; the owner may remove them.
        let mine = s.read(Some(&pupil)).await;
        assert_eq!((mine.body["comments"][0]["mine"].clone(), mine.body["comments"][0]["can_delete"].clone()), (json!(true), json!(true)));
        let owners = s.read(Some(&s.owner)).await;
        assert_eq!((owners.body["comments"][0]["mine"].clone(), owners.body["comments"][0]["can_delete"].clone()), (json!(false), json!(true)));
    }
}

db_test! {
    async fn what_is_left_must_be_words_or_a_listed_reaction_at_a_moment_that_exists(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        let b = signup_verified_as(&s.app, "b@x.in", "student").await;
        for (body, field) in [
            (json!({"kind": "shout", "body": "x"}), "kind"),
            (comment("   ", None), "body"),
            (comment(&"x".repeat(1001), None), "body"),
            (comment("fine", Some(602_000)), "at_ms"),
            (comment("fine", Some(-1)), "at_ms"),
            (reaction("🦄", 5), "emoji"),
            (json!({"kind": "reaction", "emoji": "👍"}), "at_ms"),
            (json!({"kind": "reaction", "emoji": "👍", "body": "words", "at_ms": 1}), "body"),
            (json!({"kind": "comment", "body": "x", "emoji": "👍"}), "emoji"),
        ] {
            let res = s.leave(&a, body.clone()).await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {:?}", res.body);
            assert!(res.body["error"]["fields"].to_string().contains(field), "{body}: {:?}", res.body);
        }
        // The last second of the video, and the whole video, are both fine.
        // (Every try counts against the writing limit, so these come from another person.)
        assert_eq!(s.leave(&b, comment("at the very end", Some(601_000))).await.status, StatusCode::CREATED);
        assert_eq!(s.leave(&b, comment("about all of it", None)).await.status, StatusCode::CREATED);
        assert_eq!(s.count().await, 2);
    }
}

db_test! {
    async fn only_a_signed_in_person_can_leave_one_and_only_from_a_page_of_this_site(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        let nobody = s.app.post(&format!("/v1/s/{}/comments", s.code)).json(comment("hi", None)).send().await;
        assert_eq!((nobody.status, nobody.code()), (StatusCode::UNAUTHORIZED, "unauthenticated"));
        let foreign = s.app.post(&format!("/v1/s/{}/comments", s.code)).jar(&a).origin("https://evil.example").json(comment("hi", None)).send().await;
        assert_eq!((foreign.status, foreign.code()), (StatusCode::FORBIDDEN, "forbidden_origin"));
        assert_eq!(s.count().await, 0);
        // Reading asks for nothing.
        assert_eq!(s.read(None).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn a_post_that_is_retried_after_a_lost_answer_is_left_once(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        let b = signup_verified_as(&s.app, "b@x.in", "student").await;
        let id = Uuid::new_v4();
        let body = json!({"id": id, "kind": "comment", "body": "once", "at_ms": 5});
        let first = s.leave(&a, body.clone()).await;
        let again = s.leave(&a, body.clone()).await;
        assert_eq!((first.status, again.status), (StatusCode::CREATED, StatusCode::CREATED), "{:?} {:?}", first.body, again.body);
        assert_eq!(first.body["comment"]["id"], id.to_string());
        assert_eq!(first.body["comment"]["id"], again.body["comment"]["id"]);
        assert_eq!(s.count().await, 1);

        // Changed words, somebody else's, or another video's under the same id: refused, and nothing changes.
        let changed = s.leave(&a, json!({"id": id, "kind": "comment", "body": "twice", "at_ms": 5})).await;
        assert_eq!((changed.status, changed.code()), (StatusCode::CONFLICT, "comment_id_conflict"));
        let stolen = s.leave(&b, body).await;
        assert_eq!((stolen.status, stolen.code()), (StatusCode::CONFLICT, "comment_id_conflict"));

        // Two at once make one.
        let racing = Uuid::new_v4();
        let both = json!({"id": racing, "kind": "reaction", "emoji": "👏", "at_ms": 9});
        let (x, y) = tokio::join!(s.leave(&a, both.clone()), s.leave(&a, both));
        assert_eq!((x.status, y.status), (StatusCode::CREATED, StatusCode::CREATED), "{:?} {:?}", x.body, y.body);
        assert_eq!(s.count().await, 2);
    }
}

db_test! {
    async fn the_author_and_the_owner_can_remove_a_comment_and_nobody_else(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        let b = signup_verified_as(&s.app, "b@x.in", "student").await;
        let other_owner = signup_creator(&s.app, "other@x.in").await;
        let mut made = Vec::new();
        for text in ["one", "two", "three"] {
            made.push(s.leave(&a, comment(text, None)).await.body["comment"]["id"].as_str().unwrap().to_owned());
        }

        // To everyone else it does not exist: not "forbidden", which would say it does.
        for stranger in [&b, &other_owner] {
            let res = s.remove(stranger, &made[0]).await;
            assert_eq!((res.status, res.code()), (StatusCode::NOT_FOUND, "not_found"));
        }
        assert_eq!(s.remove_unsigned(&made[0]).await, StatusCode::UNAUTHORIZED);
        assert_eq!(s.count().await, 3);

        assert_eq!(s.remove(&a, &made[0]).await.status, StatusCode::NO_CONTENT, "the author");
        assert_eq!(s.remove(&s.owner, &made[1]).await.status, StatusCode::NO_CONTENT, "the owner of the recording");
        assert_eq!(s.remove(&a, &made[0]).await.status, StatusCode::NOT_FOUND, "already gone");
        assert_eq!(ids(&s.read(None).await), [made[2].clone()]);
        assert_eq!(s.remove(&s.owner, "not-a-uuid").await.status, StatusCode::NOT_FOUND);
    }
}

impl Shared {
    async fn remove_unsigned(&self, id: &str) -> StatusCode {
        self.app
            .req(Method::DELETE, &format!("/v1/comments/{id}"))
            .send()
            .await
            .status
    }
}

db_test! {
    async fn the_owner_can_close_the_conversation_and_reopen_it(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        s.leave(&a, comment("before", None)).await;

        let rec = format!("/v1/recordings/{}", s.recording);
        let version = s.app.get(&rec).jar(&s.owner).send().await.body["recording"]["version"].clone();
        assert_eq!(s.app.get(&rec).jar(&s.owner).send().await.body["recording"]["comments_enabled"], true);
        let closed = s.app.req(Method::PATCH, &rec).jar(&s.owner).json(json!({"version": version, "comments_enabled": false})).send().await;
        assert_eq!(closed.status, StatusCode::OK, "{:?}", closed.body);
        assert_eq!(closed.body["recording"]["comments_enabled"], false);

        // Viewers see an empty, closed conversation and cannot add to it; what was there is kept.
        let read = s.read(None).await;
        assert_eq!(read.body, json!({"comments": [], "next_cursor": null, "comments_enabled": false}));
        let refused = s.leave(&a, comment("after", None)).await;
        assert_eq!((refused.status, refused.code()), (StatusCode::CONFLICT, "comments_off"));
        let owners = s.owner_list(&s.owner).await;
        assert_eq!(owners.body["comments_enabled"], false);
        assert_eq!(owners.body["comments"].as_array().unwrap().len(), 1, "the owner still sees what is there");

        let version = closed.body["recording"]["version"].clone();
        let opened = s.app.req(Method::PATCH, &rec).jar(&s.owner).json(json!({"version": version, "comments_enabled": true})).send().await;
        assert_eq!(opened.status, StatusCode::OK, "{:?}", opened.body);
        assert_eq!(s.read(None).await.body["comments"].as_array().unwrap().len(), 1);
        assert_eq!(s.leave(&a, comment("after", None)).await.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn a_link_with_a_password_shows_no_comments_to_anyone_who_has_not_unlocked_it(pool: PgPool) {
        let s = shared_with(pool, json!({"password": "open sesame"})).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;

        let locked = s.read(None).await;
        assert_eq!((locked.status, locked.code()), (StatusCode::FORBIDDEN, "password_required"));
        let refused = s.leave(&a, comment("let me in", None)).await;
        assert_eq!((refused.status, refused.code()), (StatusCode::FORBIDDEN, "password_required"));
        let junk = s.app.get(&format!("/v1/s/{}/comments?t=garbage", s.code)).send().await;
        assert_eq!((junk.status, junk.code()), (StatusCode::FORBIDDEN, "password_required"));

        // A viewing that gave the password is the proof.
        let played = s.app.post(&format!("/v1/s/{}/play", s.code)).json(json!({"password": "open sesame"})).send().await;
        assert_eq!(played.status, StatusCode::CREATED, "{:?}", played.body);
        let url = played.body["playback"]["playlist_url"].as_str().unwrap();
        let token = url.split("?t=").nth(1).unwrap().to_owned();
        let session = played.body["playback"]["session_id"].as_str().unwrap().to_owned();
        let open = s.app.get(&format!("/v1/s/{}/comments?t={token}", s.code)).send().await;
        assert_eq!(open.status, StatusCode::OK, "{:?}", open.body);
        let left = s.app.post(&format!("/v1/s/{}/comments?t={token}", s.code)).jar(&a).json(comment("now I can", None)).send().await;
        assert_eq!(left.status, StatusCode::CREATED, "{:?}", left.body);

        // A viewing of another link proves nothing here, and neither does one that has ended.
        let other = s.app.post(&format!("/v1/recordings/{}/links", s.recording)).jar(&s.owner).json(json!({"password": "another one"})).send().await;
        let other_code = other.body["link"]["code"].as_str().unwrap().to_owned();
        let other_play = s.app.post(&format!("/v1/s/{other_code}/play")).json(json!({"password": "another one"})).send().await;
        let other_token = other_play.body["playback"]["playlist_url"].as_str().unwrap().split("?t=").nth(1).unwrap().to_owned();
        let borrowed = s.app.get(&format!("/v1/s/{}/comments?t={other_token}", s.code)).send().await;
        assert_eq!((borrowed.status, borrowed.code()), (StatusCode::FORBIDDEN, "password_required"));
        sqlx::query("update playback_sessions set ended_at = now(), end_reason = 'access_revoked' where id = $1::uuid").bind(&session).execute(&s.pool).await.unwrap();
        let ended = s.app.get(&format!("/v1/s/{}/comments?t={token}", s.code)).send().await;
        assert_eq!((ended.status, ended.code()), (StatusCode::FORBIDDEN, "password_required"));

        // The owner has no need of the password to see what is said about their own recording.
        assert_eq!(s.owner_list(&s.owner).await.body["comments"].as_array().unwrap().len(), 1);
    }
}

db_test! {
    async fn a_closed_link_shows_nothing_but_the_conversation_outlives_it(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        s.leave(&a, comment("still here later", None)).await;

        let links = s.app.get(&format!("/v1/recordings/{}/links", s.recording)).jar(&s.owner).send().await;
        let link = links.body["links"][0]["id"].as_str().unwrap().to_owned();
        let revoked = s.app.req(Method::DELETE, &format!("/v1/recordings/{}/links/{link}", s.recording)).jar(&s.owner).send().await;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);
        for res in [s.read(None).await, s.leave(&a, comment("too late", None)).await] {
            assert_eq!((res.status, res.code()), (StatusCode::GONE, "link_unavailable"));
        }
        assert_eq!(s.owner_list(&s.owner).await.body["comments"].as_array().unwrap().len(), 1, "the owner's view is not the link's");

        // A new link shows the same conversation.
        let again = s.app.post(&format!("/v1/recordings/{}/links", s.recording)).jar(&s.owner).json(json!({})).send().await;
        let code = again.body["link"]["code"].as_str().unwrap();
        let read = s.app.get(&format!("/v1/s/{code}/comments")).send().await;
        assert_eq!(read.body["comments"].as_array().unwrap().len(), 1);
        assert_eq!(s.app.get("/v1/s/nonsense/comments").send().await.status, StatusCode::NOT_FOUND);
    }
}

db_test! {
    async fn nobody_can_take_over_a_page_one_person_and_one_video_have_limits(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        let b = signup_verified_as(&s.app, "b@x.in", "student").await;
        let author: Uuid = sqlx::query_scalar("select id from users where email = 'a@x.in'").fetch_one(&s.pool).await.unwrap();
        let filler = |author: Uuid, n: i64| {
            let pool = s.pool.clone();
            let recording = s.recording;
            async move {
                sqlx::query("insert into recording_comments (id, recording_id, author_id, kind, body) select gen_random_uuid(), $1, $2, 'comment', 'filler' from generate_series(1, $3)")
                    .bind(recording).bind(author).bind(n).execute(&pool).await.unwrap();
            }
        };

        filler(author, 200).await;
        let one = s.leave(&a, comment("the 201st", None)).await;
        assert_eq!((one.status, one.code()), (StatusCode::CONFLICT, "too_many_comments"));
        assert_eq!(s.leave(&b, comment("somebody else is not stopped", None)).await.status, StatusCode::CREATED);

        // A video holds only so many, whoever leaves them: nine more people at the limit make 2000.
        sqlx::query("insert into users (name, email, password_hash, role) select 'Filler', 'filler' || n || '@x.in', 'x', 'student' from generate_series(1, 9) n")
            .execute(&s.pool).await.unwrap();
        sqlx::query("insert into recording_comments (id, recording_id, author_id, kind, body) select gen_random_uuid(), $1, u.id, 'comment', 'filler' from users u cross join generate_series(1, 200) where u.email like 'filler%'")
            .bind(s.recording).execute(&s.pool).await.unwrap();
        let full = s.leave(&b, comment("one more", None)).await;
        assert_eq!((full.status, full.code()), (StatusCode::CONFLICT, "comments_full"), "{:?}", full.body);
    }
}

db_test! {
    async fn a_person_who_posts_too_fast_is_slowed_down_and_told_when_to_try_again(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        let mut slowed = None;
        for n in 0..15 {
            let res = s.leave(&a, comment(&format!("comment {n}"), None)).await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                slowed = Some((n, res));
                break;
            }
        }
        let (n, res) = slowed.expect("ten a minute is the limit");
        assert_eq!(n, 10);
        assert!(res.headers.get(header::RETRY_AFTER).is_some());
        // Reading is not held up by it, and somebody else is not affected.
        assert_eq!(s.read(Some(&a)).await.status, StatusCode::OK);
        let b = signup_verified_as(&s.app, "b@x.in", "student").await;
        assert_eq!(s.leave(&b, comment("fine", None)).await.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn the_owner_lists_their_own_conversation_a_page_at_a_time_and_nobody_else_may(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        let mut made = Vec::new();
        for n in 0..4 {
            made.push(s.leave(&a, comment(&format!("c{n}"), Some(n * 1000))).await.body["comment"]["id"].as_str().unwrap().to_owned());
        }
        made.reverse();

        let page = s.app.get(&format!("/v1/recordings/{}/comments?limit=3", s.recording)).jar(&s.owner).send().await;
        assert_eq!(ids(&page), made[..3]);
        let cursor = page.body["next_cursor"].as_str().expect("a second page").to_owned();
        let next = s.app.get(&format!("/v1/recordings/{}/comments?limit=3&cursor={cursor}", s.recording)).jar(&s.owner).send().await;
        assert_eq!(ids(&next), made[3..]);
        assert!(next.body["next_cursor"].is_null());
        // The same through the link.
        let public = s.app.get(&format!("/v1/s/{}/comments?limit=3", s.code)).send().await;
        assert_eq!(ids(&public), made[..3]);

        let bad = s.app.get(&format!("/v1/recordings/{}/comments?cursor=garbage", s.recording)).jar(&s.owner).send().await;
        assert_eq!(bad.code(), "invalid_cursor");
        let huge = s.app.get(&format!("/v1/s/{}/comments?limit=500", s.code)).send().await;
        assert_eq!(huge.status, StatusCode::UNPROCESSABLE_ENTITY);

        let other = signup_creator(&s.app, "other@x.in").await;
        assert_eq!(s.owner_list(&other).await.status, StatusCode::NOT_FOUND);
        assert_eq!(s.owner_list(&a).await.status, StatusCode::FORBIDDEN, "a student keeps no library");
        assert_eq!(s.app.get(&format!("/v1/recordings/{}/comments", s.recording)).send().await.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn deleting_the_recording_or_the_account_takes_the_comments_with_it(pool: PgPool) {
        let s = shared(pool).await;
        let a = signup_verified_as(&s.app, "a@x.in", "student").await;
        s.leave(&a, comment("mine", None)).await;
        let author: Uuid = sqlx::query_scalar("select id from users where email = 'a@x.in'").fetch_one(&s.pool).await.unwrap();
        sqlx::query("delete from users where id = $1").bind(author).execute(&s.pool).await.unwrap();
        assert_eq!(s.count().await, 0, "no comment without somebody behind it");

        let b = signup_verified_as(&s.app, "b@x.in", "student").await;
        s.leave(&b, comment("on a recording that goes", None)).await;
        let gone = s.app.req(Method::DELETE, &format!("/v1/recordings/{}", s.recording)).jar(&s.owner).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        let read = s.read(None).await;
        assert_eq!((read.status, read.code()), (StatusCode::GONE, "link_unavailable"));
    }
}
