//! Recordings in a workspace: who sees them, who may watch them, who may put them in and take them out, and that
//! watching ends when the reason for it does.

mod common;

use axum::http::{Method, StatusCode};
use common::media::ready_video;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        $(#[$meta])*
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        async fn $name($pool: PgPool) $body
    };
}

async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("select id from users where email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A workspace with exactly these people in it, made directly: how people get in is `workspaces.rs`'s business.
async fn team(pool: &PgPool, people: &[(&str, &str)]) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("insert into workspaces (id, name) values ($1, 'Team')")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    for (email, role) in people {
        sqlx::query("insert into workspace_members (workspace_id, user_id, role) values ($1, $2, $3::workspace_role)")
            .bind(id)
            .bind(user_id(pool, email).await)
            .bind(*role)
            .execute(pool)
            .await
            .unwrap();
    }
    id
}

/// A recording with no video: enough for everything that is about the team's list and who may change it.
async fn bare_recording(app: &App, jar: &Jar, title: &str) -> Uuid {
    let res = app
        .post("/v1/recordings")
        .jar(jar)
        .json(json!({"source": "upload", "title": title}))
        .send()
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    Uuid::parse_str(res.body["recording"]["id"].as_str().unwrap()).unwrap()
}

/// A recording made of a video that is ready to play.
async fn recording_with_video(app: &App, jar: &Jar, pool: &PgPool) -> (Uuid, Uuid) {
    let video = ready_video(app, jar, pool).await;
    let res = app
        .post("/v1/recordings")
        .jar(jar)
        .json(json!({"source": "upload", "video_id": video}))
        .send()
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    (
        Uuid::parse_str(res.body["recording"]["id"].as_str().unwrap()).unwrap(),
        video,
    )
}

async fn put_in(app: &App, jar: &Jar, workspace: Uuid, recording: Uuid) -> Res {
    app.req(
        Method::PUT,
        &format!("/v1/workspaces/{workspace}/recordings/{recording}"),
    )
    .jar(jar)
    .send()
    .await
}

async fn take_out(app: &App, jar: &Jar, workspace: Uuid, recording: Uuid) -> Res {
    app.req(
        Method::DELETE,
        &format!("/v1/workspaces/{workspace}/recordings/{recording}"),
    )
    .jar(jar)
    .send()
    .await
}

async fn team_list(app: &App, jar: &Jar, workspace: Uuid, query: &str) -> Res {
    app.get(&format!("/v1/workspaces/{workspace}/recordings{query}"))
        .jar(jar)
        .send()
        .await
}

fn ids(res: &Res) -> Vec<String> {
    res.body["recordings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect()
}

async fn play(app: &App, jar: &Jar, video: Uuid) -> Res {
    app.post(&format!("/v1/videos/{video}/play"))
        .jar(jar)
        .json(json!({}))
        .send()
        .await
}

async fn heartbeat(app: &App, jar: &Jar, session: &str) -> Res {
    app.post(&format!("/v1/play/{session}/heartbeat"))
        .jar(jar)
        .json(json!({}))
        .send()
        .await
}

async fn library_version(app: &App, jar: &Jar, recording: Uuid) -> i64 {
    let res = app
        .get(&format!("/v1/recordings/{recording}"))
        .jar(jar)
        .send()
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    res.body["recording"]["version"].as_i64().unwrap()
}

db_test! {
    async fn people_in_a_workspace_see_and_watch_what_is_put_in_it_and_nothing_else(pool: PgPool) {
        let app = app(pool.clone());
        let maker = signup_creator(&app, "maker@tr.in").await;
        let mate = signup_verified_as(&app, "mate@tr.in", "student").await;
        let outsider = signup_verified_as(&app, "outsider@tr.in", "student").await;
        let ws = team(&pool, &[("maker@tr.in", "owner"), ("mate@tr.in", "member")]).await;
        let (rec, video) = recording_with_video(&app, &maker, &pool).await;
        let (_, kept_back) = recording_with_video(&app, &maker, &pool).await;

        // Before it is put in: nothing to see, and nothing to play.
        assert_eq!(team_list(&app, &mate, ws, "").await.body["recordings"], json!([]));
        let early = play(&app, &mate, video).await;
        assert_eq!((early.status, early.code()), (StatusCode::FORBIDDEN, "playback_denied"));

        let placed = put_in(&app, &maker, ws, rec).await;
        assert_eq!(placed.status, StatusCode::NO_CONTENT, "{:?}", placed.body);
        let own = app.get(&format!("/v1/recordings/{rec}")).jar(&maker).send().await;
        assert_eq!(own.body["recording"]["workspace_id"], json!(ws.to_string()), "the maker's library says where it is");

        // The mate sees what it is, who made it, and what to play.
        let listed = team_list(&app, &mate, ws, "").await;
        assert_eq!(listed.status, StatusCode::OK, "{:?}", listed.body);
        let item = &listed.body["recordings"][0];
        assert_eq!(item["id"], json!(rec.to_string()));
        assert_eq!((item["mine"].as_bool(), item["status"].as_str(), item["made_by"].as_str()), (Some(false), Some("ready"), Some("Asha Rao")));
        assert_eq!(item["video_id"], json!(video.to_string()));
        assert!(item.get("visibility").is_none() && item.get("version").is_none(), "the maker's own controls are not the team's: {item}");
        assert_eq!(team_list(&app, &maker, ws, "").await.body["recordings"][0]["mine"], true);

        // And watches it, to the end of a heartbeat.
        let started = play(&app, &mate, video).await;
        assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
        let session = started.body["playback"]["session_id"].as_str().unwrap().to_owned();
        assert_eq!(heartbeat(&app, &mate, &session).await.status, StatusCode::NO_CONTENT);
        let master = app.get(started.body["playback"]["playlist_url"].as_str().unwrap()).no_origin().send().await;
        assert_eq!(master.status, StatusCode::OK);

        // Being in the team opens what is put in it, not everything its people made.
        let other = play(&app, &mate, kept_back).await;
        assert_eq!((other.status, other.code()), (StatusCode::FORBIDDEN, "playback_denied"));
        let by_the_database: String = sqlx::query_scalar("select video_access_state($1, $2)")
            .bind(user_id(&pool, "mate@tr.in").await)
            .bind(kept_back)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(by_the_database, "access_revoked");

        // Somebody outside it sees nothing and plays nothing.
        assert_eq!(team_list(&app, &outsider, ws, "").await.status, StatusCode::NOT_FOUND);
        let denied = play(&app, &outsider, video).await;
        assert_eq!((denied.status, denied.code()), (StatusCode::FORBIDDEN, "playback_denied"));
    }
}

#[derive(Clone, Copy, Debug)]
enum Way {
    TakenOut,
    MemberRemoved,
    MemberLeaves,
    RecordingDeleted,
    WorkspaceEnded,
    MovedToAnotherTeam,
}

db_test! {
    async fn a_viewing_ends_at_its_next_request_when_the_reason_for_it_goes(pool: PgPool) {
        let app = app(pool.clone());
        // Two accounts for all of them (signing up is limited), and a team, a recording and a viewing for each way.
        let (owner_mail, mate_mail) = ("owner@tr.in", "mate@tr.in");
        let owner = signup_creator(&app, owner_mail).await;
        let mate = signup_verified_as(&app, mate_mail, "student").await;
        let mate_id = user_id(&pool, mate_mail).await;
        for way in [Way::TakenOut, Way::MemberRemoved, Way::MemberLeaves, Way::RecordingDeleted, Way::WorkspaceEnded, Way::MovedToAnotherTeam] {
            let ws = team(&pool, &[(owner_mail, "owner"), (mate_mail, "member")]).await;
            let (rec, video) = recording_with_video(&app, &owner, &pool).await;
            assert_eq!(put_in(&app, &owner, ws, rec).await.status, StatusCode::NO_CONTENT);

            let started = play(&app, &mate, video).await;
            assert_eq!(started.status, StatusCode::CREATED, "{way:?}: {:?}", started.body);
            let session = started.body["playback"]["session_id"].as_str().unwrap().to_owned();
            let master_url = started.body["playback"]["playlist_url"].as_str().unwrap().to_owned();
            assert_eq!(heartbeat(&app, &mate, &session).await.status, StatusCode::NO_CONTENT, "{way:?}: it plays while the reason holds");

            let done = match way {
                Way::TakenOut => take_out(&app, &owner, ws, rec).await,
                Way::MemberRemoved => app.req(Method::DELETE, &format!("/v1/workspaces/{ws}/members/{mate_id}")).jar(&owner).send().await,
                Way::MemberLeaves => app.req(Method::DELETE, &format!("/v1/workspaces/{ws}/members/{mate_id}")).jar(&mate).send().await,
                Way::RecordingDeleted => app.req(Method::DELETE, &format!("/v1/recordings/{rec}")).jar(&owner).send().await,
                Way::WorkspaceEnded => app.req(Method::DELETE, &format!("/v1/workspaces/{ws}")).jar(&owner).send().await,
                Way::MovedToAnotherTeam => {
                    let elsewhere = team(&pool, &[(owner_mail, "owner")]).await;
                    put_in(&app, &owner, elsewhere, rec).await
                }
            };
            assert_eq!(done.status, StatusCode::NO_CONTENT, "{way:?}: {:?}", done.body);

            // The viewing already open is over at its next request, and starting another is refused.
            let after = heartbeat(&app, &mate, &session).await;
            assert_ne!(after.status, StatusCode::NO_CONTENT, "{way:?}: still playing: {:?}", after.body);
            let master = app.get(&master_url).no_origin().send().await;
            assert_eq!(master.status, StatusCode::FORBIDDEN, "{way:?}: the playlist is still given out");
            let again = play(&app, &mate, video).await;
            assert_ne!(again.status, StatusCode::CREATED, "{way:?}: a new viewing was started: {:?}", again.body);
            let ended: Option<String> = sqlx::query_scalar("select end_reason from playback_sessions where id = $1::uuid")
                .bind(&session)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(ended.is_some(), "{way:?}: the session was never ended");

            // Whoever made it keeps it, unless it was deleted: a team's changes are never the maker's loss.
            if !matches!(way, Way::RecordingDeleted) {
                let own = play(&app, &owner, video).await;
                assert_eq!(own.status, StatusCode::CREATED, "{way:?}: the maker lost their own: {:?}", own.body);
                let session = own.body["playback"]["session_id"].as_str().unwrap();
                app.req(Method::DELETE, &format!("/v1/play/{session}")).jar(&owner).send().await;
            }
        }
    }
}

db_test! {
    async fn only_the_maker_puts_a_recording_in_and_the_managers_take_it_out(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "o@tr.in").await;
        let admin = signup_creator(&app, "a@tr.in").await;
        let maker = signup_creator(&app, "m@tr.in").await;
        let plain = signup_creator(&app, "p@tr.in").await;
        let stranger = signup_creator(&app, "s@tr.in").await;
        let ws = team(&pool, &[("o@tr.in", "owner"), ("a@tr.in", "admin"), ("m@tr.in", "member"), ("p@tr.in", "member")]).await;
        let (mine, theirs) = (bare_recording(&app, &maker, "Maker's").await, bare_recording(&app, &owner, "Owner's").await);

        // Not yours is not there; a team that is not yours is not there; and neither changes anything.
        let not_yours = put_in(&app, &plain, ws, theirs).await;
        assert_eq!((not_yours.status, not_yours.code()), (StatusCode::NOT_FOUND, "not_found"));
        let not_in_it = put_in(&app, &stranger, ws, bare_recording(&app, &stranger, "Stray").await).await;
        assert_eq!(not_in_it.status, StatusCode::NOT_FOUND, "{:?}", not_in_it.body);
        assert_eq!(team_list(&app, &owner, ws, "").await.body["recordings"], json!([]));
        assert_eq!(put_in(&app, &maker, Uuid::new_v4(), mine).await.status, StatusCode::NOT_FOUND);
        assert_eq!(put_in(&app, &maker, ws, Uuid::new_v4()).await.status, StatusCode::NOT_FOUND);

        // Putting in is repeatable and does not make the maker's copy look edited again.
        assert_eq!(put_in(&app, &maker, ws, mine).await.status, StatusCode::NO_CONTENT);
        let version = library_version(&app, &maker, mine).await;
        assert_eq!(put_in(&app, &maker, ws, mine).await.status, StatusCode::NO_CONTENT);
        assert_eq!(library_version(&app, &maker, mine).await, version);
        assert_eq!(put_in(&app, &owner, ws, theirs).await.status, StatusCode::NO_CONTENT);

        // A member may not take out what someone else made; the stranger is told there is no such team.
        let may_not = take_out(&app, &plain, ws, mine).await;
        assert_eq!((may_not.status, may_not.code()), (StatusCode::FORBIDDEN, "forbidden"));
        assert_eq!(take_out(&app, &stranger, ws, mine).await.status, StatusCode::NOT_FOUND);
        assert_eq!(ids(&team_list(&app, &plain, ws, "").await).len(), 2);

        // Its maker may, and so may an admin or an owner; it is then in nobody's team and still the maker's.
        for (who, label) in [(&admin, "admin"), (&owner, "owner"), (&maker, "maker")] {
            assert_eq!(put_in(&app, &maker, ws, mine).await.status, StatusCode::NO_CONTENT);
            let out = take_out(&app, who, ws, mine).await;
            assert_eq!(out.status, StatusCode::NO_CONTENT, "{label}: {:?}", out.body);
            assert_eq!(ids(&team_list(&app, &plain, ws, "").await), vec![theirs.to_string()], "{label}");
            let own = app.get(&format!("/v1/recordings/{mine}")).jar(&maker).send().await;
            assert_eq!(own.status, StatusCode::OK, "{label}: the maker still has it");
            assert!(own.body["recording"]["workspace_id"].is_null(), "{label}");
        }
        let twice = take_out(&app, &maker, ws, mine).await;
        assert_eq!((twice.status, twice.code()), (StatusCode::NOT_FOUND, "not_found"), "taking out what is not in");

        // In one team at a time: putting it in another moves it.
        let second = team(&pool, &[("m@tr.in", "owner")]).await;
        assert_eq!(put_in(&app, &maker, ws, mine).await.status, StatusCode::NO_CONTENT);
        assert_eq!(put_in(&app, &maker, second, mine).await.status, StatusCode::NO_CONTENT);
        assert_eq!(ids(&team_list(&app, &maker, second, "").await), vec![mine.to_string()]);
        assert_eq!(ids(&team_list(&app, &plain, ws, "").await), vec![theirs.to_string()]);

        // A deleted recording is gone from the team, and cannot be put back.
        assert_eq!(put_in(&app, &maker, ws, mine).await.status, StatusCode::NO_CONTENT);
        assert_eq!(app.req(Method::DELETE, &format!("/v1/recordings/{mine}")).jar(&maker).send().await.status, StatusCode::NO_CONTENT);
        assert_eq!(ids(&team_list(&app, &plain, ws, "").await), vec![theirs.to_string()]);
        assert_eq!(put_in(&app, &maker, ws, mine).await.status, StatusCode::NOT_FOUND);

        // Somebody who leaves the team leaves what they made in it: it is the team's to see, still theirs to change.
        let kept = bare_recording(&app, &maker, "Kept").await;
        assert_eq!(put_in(&app, &maker, ws, kept).await.status, StatusCode::NO_CONTENT);
        let maker_id = user_id(&pool, "m@tr.in").await;
        let removed = app.req(Method::DELETE, &format!("/v1/workspaces/{ws}/members/{maker_id}")).jar(&owner).send().await;
        assert_eq!(removed.status, StatusCode::NO_CONTENT, "{:?}", removed.body);
        assert_eq!(ids(&team_list(&app, &plain, ws, "").await).len(), 2);
        assert_eq!(team_list(&app, &maker, ws, "").await.status, StatusCode::NOT_FOUND, "but they no longer see the team");
        assert_eq!(put_in(&app, &maker, ws, kept).await.status, StatusCode::NOT_FOUND, "and cannot add to it");
    }
}

db_test! {
    async fn the_teams_list_goes_newest_first_a_page_at_a_time(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "o@tr.in").await;
        let ws = team(&pool, &[("o@tr.in", "owner")]).await;
        let mut made = Vec::new();
        for n in 0..5 {
            let id = bare_recording(&app, &owner, &format!("Recording {n}")).await;
            assert_eq!(put_in(&app, &owner, ws, id).await.status, StatusCode::NO_CONTENT);
            made.push(id.to_string());
        }
        made.reverse();

        let mut seen = Vec::new();
        let mut query = "?limit=2".to_owned();
        let mut pages = 0;
        loop {
            let page = team_list(&app, &owner, ws, &query).await;
            assert_eq!(page.status, StatusCode::OK, "{:?}", page.body);
            seen.extend(ids(&page));
            pages += 1;
            match page.body["next_cursor"].as_str() {
                Some(cursor) => query = format!("?limit=2&cursor={cursor}"),
                None => break,
            }
        }
        assert_eq!((pages, seen), (3, made), "every recording once, newest first");

        for (query, want) in [("?limit=0", 1), ("?limit=500", 5)] {
            assert_eq!(ids(&team_list(&app, &owner, ws, query).await).len(), want, "{query}");
        }
        let bad = team_list(&app, &owner, ws, "?cursor=not-a-cursor").await;
        assert_eq!((bad.status, bad.code()), (StatusCode::BAD_REQUEST, "invalid_cursor"));
        assert_eq!(team_list(&app, &owner, Uuid::new_v4(), "").await.status, StatusCode::NOT_FOUND);
    }
}

db_test! {
    async fn somebody_being_removed_cannot_put_something_in_at_that_moment(pool: PgPool) {
        let app = app(pool.clone());
        signup_creator(&app, "o@tr.in").await;
        let leaving = signup_creator(&app, "l@tr.in").await;
        let ws = team(&pool, &[("o@tr.in", "owner"), ("l@tr.in", "member")]).await;
        let rec = bare_recording(&app, &leaving, "Last words").await;
        let leaving_id = user_id(&pool, "l@tr.in").await;

        // A removal is under way: it holds the workspace, and has not yet taken the member out. The put arrives now.
        let mut removal = pool.begin().await.unwrap();
        sqlx::query("select 1 from workspaces where id = $1 for update").bind(ws).execute(&mut *removal).await.unwrap();
        let (res, _) = tokio::join!(put_in(&app, &leaving, ws, rec), async {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            sqlx::query("delete from workspace_members where workspace_id = $1 and user_id = $2")
                .bind(ws)
                .bind(leaving_id)
                .execute(&mut *removal)
                .await
                .unwrap();
            removal.commit().await.unwrap();
        });
        assert_eq!(res.status, StatusCode::NOT_FOUND, "the put went through a removal: {:?}", res.body);
        let placed: Option<Uuid> = sqlx::query_scalar("select workspace_id from recordings where id = $1")
            .bind(rec)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(placed, None);
    }
}

db_test! {
    async fn the_rule_in_the_database_has_one_condition_for_each_way_in(pool: PgPool) {
        let app = app(pool.clone());
        let maker = signup_creator(&app, "maker@tr.in").await;
        signup_verified_as(&app, "mate@tr.in", "student").await;
        let mate = user_id(&pool, "mate@tr.in").await;
        let ws = team(&pool, &[("maker@tr.in", "owner"), ("mate@tr.in", "member")]).await;
        let (rec, video) = recording_with_video(&app, &maker, &pool).await;
        let state = || {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>("select video_access_state($1, $2)")
                    .bind(mate)
                    .bind(video)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(state().await, "access_revoked", "in no team yet");

        sqlx::query("update recordings set workspace_id = $1 where id = $2").bind(ws).bind(rec).execute(&pool).await.unwrap();
        assert_eq!(state().await, "allowed", "in the team");

        sqlx::query("update workspaces set deleted_at = now() where id = $1").bind(ws).execute(&pool).await.unwrap();
        assert_eq!(state().await, "access_revoked", "the team was ended");
        sqlx::query("update workspaces set deleted_at = null where id = $1").bind(ws).execute(&pool).await.unwrap();

        sqlx::query("update recordings set deleted_at = now() where id = $1").bind(rec).execute(&pool).await.unwrap();
        assert_eq!(state().await, "access_revoked", "the recording was deleted");
        sqlx::query("update recordings set deleted_at = null where id = $1").bind(rec).execute(&pool).await.unwrap();
        assert_eq!(state().await, "allowed");

        sqlx::query("delete from workspace_members where workspace_id = $1 and user_id = $2").bind(ws).bind(mate).execute(&pool).await.unwrap();
        assert_eq!(state().await, "access_revoked", "no longer in the team");
        sqlx::query("insert into workspace_members (workspace_id, user_id, role) values ($1, $2, 'member')").bind(ws).bind(mate).execute(&pool).await.unwrap();

        // A video that is in the team's recording is the only one opened: another of the maker's stays shut.
        let other = ready_video(&app, &maker, &pool).await;
        let shut: String = sqlx::query_scalar("select video_access_state($1, $2)").bind(mate).bind(other).fetch_one(&pool).await.unwrap();
        assert_eq!(shut, "access_revoked");
    }
}
