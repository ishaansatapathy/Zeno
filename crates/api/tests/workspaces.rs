//! Workspaces: who is in one, what each may do, and the invitations that bring people in.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use futures_util::future::join_all;
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

async fn make(app: &App, jar: &Jar, name: &str) -> Uuid {
    let res = app
        .post("/v1/workspaces")
        .jar(jar)
        .json(json!({"name": name}))
        .send()
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    Uuid::parse_str(res.body["workspace"]["id"].as_str().unwrap()).unwrap()
}

async fn invite(app: &App, jar: &Jar, workspace: Uuid, email: &str, role: &str) -> Res {
    app.post(&format!("/v1/workspaces/{workspace}/invites"))
        .jar(jar)
        .json(json!({"email": email, "role": role}))
        .send()
        .await
}

async fn accept(app: &App, jar: &Jar, token: &str) -> Res {
    app.post("/v1/invites/accept")
        .jar(jar)
        .json(json!({"token": token}))
        .send()
        .await
}

/// The token in the newest email sent to `email`.
async fn token_sent_to(app: &App, email: &str) -> String {
    app.settle().await;
    let mail = app
        .mailer
        .sent()
        .into_iter()
        .rev()
        .find(|m| m.to == email && m.subject.contains("invited you"))
        .unwrap_or_else(|| panic!("no invitation was sent to {email}"));
    token_from(&mail)
}

/// Invites `email`, makes the account (a stranger until now) and has it accept. Returns its session.
async fn join(app: &App, owner: &Jar, workspace: Uuid, email: &str, role: &str) -> Jar {
    let sent = invite(app, owner, workspace, email, role).await;
    assert_eq!(sent.status, StatusCode::CREATED, "{:?}", sent.body);
    // The invitation is delivered in the background: let it land before the sign-up mail is waited for, or the
    // two could be taken for each other.
    app.settle().await;
    let jar = signup_verified(app, email).await;
    let token = token_sent_to(app, email).await;
    let joined = accept(app, &jar, &token).await;
    assert_eq!(joined.status, StatusCode::OK, "{:?}", joined.body);
    jar
}

async fn show(app: &App, jar: &Jar, workspace: Uuid) -> Res {
    app.get(&format!("/v1/workspaces/{workspace}"))
        .jar(jar)
        .send()
        .await
}

async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("select id from users where email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn owners(pool: &PgPool, workspace: Uuid) -> i64 {
    sqlx::query_scalar(
        "select count(*) from workspace_members where workspace_id = $1 and role = 'owner'",
    )
    .bind(workspace)
    .fetch_one(pool)
    .await
    .unwrap()
}

db_test! {
    async fn a_person_starts_a_workspace_and_is_its_owner(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        assert_eq!(app.get("/v1/workspaces").jar(&asha).send().await.body["workspaces"], json!([]));

        let res = app.post("/v1/workspaces").jar(&asha).json(json!({"name": "  Maths 9A "})).send().await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        assert_eq!(res.body["workspace"]["name"], "Maths 9A");
        assert_eq!(res.body["workspace"]["role"], "owner");
        assert_eq!(res.body["workspace"]["member_count"], 1);
        let id = Uuid::parse_str(res.body["workspace"]["id"].as_str().unwrap()).unwrap();

        let listed = app.get("/v1/workspaces").jar(&asha).send().await;
        assert_eq!(listed.body["workspaces"].as_array().unwrap().len(), 1);
        let detail = show(&app, &asha, id).await;
        assert_eq!(detail.status, StatusCode::OK);
        let members = detail.body["members"].as_array().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!((members[0]["role"].as_str(), members[0]["you"].as_bool(), members[0]["email"].as_str()), (Some("owner"), Some(true), Some("asha@ws.in")));

        for (name, why) in [("", "empty"), ("x", "one character"), ("   ", "blank"), ("a\u{0000}b", "a control character")] {
            let res = app.post("/v1/workspaces").jar(&asha).json(json!({"name": name})).send().await;
            assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {:?}", res.body);
        }
        assert_eq!(app.post("/v1/workspaces").json(json!({"name": "Nobody"})).send().await.status, StatusCode::UNAUTHORIZED);
    }
}

db_test! {
    async fn an_account_owns_at_most_five_even_when_it_asks_for_them_all_at_once(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let answers = join_all((0..8).map(|n| app.post("/v1/workspaces").jar(&asha).json(json!({"name": format!("Team {n}")})).send())).await;
        let made = answers.iter().filter(|r| r.status == StatusCode::CREATED).count();
        assert_eq!(made, 5, "{:?}", answers.iter().map(|r| (r.status, r.code().to_owned())).collect::<Vec<_>>());
        assert!(answers.iter().filter(|r| r.status != StatusCode::CREATED).all(|r| r.code() == "too_many_workspaces"));
        // Belonging to other people's is not limited, and a workspace given up makes room.
        let ben = signup_verified(&app, "ben@ws.in").await;
        let theirs = make(&app, &ben, "Ben's").await;
        let sent = invite(&app, &ben, theirs, "asha@ws.in", "member").await;
        assert_eq!(sent.status, StatusCode::CREATED);
        let token = token_sent_to(&app, "asha@ws.in").await;
        assert_eq!(accept(&app, &asha, &token).await.status, StatusCode::OK);
        assert_eq!(app.get("/v1/workspaces").jar(&asha).send().await.body["workspaces"].as_array().unwrap().len(), 6);
    }
}

db_test! {
    async fn somebody_who_is_not_in_a_workspace_cannot_tell_it_exists(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let stranger = signup_verified(&app, "ben@ws.in").await;
        let id = make(&app, &asha, "Private").await;
        let nobody = Uuid::new_v4();
        let owner = user_id(&pool, "asha@ws.in").await;

        for (method, path, body) in [
            (Method::GET, format!("/v1/workspaces/{id}"), None),
            (Method::PATCH, format!("/v1/workspaces/{id}"), Some(json!({"name": "Mine now"}))),
            (Method::DELETE, format!("/v1/workspaces/{id}"), None),
            (Method::GET, format!("/v1/workspaces/{id}/invites"), None),
            (Method::POST, format!("/v1/workspaces/{id}/invites"), Some(json!({"email": "x@ws.in", "role": "member"}))),
            (Method::PATCH, format!("/v1/workspaces/{id}/members/{owner}"), Some(json!({"role": "member"}))),
            (Method::DELETE, format!("/v1/workspaces/{id}/members/{owner}"), None),
        ] {
            let there = {
                let mut req = app.req(method.clone(), &path).jar(&stranger);
                if let Some(body) = body.clone() { req = req.json(body); }
                req.send().await
            };
            let gone = {
                let swapped = path.replace(&id.to_string(), &nobody.to_string());
                let mut req = app.req(method.clone(), &swapped).jar(&stranger);
                if let Some(body) = body.clone() { req = req.json(body); }
                req.send().await
            };
            assert_eq!(there.status, StatusCode::NOT_FOUND, "{method} {path}: {:?}", there.body);
            assert_eq!(there.status, gone.status, "{method} {path} answers as a workspace that is not there");
            assert_eq!(there.body["error"]["code"], gone.body["error"]["code"]);
        }
        let junk = app.get("/v1/workspaces/not-a-uuid").jar(&stranger).send().await;
        assert_eq!(junk.status, StatusCode::NOT_FOUND);
        // And none of it did anything.
        assert_eq!(show(&app, &asha, id).await.body["workspace"]["name"], "Private");
    }
}

db_test! {
    async fn an_invitation_brings_in_only_the_account_it_was_sent_to(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;
        let sent = invite(&app, &asha, id, " Ben@WS.in ", "member").await;
        assert_eq!(sent.status, StatusCode::CREATED, "{:?}", sent.body);
        assert_eq!(sent.body["invite"]["email"], "ben@ws.in", "kept the way the account's own address is");
        assert!(sent.body.to_string().contains("\"id\"") && !sent.body.to_string().to_lowercase().contains("token"), "the token is only in the email");
        let token = token_sent_to(&app, "ben@ws.in").await;
        let mail = app.mailer.sent().into_iter().rev().find(|m| m.to == "ben@ws.in").unwrap();
        assert_eq!(mail.subject, "Asha Rao invited you to Maths 9A on Zeno");

        // Somebody else holding the link gets nowhere, and does not use it up.
        let carol = signup_verified(&app, "carol@ws.in").await;
        let wrong = accept(&app, &carol, &token).await;
        assert_eq!(wrong.status, StatusCode::FORBIDDEN, "{:?}", wrong.body);
        assert_eq!(wrong.code(), "wrong_account");
        assert_eq!(show(&app, &carol, id).await.status, StatusCode::NOT_FOUND);
        // Not signed in at all.
        assert_eq!(app.post("/v1/invites/accept").json(json!({"token": token})).send().await.status, StatusCode::UNAUTHORIZED);

        let ben = signup_verified(&app, "ben@ws.in").await;
        let joined = accept(&app, &ben, &token).await;
        assert_eq!(joined.status, StatusCode::OK, "{:?}", joined.body);
        assert_eq!(joined.body["workspace"]["role"], "member");
        assert_eq!(joined.body["workspace"]["member_count"], 2);
        assert_eq!(show(&app, &ben, id).await.status, StatusCode::OK);

        // It works once.
        let again = accept(&app, &ben, &token).await;
        assert_eq!((again.status, again.code()), (StatusCode::BAD_REQUEST, "invalid_token"));
        assert_eq!(app.get(&format!("/v1/workspaces/{id}/invites")).jar(&asha).send().await.body["invites"], json!([]));
    }
}

db_test! {
    async fn a_link_that_is_revoked_expired_forged_or_for_a_gone_workspace_is_one_answer(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let ben = signup_verified(&app, "ben@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;

        // Revoked.
        let sent = invite(&app, &asha, id, "ben@ws.in", "member").await;
        let token = token_sent_to(&app, "ben@ws.in").await;
        let revoke = app.req(Method::DELETE, &format!("/v1/workspaces/{id}/invites/{}", sent.body["invite"]["id"].as_str().unwrap())).jar(&asha).send().await;
        assert_eq!(revoke.status, StatusCode::NO_CONTENT, "{:?}", revoke.body);
        let revoked = accept(&app, &ben, &token).await;
        assert_eq!((revoked.status, revoked.code()), (StatusCode::BAD_REQUEST, "invalid_token"));

        // Out of time: and a new one is then allowed for the address.
        invite(&app, &asha, id, "ben@ws.in", "member").await;
        let late = token_sent_to(&app, "ben@ws.in").await;
        sqlx::query("update workspace_invites set expires_at = now() - interval '1 minute' where revoked_at is null")
            .execute(&pool).await.unwrap();
        let expired = accept(&app, &ben, &late).await;
        assert_eq!((expired.status, expired.code()), (StatusCode::BAD_REQUEST, "invalid_token"));
        let fresh = invite(&app, &asha, id, "ben@ws.in", "admin").await;
        assert_eq!(fresh.status, StatusCode::CREATED, "an invitation nobody answered does not stand in the way: {:?}", fresh.body);

        // Not a token at all, empty, or far too long.
        for token in ["", "nope", &"x".repeat(5000)] {
            let res = accept(&app, &ben, token).await;
            assert_eq!((res.status, res.code()), (StatusCode::BAD_REQUEST, "invalid_token"), "{token:?}");
        }

        // Its workspace ended.
        let fresh_token = token_sent_to(&app, "ben@ws.in").await;
        let gone = app.req(Method::DELETE, &format!("/v1/workspaces/{id}")).jar(&asha).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        let after = accept(&app, &ben, &fresh_token).await;
        assert_eq!((after.status, after.code()), (StatusCode::BAD_REQUEST, "invalid_token"));
        // The wrong answer does not say which of these it was.
        assert_eq!(revoked.body, expired.body);
        assert_eq!(expired.body, after.body);
    }
}

db_test! {
    async fn inviting_has_rules_about_who_may_ask_whom_and_what_is_already_so(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;
        let admin = join(&app, &asha, id, "admin@ws.in", "admin").await;
        let member = join(&app, &asha, id, "member@ws.in", "member").await;

        // Nobody is invited as an owner; an admin invites members only; a member invites nobody.
        assert_eq!(invite(&app, &asha, id, "o@ws.in", "owner").await.status, StatusCode::FORBIDDEN);
        assert_eq!(invite(&app, &admin, id, "a2@ws.in", "admin").await.status, StatusCode::FORBIDDEN);
        assert_eq!(invite(&app, &admin, id, "m2@ws.in", "member").await.status, StatusCode::CREATED);
        assert_eq!(invite(&app, &member, id, "m3@ws.in", "member").await.status, StatusCode::FORBIDDEN);
        let unknown = app.post(&format!("/v1/workspaces/{id}/invites")).jar(&asha).json(json!({"email": "x@ws.in", "role": "king"})).send().await;
        assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
        let bad = invite(&app, &asha, id, "not an email", "member").await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);

        // Already in, and already invited.
        let in_already = invite(&app, &asha, id, "member@ws.in", "member").await;
        assert_eq!((in_already.status, in_already.code()), (StatusCode::CONFLICT, "already_member"));
        let twice = invite(&app, &asha, id, "m2@ws.in", "member").await;
        assert_eq!((twice.status, twice.code()), (StatusCode::CONFLICT, "already_invited"));
        // Whoever invites an address that has no account is not told so (the answer is the same).
        let stranger = invite(&app, &asha, id, "nobody-at-all@ws.in", "member").await;
        assert_eq!(stranger.status, StatusCode::CREATED);

        // Who sees the open invitations: owners and admins, not members.
        assert_eq!(app.get(&format!("/v1/workspaces/{id}/invites")).jar(&member).send().await.status, StatusCode::FORBIDDEN);
        let seen = app.get(&format!("/v1/workspaces/{id}/invites")).jar(&admin).send().await;
        assert_eq!(seen.body["invites"].as_array().unwrap().len(), 2);
        assert_eq!(seen.body["invites"][0]["invited_by"], "Asha Rao");
        assert!(!seen.body.to_string().contains("token"));
    }
}

db_test! {
    async fn a_workspace_holds_fifty_people_counting_open_invitations(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Everyone").await;
        // 47 more members, made directly: signing 47 people up would spend the sign-up limit.
        for n in 0..47 {
            let user = Uuid::new_v4();
            sqlx::query("insert into users (id, name, email, password_hash, role) values ($1, 'Pupil', $2, 'x', 'student')")
                .bind(user).bind(format!("pupil{n}@ws.in")).execute(&pool).await.unwrap();
            sqlx::query("insert into workspace_members (workspace_id, user_id, role) values ($1, $2, 'member')")
                .bind(id).bind(user).execute(&pool).await.unwrap();
        }
        // 48 members; two invitations make fifty.
        assert_eq!(invite(&app, &asha, id, "one@ws.in", "member").await.status, StatusCode::CREATED);
        assert_eq!(invite(&app, &asha, id, "two@ws.in", "member").await.status, StatusCode::CREATED);
        let full = invite(&app, &asha, id, "three@ws.in", "member").await;
        assert_eq!((full.status, full.code()), (StatusCode::CONFLICT, "workspace_full"));

        // When the last place is taken by a member, the one holding an invitation for it is told so, and keeps it.
        app.settle().await;
        let late = signup_verified(&app, "two@ws.in").await;
        let token = token_sent_to(&app, "two@ws.in").await;
        let mut squeezed = Vec::new();
        for n in 0..2 {
            let user = Uuid::new_v4();
            sqlx::query("insert into users (id, name, email, password_hash, role) values ($1, 'Pupil', $2, 'x', 'student')")
                .bind(user).bind(format!("squeezed{n}@ws.in")).execute(&pool).await.unwrap();
            sqlx::query("insert into workspace_members (workspace_id, user_id, role) values ($1, $2, 'member')").bind(id).bind(user).execute(&pool).await.unwrap();
            squeezed.push(user);
        }
        // Now 50 are in, and the one with an invitation for a place that was promised to them is told it is gone.
        let joined = accept(&app, &late, &token).await;
        assert_eq!((joined.status, joined.code()), (StatusCode::CONFLICT, "workspace_full"), "{:?}", joined.body);
        // Refused, so not used up: room made later and it works.
        sqlx::query("delete from workspace_members where user_id = $1").bind(squeezed[0]).execute(&pool).await.unwrap();
        assert_eq!(accept(&app, &late, &token).await.status, StatusCode::OK);
    }
}

db_test! {
    async fn members_see_names_and_owners_and_admins_see_addresses(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;
        let admin = join(&app, &asha, id, "admin@ws.in", "admin").await;
        let member = join(&app, &asha, id, "member@ws.in", "member").await;

        let emails = |res: &Res| -> Vec<Option<String>> {
            res.body["members"].as_array().unwrap().iter().map(|m| m["email"].as_str().map(str::to_owned)).collect()
        };
        let as_owner = emails(&show(&app, &asha, id).await);
        assert!(as_owner.iter().all(Option::is_some), "{as_owner:?}");
        assert!(emails(&show(&app, &admin, id).await).iter().all(Option::is_some));
        let seen_by_member = show(&app, &member, id).await;
        assert_eq!(emails(&seen_by_member).iter().flatten().collect::<Vec<_>>(), ["member@ws.in"], "a member sees their own address and no one else's");
        assert!(seen_by_member.body["members"].as_array().unwrap().iter().all(|m| m["name"].is_string()));
        assert_eq!(seen_by_member.body["workspace"]["role"], "member");
    }
}

db_test! {
    async fn only_an_owner_changes_roles_and_a_workspace_keeps_an_owner(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;
        let admin = join(&app, &asha, id, "admin@ws.in", "admin").await;
        let member = join(&app, &asha, id, "member@ws.in", "member").await;
        let (a, adm, mem) = (user_id(&pool, "asha@ws.in").await, user_id(&pool, "admin@ws.in").await, user_id(&pool, "member@ws.in").await);
        let set = |jar: &Jar, target: Uuid, role: &str| {
            let url = format!("/v1/workspaces/{id}/members/{target}");
            app.req(Method::PATCH, &url).jar(jar).json(json!({"role": role})).send()
        };

        assert_eq!(set(&admin, mem, "admin").await.status, StatusCode::FORBIDDEN, "an admin does not choose admins");
        assert_eq!(set(&member, mem, "owner").await.status, StatusCode::FORBIDDEN, "nobody makes themselves an owner");
        assert_eq!(set(&asha, mem, "admin").await.status, StatusCode::NO_CONTENT);
        assert_eq!(set(&asha, mem, "admin").await.status, StatusCode::NO_CONTENT, "saying it again is not an error");
        assert_eq!(set(&asha, Uuid::new_v4(), "admin").await.status, StatusCode::NOT_FOUND);
        assert_eq!(set(&asha, mem, "king").await.status, StatusCode::UNPROCESSABLE_ENTITY);

        // The only owner cannot step down; with a second one they can.
        let alone = set(&asha, a, "member").await;
        assert_eq!((alone.status, alone.code()), (StatusCode::CONFLICT, "last_owner"));
        assert_eq!(set(&asha, adm, "owner").await.status, StatusCode::NO_CONTENT);
        assert_eq!(owners(&pool, id).await, 2);
        assert_eq!(set(&asha, a, "member").await.status, StatusCode::NO_CONTENT);
        assert_eq!(show(&app, &asha, id).await.body["workspace"]["role"], "member");
        assert_eq!(owners(&pool, id).await, 1);
        // And the one who is left is now the only one.
        let last = set(&admin, adm, "admin").await;
        assert_eq!((last.status, last.code()), (StatusCode::CONFLICT, "last_owner"));
    }
}

db_test! {
    async fn removing_and_leaving_follow_the_roles(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;
        let admin = join(&app, &asha, id, "admin@ws.in", "admin").await;
        let admin2 = join(&app, &asha, id, "admin2@ws.in", "admin").await;
        let member = join(&app, &asha, id, "member@ws.in", "member").await;
        let member2 = join(&app, &asha, id, "member2@ws.in", "member").await;
        let id_of = |email: &'static str| { let pool = pool.clone(); async move { user_id(&pool, email).await } };
        let (a, adm, adm2, mem, mem2) = (id_of("asha@ws.in").await, id_of("admin@ws.in").await, id_of("admin2@ws.in").await, id_of("member@ws.in").await, id_of("member2@ws.in").await);
        let remove = |jar: &Jar, target: Uuid| app.req(Method::DELETE, &format!("/v1/workspaces/{id}/members/{target}")).jar(jar).send();

        // A member removes nobody, and is not told who exists.
        assert_eq!(remove(&member, mem2).await.status, StatusCode::FORBIDDEN);
        assert_eq!(remove(&member, Uuid::new_v4()).await.status, StatusCode::FORBIDDEN);
        // An admin removes members, not admins or owners.
        assert_eq!(remove(&admin, adm2).await.status, StatusCode::FORBIDDEN);
        assert_eq!(remove(&admin, a).await.status, StatusCode::FORBIDDEN);
        assert_eq!(remove(&admin, Uuid::new_v4()).await.status, StatusCode::NOT_FOUND);
        assert_eq!(remove(&admin, mem2).await.status, StatusCode::NO_CONTENT);
        assert_eq!(show(&app, &member2, id).await.status, StatusCode::NOT_FOUND, "gone at once");
        // An owner removes anyone else.
        assert_eq!(remove(&asha, adm2).await.status, StatusCode::NO_CONTENT);
        // Anyone may leave: the person is out, and the workspace is not theirs to see.
        assert_eq!(remove(&member, mem).await.status, StatusCode::NO_CONTENT);
        assert_eq!(show(&app, &member, id).await.status, StatusCode::NOT_FOUND);
        assert_eq!(remove(&member, mem).await.status, StatusCode::NOT_FOUND, "leaving twice");
        // The only owner cannot leave; after handing over, they can.
        let stuck = remove(&asha, a).await;
        assert_eq!((stuck.status, stuck.code()), (StatusCode::CONFLICT, "last_owner"));
        assert_eq!(app.req(Method::PATCH, &format!("/v1/workspaces/{id}/members/{adm}")).jar(&asha).json(json!({"role": "owner"})).send().await.status, StatusCode::NO_CONTENT);
        assert_eq!(remove(&asha, a).await.status, StatusCode::NO_CONTENT);
        assert_eq!(show(&app, &admin, id).await.body["workspace"]["member_count"], 1);
        let _ = (admin2, member2);
    }
}

db_test! {
    async fn two_owners_cannot_both_walk_away_nor_both_step_down(pool: PgPool) {
        // Each looks at the owners, sees another one left, and goes: unless the changes take turns.
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;
        let ben = join(&app, &asha, id, "ben@ws.in", "admin").await;
        let (a, b) = (user_id(&pool, "asha@ws.in").await, user_id(&pool, "ben@ws.in").await);
        let promoted = app.req(Method::PATCH, &format!("/v1/workspaces/{id}/members/{b}")).jar(&asha).json(json!({"role": "owner"})).send().await;
        assert_eq!(promoted.status, StatusCode::NO_CONTENT);

        for round in 0..5 {
            // Both leave at the same moment.
            let (left, right) = tokio::join!(
                app.req(Method::DELETE, &format!("/v1/workspaces/{id}/members/{a}")).jar(&asha).send(),
                app.req(Method::DELETE, &format!("/v1/workspaces/{id}/members/{b}")).jar(&ben).send(),
            );
            let mut statuses = [left.status.as_u16(), right.status.as_u16()];
            statuses.sort_unstable();
            assert_eq!(statuses, [204, 409], "round {round}: one leaves, the other is told they are the last: {:?} {:?}", left.body, right.body);
            assert_eq!(owners(&pool, id).await, 1, "round {round}");
            // Put the one who left back, as an owner, for the next round.
            let gone = if left.status == StatusCode::NO_CONTENT { a } else { b };
            sqlx::query("insert into workspace_members (workspace_id, user_id, role) values ($1, $2, 'owner')").bind(id).bind(gone).execute(&pool).await.unwrap();
        }

        // Both hand the other the title of member at once: the second finds they are no longer an owner.
        let (one, two) = tokio::join!(
            app.req(Method::PATCH, &format!("/v1/workspaces/{id}/members/{b}")).jar(&asha).json(json!({"role": "member"})).send(),
            app.req(Method::PATCH, &format!("/v1/workspaces/{id}/members/{a}")).jar(&ben).json(json!({"role": "member"})).send(),
        );
        let mut statuses = [one.status.as_u16(), two.status.as_u16()];
        statuses.sort_unstable();
        assert_eq!(statuses[0], 204, "{:?} {:?}", one.body, two.body);
        assert_ne!(statuses[1], 204, "{:?} {:?}", one.body, two.body);
        assert_eq!(owners(&pool, id).await, 1, "a workspace always has an owner");
    }
}

db_test! {
    async fn one_link_clicked_twice_at_once_makes_one_member(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;
        invite(&app, &asha, id, "ben@ws.in", "member").await;
        app.settle().await;
        let ben = signup_verified(&app, "ben@ws.in").await;
        let token = token_sent_to(&app, "ben@ws.in").await;
        let answers = join_all((0..4).map(|_| accept(&app, &ben, &token))).await;
        let ok = answers.iter().filter(|r| r.status == StatusCode::OK).count();
        assert_eq!(ok, 1, "{:?}", answers.iter().map(|r| (r.status, r.code().to_owned())).collect::<Vec<_>>());
        assert!(answers.iter().filter(|r| r.status != StatusCode::OK).all(|r| r.code() == "invalid_token"));
        assert_eq!(show(&app, &asha, id).await.body["workspace"]["member_count"], 2);
    }
}

db_test! {
    async fn ending_a_workspace_is_for_owners_and_ends_it_for_everyone(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Maths 9A").await;
        let admin = join(&app, &asha, id, "admin@ws.in", "admin").await;
        let member = join(&app, &asha, id, "member@ws.in", "member").await;
        let rename = |jar: &Jar, name: &str| app.req(Method::PATCH, &format!("/v1/workspaces/{id}")).jar(jar).json(json!({"name": name})).send();

        // Renaming: owners and admins.
        assert_eq!(rename(&member, "Nope").await.status, StatusCode::FORBIDDEN);
        let renamed = rename(&admin, "  Maths 9B ").await;
        assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
        assert_eq!(renamed.body["workspace"]["name"], "Maths 9B");
        assert_eq!(rename(&asha, "x").await.status, StatusCode::UNPROCESSABLE_ENTITY);

        // Ending it: an owner, and nobody else.
        for jar in [&admin, &member] {
            assert_eq!(app.req(Method::DELETE, &format!("/v1/workspaces/{id}")).jar(jar).send().await.status, StatusCode::FORBIDDEN);
        }
        assert_eq!(app.req(Method::DELETE, &format!("/v1/workspaces/{id}")).jar(&asha).send().await.status, StatusCode::NO_CONTENT);
        for jar in [&asha, &admin, &member] {
            assert_eq!(show(&app, jar, id).await.status, StatusCode::NOT_FOUND);
            assert_eq!(app.get("/v1/workspaces").jar(jar).send().await.body["workspaces"], json!([]));
        }
        assert_eq!(app.req(Method::DELETE, &format!("/v1/workspaces/{id}")).jar(&asha).send().await.status, StatusCode::NOT_FOUND, "ending it twice");
        // The owner has room again for another.
        let again = app.post("/v1/workspaces").jar(&asha).json(json!({"name": "Maths 9C"})).send().await;
        assert_eq!(again.status, StatusCode::CREATED);
    }
}

db_test! {
    async fn invitations_are_limited_to_twenty_an_hour_per_person(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        let id = make(&app, &asha, "Big").await;
        let mut limited = None;
        for n in 0..25 {
            let res = invite(&app, &asha, id, &format!("guest{n}@ws.in"), "member").await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some((n, res));
                break;
            }
        }
        let (at, res) = limited.expect("sending runs out");
        assert_eq!(at, 20, "the twenty-first is the first refused");
        assert!(res.headers.get("retry-after").is_some());
    }
}

db_test! {
    async fn the_invitation_text_is_what_a_person_typed_and_nothing_added_to_it(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_verified(&app, "asha@ws.in").await;
        // A name that tries to add a line to the email's subject or body is refused when the workspace is made.
        let bad = app.post("/v1/workspaces").jar(&asha).json(json!({"name": "Team\r\nBcc: someone@evil.example"})).send().await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
        let id = make(&app, &asha, "Maths \"9A\"").await;
        invite(&app, &asha, id, "ben@ws.in", "member").await;
        app.settle().await;
        let mail = app.mailer.sent().into_iter().rev().find(|m| m.to == "ben@ws.in").unwrap();
        assert!(!mail.subject.contains('\n') && !mail.subject.contains('\r'));
        assert!(mail.text.contains("Maths \"9A\""));
        assert!(mail.text.contains(&format!("{ORIGIN}/invite/")), "the link goes to the web app: {}", mail.text);
    }
}
