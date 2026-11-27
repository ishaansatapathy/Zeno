//! The OpenAPI document is a promise to the web app, so it is held to the same standard as the code:
//!
//! * the committed `docs/openapi.json` (what the web app generates its types from) is exactly what
//!   the code produces;
//! * every route is in it, and everything in it is a real route;
//! * every response the API really sends is one the document lists, with the headers it lists and
//!   a body that validates against the schema it lists;
//! * every response the document lists, bar the two that cannot be provoked in a test (our own
//!   bug, and a Redis outage), has been seen.

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use axum::http::StatusCode;
use common::contract::Contract;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn document() -> Value {
    serde_json::to_value(zeno_api::openapi::document()).unwrap()
}

#[test]
fn the_committed_document_is_what_the_code_produces() {
    let path = repo_file("docs/openapi.json");
    let produced = zeno_api::openapi::document_json();
    if std::env::var("UPDATE_OPENAPI").is_ok() {
        std::fs::write(&path, &produced).unwrap();
        return;
    }
    // Git may check the file out with Windows line endings; the content is what matters.
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        committed == produced,
        "docs/openapi.json is out of date. Regenerate it with\n    UPDATE_OPENAPI=1 cargo test -p zeno-api --test openapi\nand then run `npm run api:types` in web/ to update the types."
    );
}

/// Where routers are declared, and where each is mounted.
const ROUTERS: &[(&str, &[(&str, &str)])] = &[
    ("crates/api/src/auth/routes.rs", &[("router", "/v1/auth")]),
    (
        "crates/api/src/recordings/routes.rs",
        &[("router", "/v1/recordings")],
    ),
    (
        "crates/api/src/shares/routes.rs",
        &[
            ("recording_router", "/v1/recordings"),
            ("public_router", "/v1/s"),
        ],
    ),
    (
        "crates/api/src/views/routes.rs",
        &[("recording_router", "/v1/recordings")],
    ),
    (
        "crates/api/src/videos/routes.rs",
        &[("router", "/v1/videos")],
    ),
    (
        "crates/api/src/comments/routes.rs",
        &[
            ("viewer_router", "/v1/s"),
            ("recording_router", "/v1/recordings"),
            ("router", "/v1/comments"),
        ],
    ),
    ("crates/api/src/quota/routes.rs", &[("me_router", "/v1/me")]),
    (
        "crates/api/src/workspaces/routes.rs",
        &[
            ("router", "/v1/workspaces"),
            ("invites_router", "/v1/invites"),
        ],
    ),
];

/// Routes that exist and are not in the document yet. The list is explicit so that a new route
/// cannot be forgotten silently, and it can only shrink: a route here that is documented, or that
/// no longer exists, fails the test below.
const NOT_DESCRIBED_YET: &[&str] = &[
    "put /v1/recordings/{id}/video",
    "get /v1/recordings/{id}/views",
    "post /v1/videos",
    "get /v1/videos/{id}",
    "delete /v1/videos/{id}",
    "post /v1/videos/{id}/abort",
    "post /v1/videos/{id}/play",
];

/// The routes the routers declare, read from their source: `.route("/:id/links", post(create))`
/// becomes `post /v1/recordings/{id}/links`.
fn declared_routes() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (file, mounts) in ROUTERS {
        let source = std::fs::read_to_string(repo_file(file)).unwrap();
        let mut prefix: Option<&str> = None;
        for line in source.lines() {
            let line = line.trim_start();
            if let Some(rest) = line.strip_prefix("pub fn ") {
                let name = rest.split('(').next().unwrap();
                prefix = mounts.iter().find(|(n, _)| *n == name).map(|(_, p)| *p);
            }
            let (Some(prefix), Some((_, rest))) = (prefix, line.split_once(".route(\"")) else {
                continue;
            };
            let path = rest.split('"').next().unwrap();
            let path = if path == "/" {
                String::new()
            } else {
                path.to_owned()
            };
            let template: Vec<String> = path
                .split('/')
                .map(|segment| match segment.strip_prefix(':') {
                    Some(name) => format!("{{{name}}}"),
                    None => segment.to_owned(),
                })
                .collect();
            for method in ["get", "post", "put", "patch", "delete"] {
                if rest.contains(&format!("{method}(")) {
                    found.insert(format!("{method} {prefix}{}", template.join("/")));
                }
            }
        }
    }
    found
}

#[test]
fn every_route_is_documented_and_every_documented_route_exists() {
    let doc = document();
    let mut documented = BTreeSet::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for method in ["get", "post", "put", "patch", "delete"] {
            if item.get(method).is_some() {
                documented.insert(format!("{method} {path}"));
            }
        }
    }
    let declared = declared_routes();
    assert!(declared.len() >= 20, "the source scan found {declared:?}");

    let waiting: BTreeSet<String> = NOT_DESCRIBED_YET.iter().map(|r| (*r).to_owned()).collect();
    for route in &waiting {
        assert!(declared.contains(route), "{route} is listed as not described yet, but no such route exists: remove it from the list");
        assert!(
            !documented.contains(route),
            "{route} is documented now: remove it from the list of routes not described yet"
        );
    }
    let expected: BTreeSet<String> = declared.difference(&waiting).cloned().collect();
    assert_eq!(
        expected, documented,
        "routes and the OpenAPI document disagree (left: declared and not listed as waiting, right: documented). Describe the new route, or add it to NOT_DESCRIBED_YET."
    );
}

#[test]
fn the_schema_limits_are_the_ones_the_validation_enforces() {
    use zeno_api::auth::validation::{PASSWORD_MAX_CHARS, PASSWORD_MIN_CHARS};
    let doc = document();
    let schemas = &doc["components"]["schemas"];
    for (schema, field) in [
        ("RegisterRequest", "password"),
        ("ResetPasswordRequest", "new_password"),
    ] {
        let property = &schemas[schema]["properties"][field];
        assert_eq!(
            property["minLength"], PASSWORD_MIN_CHARS,
            "{schema}.{field}"
        );
        assert_eq!(
            property["maxLength"], PASSWORD_MAX_CHARS,
            "{schema}.{field}"
        );
    }
    let name = &schemas["RegisterRequest"]["properties"]["name"];
    assert_eq!(
        (name["minLength"].as_u64(), name["maxLength"].as_u64()),
        (Some(2), Some(80))
    );
}

#[test]
fn secrets_are_marked_as_passwords_and_the_role_is_an_enum() {
    let doc = document();
    let schemas = &doc["components"]["schemas"];
    assert_eq!(
        schemas["LoginRequest"]["properties"]["password"]["format"],
        "password"
    );
    assert_eq!(
        schemas["UserResponse"]["properties"]["role"]["$ref"],
        "#/components/schemas/Role"
    );
    assert_eq!(
        schemas["Role"]["enum"],
        json!(["student", "creator", "admin"])
    );
    assert_eq!(
        doc["components"]["securitySchemes"]["session"]["in"],
        "cookie"
    );
}

db_test! {
    async fn every_real_response_is_one_the_document_promises(pool: PgPool) {
        let app = app(pool);
        let contract = Contract::new();
        let email = "asha@example.com";

        // Registering: everything that can be wrong with the request.
        let bad = app
            .post("/v1/auth/register")
            .json(json!({"name": "x", "email": "nope", "password": "short"}))
            .send()
            .await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(bad.body["error"]["fields"].as_array().unwrap().len(), 3);
        contract.check("post", "/v1/auth/register", &bad);

        let malformed = app.post("/v1/auth/register").raw_body(b"{".to_vec(), Some("application/json")).send().await;
        assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
        contract.check("post", "/v1/auth/register", &malformed);
        let wrong_type = app.post("/v1/auth/register").raw_body(b"x".to_vec(), Some("text/plain")).send().await;
        assert_eq!(wrong_type.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        contract.check("post", "/v1/auth/register", &wrong_type);
        let huge = app.post("/v1/auth/register").raw_body(vec![b' '; 3_000_000], Some("application/json")).send().await;
        assert_eq!(huge.status, StatusCode::PAYLOAD_TOO_LARGE);
        contract.check("post", "/v1/auth/register", &huge);
        let foreign = app.post("/v1/auth/register").origin("https://evil.example").json(register_body(email)).send().await;
        assert_eq!(foreign.status, StatusCode::FORBIDDEN);
        assert_eq!(foreign.code(), "forbidden_origin");
        contract.check("post", "/v1/auth/register", &foreign);

        let before = app.mailer.sent().len();
        let made = app.post("/v1/auth/register").json(register_body(email)).send().await;
        assert_eq!(made.status, StatusCode::ACCEPTED);
        contract.check("post", "/v1/auth/register", &made);
        let token = token_from(&app.mailer.wait_for(before + 1).await[before]);

        // Signing in before verifying, then verifying.
        let unverified = login(&app, email, PASSWORD).await;
        assert_eq!(unverified.code(), "email_not_verified");
        contract.check("post", "/v1/auth/login", &unverified);
        let wrong_token = verify(&app, "not-a-real-token").await;
        assert_eq!(wrong_token.code(), "invalid_token");
        contract.check("post", "/v1/auth/verify-email", &wrong_token);
        let malformed = app.post("/v1/auth/verify-email").raw_body(b"[]".to_vec(), Some("application/json")).send().await;
        contract.check("post", "/v1/auth/verify-email", &malformed);
        let verified = verify(&app, &token).await;
        assert_eq!(verified.status, StatusCode::OK);
        contract.check("post", "/v1/auth/verify-email", &verified);
        let mut jar = Jar::of(&verified);

        // Who am I, and what has happened to my account.
        let me_uri = "/v1/auth/me";
        let me = app.get(me_uri).jar(&jar).send().await;
        assert_eq!(me.body["user"]["email_verified"], true);
        contract.check("get", me_uri, &me);
        contract.check("get", me_uri, &app.get(me_uri).send().await);
        let activity = app.get("/v1/auth/activity").jar(&jar).send().await;
        contract.check("get", "/v1/auth/activity", &activity);
        contract.check("get", "/v1/auth/activity", &app.get("/v1/auth/activity").send().await);

        // Signing in, again.
        let wrong = login(&app, email, "not the password at all").await;
        assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
        contract.check("post", "/v1/auth/login", &wrong);
        let ok = login(&app, email, PASSWORD).await;
        assert_eq!(ok.status, StatusCode::OK);
        contract.check("post", "/v1/auth/login", &ok);
        jar = Jar::of(&ok);

        // Rotating the session and leaving it.
        let refreshed = app.post("/v1/auth/refresh").jar(&jar).send().await;
        assert_eq!(refreshed.status, StatusCode::NO_CONTENT);
        contract.check("post", "/v1/auth/refresh", &refreshed);
        contract.check("post", "/v1/auth/refresh", &app.post("/v1/auth/refresh").send().await);
        jar.apply(&refreshed);
        let out = app.post("/v1/auth/logout").jar(&jar).send().await;
        assert_eq!(out.status, StatusCode::NO_CONTENT);
        contract.check("post", "/v1/auth/logout", &out);

        // The mail routes answer the same whoever asks.
        let mails_before = app.mailer.sent().len();
        for (path, body) in [
            ("/v1/auth/resend-verification", json!({"email": email})),
            ("/v1/auth/forgot-password", json!({"email": email})),
        ] {
            let accepted = app.post(path).json(body).send().await;
            assert_eq!(accepted.status, StatusCode::ACCEPTED);
            contract.check("post", path, &accepted);
            let invalid = app.post(path).json(json!({"email": "nope"})).send().await;
            assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
            contract.check("post", path, &invalid);
        }

        // Resetting the password.
        let mails = app.mailer.wait_for(mails_before + 1).await;
        let reset_mail = mails.iter().rev().find(|m| m.subject.contains("Reset")).expect("a reset email");
        let reset_token = token_from(reset_mail);
        let weak = app.post("/v1/auth/reset-password").json(json!({"token": reset_token, "new_password": "short"})).send().await;
        assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("post", "/v1/auth/reset-password", &weak);
        let bad_token = app.post("/v1/auth/reset-password").json(json!({"token": "nope", "new_password": "a brand new passphrase"})).send().await;
        assert_eq!(bad_token.code(), "invalid_token");
        contract.check("post", "/v1/auth/reset-password", &bad_token);
        let reset = app.post("/v1/auth/reset-password").json(json!({"token": reset_token, "new_password": "a brand new passphrase"})).send().await;
        assert_eq!(reset.status, StatusCode::OK);
        contract.check("post", "/v1/auth/reset-password", &reset);

        // Running out of attempts: the document says what that looks like, headers and all.
        let mut limited = None;
        for n in 0..25 {
            let res = app.post("/v1/auth/register").json(register_body(&format!("flood{n}@example.com"))).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        let limited = limited.expect("the register limit is reached");
        assert!(limited.headers.contains_key("retry-after"));
        contract.check("post", "/v1/auth/register", &limited);
        let mut locked = None;
        for _ in 0..30 {
            let res = login(&app, "target@example.com", "guess after guess after guess").await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                locked = Some(res);
                break;
            }
        }
        let locked = locked.expect("repeated failures lock the sign-in");
        contract.check("post", "/v1/auth/login", &locked);
        for path in ["/v1/auth/resend-verification", "/v1/auth/forgot-password"] {
            let mut hit = None;
            for _ in 0..15 {
                let res = app.post(path).json(json!({"email": email})).send().await;
                if res.status == StatusCode::TOO_MANY_REQUESTS {
                    hit = Some(res);
                    break;
                }
            }
            if let Some(res) = hit {
                contract.check("post", path, &res);
            }
        }

        // Changing the password and the name: signed in, so the common errors are provoked with a session.
        // The password was reset above.
        let now = "a brand new passphrase";
        let mut jar = Jar::of(&login(&app, email, now).await);
        let change = "/v1/auth/change-password";
        let name = |name: &str| json!({"name": name});
        let renamed = app.req(axum::http::Method::PATCH, me_uri).jar(&jar).json(name("Asha R")).send().await;
        assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
        contract.check("patch", me_uri, &renamed);
        let blank = app.req(axum::http::Method::PATCH, me_uri).jar(&jar).json(name("   ")).send().await;
        assert_eq!(blank.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("patch", me_uri, &blank);
        contract.check("patch", me_uri, &app.req(axum::http::Method::PATCH, me_uri).json(name("x")).send().await);
        let mut limited = None;
        for _ in 0..25 {
            let res = app.req(axum::http::Method::PATCH, me_uri).jar(&jar).json(name("Asha R")).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("patch", me_uri, &limited.expect("renaming runs out"));
        let attempt = |current: &str, new: &str, jar: &Jar| {
            app.post(change).jar(jar).json(json!({"current_password": current, "new_password": new})).send()
        };
        let wrong = attempt("not the password at all", "a-completely-new-passphrase", &jar).await;
        assert_eq!(wrong.code(), "wrong_password");
        contract.check("post", change, &wrong);
        let same = attempt(now, now, &jar).await;
        assert_eq!(same.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", same.body);
        contract.check("post", change, &same);
        contract.check("post", change, &app.post(change).json(json!({"current_password": now, "new_password": "a-completely-new-passphrase"})).send().await);
        // What can go wrong on any route that needs a session.
        contract.provoke_common_errors(&app, me_uri, Some(&jar), &[]).await;
        contract.provoke_common_errors(&app, "/v1/auth/activity", Some(&jar), &[]).await;
        contract.provoke_common_errors(&app, change, Some(&jar), &[]).await;
        let changed = attempt(now, "a-completely-new-passphrase", &jar).await;
        assert_eq!(changed.status, StatusCode::OK, "{:?}", changed.body);
        contract.check("post", change, &changed);
        // Five tries an hour, right or wrong.
        jar = Jar::of(&login(&app, email, "a-completely-new-passphrase").await);
        let mut limited = None;
        for _ in 0..8 {
            let res = attempt("still not the password", "another-new-passphrase!", &jar).await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("post", change, &limited.expect("changing runs out"));

        // What can go wrong on any route that needs no session.
        for path in [
            "/v1/auth/register",
            "/v1/auth/verify-email",
            "/v1/auth/resend-verification",
            "/v1/auth/login",
            "/v1/auth/refresh",
            "/v1/auth/logout",
            "/v1/auth/forgot-password",
            "/v1/auth/reset-password",
        ] {
            contract.provoke_common_errors(&app, path, None, &[]).await;
        }

        let unseen = contract.unseen("/v1/auth/");
        assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
    }
}

#[test]
fn the_words_the_api_chooses_from_are_listed_in_the_document() {
    use zeno_api::recordings::dto::{SOURCES, STATUSES, VISIBILITIES};
    use zeno_api::shares::dto::LINK_STATES;
    let doc = document();
    let schemas = &doc["components"]["schemas"];
    let listed = |schema: &str, field: &str| -> Vec<String> {
        schemas[schema]["properties"][field]["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("{schema}.{field} lists no values"))
            .iter()
            .filter_map(|word| word.as_str().map(str::to_owned))
            .collect()
    };
    let own = |words: &[&str]| -> Vec<String> { words.iter().map(|w| (*w).to_owned()).collect() };
    assert_eq!(listed("RecordingResponse", "source"), own(SOURCES));
    assert_eq!(listed("CreateRecordingRequest", "source"), own(SOURCES));
    assert_eq!(listed("RecordingResponse", "visibility"), own(VISIBILITIES));
    assert_eq!(
        listed("UpdateRecordingRequest", "visibility"),
        own(VISIBILITIES)
    );
    assert_eq!(listed("RecordingResponse", "status"), own(STATUSES));
    assert_eq!(listed("LinkResponse", "state"), own(LINK_STATES));
    {
        use zeno_api::comments::dto::{EMOJIS, KINDS};
        assert_eq!(listed("CreateCommentRequest", "kind"), own(KINDS));
        assert_eq!(listed("CommentResponse", "kind"), own(KINDS));
        assert_eq!(listed("CreateCommentRequest", "emoji"), own(EMOJIS));
        assert_eq!(listed("CommentResponse", "emoji"), own(EMOJIS));
    }
    // A field that may be absent may be null, and the list must say so.
    let optional = &schemas["UpdateRecordingRequest"]["properties"]["visibility"]["enum"];
    assert!(optional.as_array().unwrap().contains(&Value::Null));
}

db_test! {
    async fn the_library_api_keeps_its_promises(pool: PgPool) {
        let app = app(pool);
        let contract = Contract::new();
        let owner = signup_creator(&app, "teacher@example.com").await;
        let student = signup_verified_as(&app, "pupil@example.com", "student").await;
        let nobody = uuid::Uuid::new_v4();
        let nobody_s = nobody.to_string();
        let patch = axum::http::Method::PATCH;
        let delete = axum::http::Method::DELETE;

        // What each route accepts, for asking it things without knowing anything about it.
        let sample = |method: &str, path: &str| -> Option<Value> {
            match (method, path) {
                ("post", "/v1/recordings") => Some(json!({"source": "upload"})),
                ("patch", "/v1/recordings/{id}") => Some(json!({"version": 1, "title": "x"})),
                ("post", "/v1/recordings/{id}/links") => Some(json!({})),
                _ => None,
            }
        };

        // Signed out, and signed in without a library: every route says so.
        for (method, path, _) in contract.operations("/v1/recordings") {
            let uri = path.replace("{id}", &nobody_s).replace("{link}", &nobody_s);
            for (jar, status, code) in [(None, 401, "unauthenticated"), (Some(&student), 403, "forbidden")] {
                let mut req = app.req(method.to_uppercase().parse().unwrap(), &uri);
                if let Some(jar) = jar {
                    req = req.jar(jar);
                }
                if let Some(body) = sample(&method, &path) {
                    req = req.json(body);
                }
                let res = req.send().await;
                assert_eq!(res.status.as_u16(), status, "{method} {path}: {:?}", res.body);
                assert_eq!(res.code(), code, "{method} {path}");
                contract.check(&method, &path, &res);
            }
        }

        // Making recordings, and every way that can go wrong.
        let id = uuid::Uuid::new_v4();
        let made = app.post("/v1/recordings").jar(&owner).json(json!({"id": id, "source": "extension", "title": "Week 3"})).send().await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
        contract.check("post", "/v1/recordings", &made);
        let again = app.post("/v1/recordings").jar(&owner).json(json!({"id": id, "source": "extension"})).send().await;
        assert_eq!(again.status, StatusCode::OK);
        contract.check("post", "/v1/recordings", &again);
        let clash = app.post("/v1/recordings").jar(&owner).json(json!({"id": id, "source": "studio"})).send().await;
        assert_eq!(clash.code(), "id_conflict");
        contract.check("post", "/v1/recordings", &clash);
        let ghost = app.post("/v1/recordings").jar(&owner).json(json!({"source": "upload", "video_id": nobody})).send().await;
        assert_eq!(ghost.code(), "video_not_found");
        contract.check("post", "/v1/recordings", &ghost);
        let invalid = app.post("/v1/recordings").jar(&owner).json(json!({"source": "telepathy", "title": ""})).send().await;
        assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("post", "/v1/recordings", &invalid);
        let second = app.post("/v1/recordings").jar(&owner).json(json!({"source": "upload", "title": "Second"})).send().await;
        assert_eq!(second.status, StatusCode::CREATED);
        let second_id = second.body["recording"]["id"].as_str().unwrap().to_owned();

        // Reading the library, a page at a time.
        let list = "/v1/recordings";
        let page = app.get("/v1/recordings?limit=1").jar(&owner).send().await;
        assert_eq!(page.status, StatusCode::OK);
        let cursor = page.body["next_cursor"].as_str().expect("a second page").to_owned();
        contract.check("get", list, &page);
        let next = app.get(&format!("/v1/recordings?limit=1&cursor={cursor}")).jar(&owner).send().await;
        assert_eq!(next.status, StatusCode::OK);
        contract.check("get", list, &next);
        contract.check("get", list, &app.get("/v1/recordings?source=upload").jar(&owner).send().await);
        let bad_cursor = app.get("/v1/recordings?cursor=garbage").jar(&owner).send().await;
        assert_eq!(bad_cursor.code(), "invalid_cursor");
        contract.check("get", list, &bad_cursor);
        let bad_source = app.get("/v1/recordings?source=telepathy").jar(&owner).send().await;
        assert_eq!(bad_source.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("get", list, &bad_source);

        // One recording.
        let one = "/v1/recordings/{id}";
        let got = app.get(&format!("/v1/recordings/{id}")).jar(&owner).send().await;
        assert_eq!(got.body["recording"]["status"], "capturing");
        assert_eq!(got.body["recording"]["view_count"], 0);
        contract.check("get", one, &got);
        contract.check("get", one, &app.get(&format!("/v1/recordings/{nobody}")).jar(&owner).send().await);
        contract.check("get", one, &app.get("/v1/recordings/nope").jar(&owner).send().await);

        // Renaming: the version says which copy was read.
        let uri = format!("/v1/recordings/{id}");
        let renamed = app.req(patch.clone(), &uri).jar(&owner).json(json!({"version": 1, "title": "Renamed"})).send().await;
        assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
        assert_eq!(renamed.body["recording"]["version"], 2);
        contract.check("patch", one, &renamed);
        let by_header = app.req(patch.clone(), &uri).jar(&owner).header("if-match", "\"2\"").json(json!({"title": "Renamed again"})).send().await;
        assert_eq!(by_header.status, StatusCode::OK, "{:?}", by_header.body);
        contract.check("patch", one, &by_header);
        let stale = app.req(patch.clone(), &uri).jar(&owner).json(json!({"version": 1, "title": "Too late"})).send().await;
        assert_eq!(stale.code(), "version_conflict");
        contract.check("patch", one, &stale);
        let unversioned = app.req(patch.clone(), &uri).jar(&owner).json(json!({"title": "No version"})).send().await;
        assert_eq!(unversioned.status, StatusCode::PRECONDITION_REQUIRED);
        contract.check("patch", one, &unversioned);
        let junk = app.req(patch.clone(), &uri).jar(&owner).header("if-match", "abc").json(json!({"title": "x"})).send().await;
        assert_eq!(junk.status, StatusCode::BAD_REQUEST);
        contract.check("patch", one, &junk);
        let nothing = app.req(patch.clone(), &uri).jar(&owner).json(json!({"version": 3})).send().await;
        assert_eq!(nothing.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("patch", one, &nothing);
        let missing = app.req(patch, &format!("/v1/recordings/{nobody}")).jar(&owner).json(json!({"version": 1, "title": "x"})).send().await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        contract.check("patch", one, &missing);

        // Share links.
        let links = "/v1/recordings/{id}/links";
        let link = "/v1/recordings/{id}/links/{link}";
        let links_uri = format!("/v1/recordings/{id}/links");
        let weak = app.post(&links_uri).jar(&owner).json(json!({"password": "abc"})).send().await;
        assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("post", links, &weak);
        let first = app.post(&links_uri).jar(&owner).json(json!({})).send().await;
        assert_eq!(first.status, StatusCode::CREATED, "{:?}", first.body);
        assert_eq!(first.body["link"]["state"], "active");
        contract.check("post", links, &first);
        let first_id = first.body["link"]["id"].as_str().unwrap().to_owned();
        let listed = app.get(&links_uri).jar(&owner).send().await;
        assert_eq!(listed.body["links"].as_array().unwrap().len(), 1);
        contract.check("get", links, &listed);
        contract.check("get", links, &app.get(&format!("/v1/recordings/{nobody}/links")).jar(&owner).send().await);
        contract.check("post", links, &app.post(&format!("/v1/recordings/{nobody}/links")).jar(&owner).json(json!({})).send().await);
        let revoke = |recording: String, link_id: String| {
            app.req(delete.clone(), &format!("/v1/recordings/{recording}/links/{link_id}")).jar(&owner).send()
        };
        let revoked = revoke(id.to_string(), first_id.clone()).await;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);
        contract.check("delete", link, &revoked);
        contract.check("delete", link, &revoke(id.to_string(), first_id.clone()).await);
        contract.check("delete", link, &revoke(id.to_string(), nobody_s.clone()).await);
        let relisted = app.get(&links_uri).jar(&owner).send().await;
        assert_eq!(relisted.body["links"][0]["state"], "revoked");
        contract.check("get", links, &relisted);

        // Ten live links is the most a recording may have.
        let mut refused = None;
        for _ in 0..11 {
            let res = app.post(&links_uri).jar(&owner).json(json!({})).send().await;
            if res.status == StatusCode::CONFLICT {
                refused = Some(res);
                break;
            }
            assert_eq!(res.status, StatusCode::CREATED);
        }
        let refused = refused.expect("the eleventh live link is refused");
        assert_eq!(refused.code(), "too_many_links");
        contract.check("post", links, &refused);

        // Deleting.
        let gone = app.req(delete.clone(), &format!("/v1/recordings/{second_id}")).jar(&owner).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        contract.check("delete", one, &gone);
        contract.check("delete", one, &app.req(delete.clone(), &format!("/v1/recordings/{second_id}")).jar(&owner).send().await);

        // Running out of budget: for links (and then revoking them), and for recordings.
        let mut limited = None;
        for _ in 0..40 {
            let res = app.post(&format!("/v1/recordings/{nobody}/links")).jar(&owner).json(json!({})).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("post", links, &limited.expect("the link budget runs out"));
        let blocked = revoke(id.to_string(), nobody_s.clone()).await;
        assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
        contract.check("delete", link, &blocked);
        let mut limited = None;
        for _ in 0..70 {
            let res = app.post("/v1/recordings").jar(&owner).json(json!({"source": "upload"})).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("post", "/v1/recordings", &limited.expect("the recording budget runs out"));

        // What can go wrong on any route.
        contract
            .provoke_common_errors(&app, "/v1/recordings", Some(&owner), &[("id", &id.to_string()), ("link", &nobody_s)])
            .await;

        let unseen: Vec<String> = contract.unseen("/v1/recordings").into_iter().filter(|route| !route.contains("/comments")).collect();
        assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
    }
}

db_test! {
    async fn the_visitors_api_keeps_its_promises(pool: PgPool) {
        use common::media::ready_video;

        let app = app(pool.clone());
        let contract = Contract::new();
        let owner = signup_creator(&app, "teacher@example.com").await;
        let nobody = uuid::Uuid::new_v4();

        // A recording whose video is ready, with an open link, a password link and a closed one.
        let video = ready_video(&app, &owner, &pool).await;
        let made = app.post("/v1/recordings").jar(&owner).json(json!({"source": "extension", "title": "Week 3", "video_id": video})).send().await;
        let ready = made.body["recording"]["id"].as_str().unwrap().to_owned();
        let link = |recording: String, body: Value| {
            let app = &app;
            let owner = &owner;
            async move {
                let res = app.post(&format!("/v1/recordings/{recording}/links")).jar(owner).json(body).send().await;
                assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
                (res.body["link"]["code"].as_str().unwrap().to_owned(), res.body["link"]["id"].as_str().unwrap().to_owned())
            }
        };
        let (open, _) = link(ready.clone(), json!({})).await;
        let (locked, _) = link(ready.clone(), json!({"password": "open sesame"})).await;
        let (closed, closed_id) = link(ready.clone(), json!({})).await;
        let revoked = app.req(axum::http::Method::DELETE, &format!("/v1/recordings/{ready}/links/{closed_id}")).jar(&owner).send().await;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);

        // A recording whose video is still being prepared.
        let second = ready_video(&app, &owner, &pool).await;
        sqlx::query("update videos set status = 'processing' where id = $1").bind(second).execute(&pool).await.unwrap();
        let made = app.post("/v1/recordings").jar(&owner).json(json!({"source": "upload", "title": "Week 4", "video_id": second})).send().await;
        let (preparing, _) = link(made.body["recording"]["id"].as_str().unwrap().to_owned(), json!({})).await;

        let page = "/v1/s/{code}";
        let play = "/v1/s/{code}/play";
        let get = |code: String| app.get(&format!("/v1/s/{code}")).send();
        let press = |code: String, body: Value| app.post(&format!("/v1/s/{code}/play")).json(body).send();

        // The page: what it says for each kind of link, and that it can be reused.
        let shown = get(open.clone()).await;
        assert_eq!(shown.status, StatusCode::OK, "{:?}", shown.body);
        assert_eq!(shown.body["share"]["status"], "ready");
        contract.check("get", page, &shown);
        let etag = shown.headers["etag"].to_str().unwrap().to_owned();
        let again = app.get(&format!("/v1/s/{open}")).header("if-none-match", &etag).send().await;
        assert_eq!(again.status, StatusCode::NOT_MODIFIED);
        contract.check("get", page, &again);
        let protected = get(locked.clone()).await;
        assert_eq!(protected.body["share"]["requires_password"], true);
        assert!(protected.body["share"].get("title").is_none(), "a protected link shows no title");
        contract.check("get", page, &protected);
        let waiting = get(preparing.clone()).await;
        assert_eq!(waiting.body["share"]["status"], "processing");
        contract.check("get", page, &waiting);
        contract.check("get", page, &get("A".repeat(22)).await);
        contract.check("get", page, &get("short".into()).await);
        let gone = get(closed.clone()).await;
        assert_eq!(gone.status, StatusCode::GONE);
        contract.check("get", page, &gone);

        // Pressing play.
        let started = press(open.clone(), json!({})).await;
        assert_eq!(started.status, StatusCode::CREATED, "{:?}", started.body);
        assert!(started.body["playback"]["playlist_url"].as_str().unwrap().starts_with("/v1/play/"));
        contract.check("post", play, &started);
        let missing_password = press(locked.clone(), json!({})).await;
        assert_eq!(missing_password.code(), "password_required");
        contract.check("post", play, &missing_password);
        let wrong_password = press(locked.clone(), json!({"password": "open sesamE"})).await;
        assert_eq!(wrong_password.code(), "wrong_password");
        contract.check("post", play, &wrong_password);
        let unlocked = press(locked.clone(), json!({"password": "open sesame"})).await;
        assert_eq!(unlocked.status, StatusCode::CREATED);
        contract.check("post", play, &unlocked);
        let not_ready = press(preparing.clone(), json!({})).await;
        assert_eq!(not_ready.code(), "not_ready");
        contract.check("post", play, &not_ready);
        contract.check("post", play, &press("A".repeat(22), json!({})).await);
        contract.check("post", play, &press(closed.clone(), json!({})).await);

        // What can go wrong on any route.
        contract.provoke_common_errors(&app, "/v1/s", Some(&owner), &[("code", &open)]).await;

        // Running out of budget: password guesses on one link, starting viewings, and opening pages.
        let mut limited = None;
        for n in 0..12 {
            let res = press(locked.clone(), json!({"password": format!("guess number {n}")})).await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("post", play, &limited.expect("password guesses run out"));
        let mut limited = None;
        for _ in 0..40 {
            let res = press(open.clone(), json!({})).await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("post", play, &limited.expect("starting viewings runs out"));
        let mut limited = None;
        for _ in 0..320 {
            let res = get(open.clone()).await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("get", page, &limited.expect("opening pages runs out"));
        let _ = nobody;

        let unseen: Vec<String> = contract.unseen("/v1/s").into_iter().filter(|route| !route.contains("/comments")).collect();
        assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
    }
}

db_test! {
    async fn the_upload_api_keeps_its_promises(pool: PgPool) {
        const MIB: usize = 1024 * 1024;
        let app = app(pool);
        let contract = Contract::new();
        let owner = signup_creator(&app, "maker@example.com").await;
        let student = signup_verified_as(&app, "pupil@example.com", "student").await;
        let nobody = uuid::Uuid::new_v4();
        let start = "/v1/videos/streaming";
        let parts = "/v1/videos/{id}/parts";
        let complete = "/v1/videos/{id}/complete";
        let resume = "/v1/videos/{id}/upload";
        let started = json!({"filename": "Zeno.webm", "content_type": "video/webm", "part_size_bytes": 5 * MIB});

        // Signed out, and signed in without the right to upload: every route says so.
        for (method, path, _) in contract.operations("/v1/videos") {
            let uri = path.replace("{id}", &nobody.to_string());
            for (jar, status, code) in [(None, 401, "unauthenticated"), (Some(&student), 403, "forbidden")] {
                let mut req = app.req(method.to_uppercase().parse().unwrap(), &uri);
                if let Some(jar) = jar {
                    req = req.jar(jar);
                }
                let body = match path.as_str() {
                    "/v1/videos/streaming" => Some(started.clone()),
                    "/v1/videos/{id}/parts" => Some(json!({"number": 1, "size": 5 * MIB})),
                    _ => None,
                };
                let res = match body {
                    Some(body) => req.json(body).send().await,
                    None => req.send().await,
                };
                assert_eq!(res.status.as_u16(), status, "{method} {path}: {:?}", res.body);
                assert_eq!(res.code(), code, "{method} {path}");
                contract.check(&method, &path, &res);
            }
        }

        // Starting, and every way that can go wrong.
        let made = app.post(start).jar(&owner).json(started.clone()).send().await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
        contract.check("post", start, &made);
        let id = made.body["video"]["id"].as_str().unwrap().to_owned();
        let stable = uuid::Uuid::new_v4();
        let seeded = app.post(start).jar(&owner).json(json!({"id": stable, "filename": "stable.webm", "content_type": "video/webm", "part_size_bytes": 5 * MIB})).send().await;
        assert_eq!(seeded.status, StatusCode::CREATED);
        let conflict = app.post(start).jar(&owner).json(json!({"id": stable, "filename": "changed.webm", "content_type": "video/webm", "part_size_bytes": 5 * MIB})).send().await;
        assert_eq!(conflict.code(), "streaming_id_conflict");
        contract.check("post", start, &conflict);
        let tiny = app.post(start).jar(&owner).json(json!({"filename": "a.webm", "content_type": "video/webm", "part_size_bytes": 10})).send().await;
        assert_eq!(tiny.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("post", start, &tiny);

        // Asking for parts.
        let uri = format!("/v1/videos/{id}/parts");
        let first = app.post(&uri).jar(&owner).json(json!({"number": 1, "size": 5 * MIB})).send().await;
        assert_eq!(first.status, StatusCode::OK, "{:?}", first.body);
        contract.check("post", parts, &first);
        let last = app.post(&uri).jar(&owner).json(json!({"number": 2, "size": MIB, "last": true})).send().await;
        assert_eq!(last.status, StatusCode::OK);
        contract.check("post", parts, &last);
        let wrong = app.post(&uri).jar(&owner).json(json!({"number": 1, "size": 7})).send().await;
        assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("post", parts, &wrong);
        let ghost = app.post(&format!("/v1/videos/{nobody}/parts")).jar(&owner).json(json!({"number": 1, "size": 5 * MIB})).send().await;
        assert_eq!(ghost.status, StatusCode::NOT_FOUND);
        contract.check("post", parts, &ghost);
        let classic = app.post("/v1/videos").jar(&owner).json(json!({"filename": "a.mp4", "size_bytes": MIB, "content_type": "video/mp4"})).send().await;
        let classic_id = classic.body["video"]["id"].as_str().unwrap().to_owned();
        let refused = app.post(&format!("/v1/videos/{classic_id}/parts")).jar(&owner).json(json!({"number": 1, "size": MIB, "last": true})).send().await;
        assert_eq!(refused.code(), "not_streaming");
        contract.check("post", parts, &refused);

        // Finishing.
        let finish = format!("/v1/videos/{id}/complete");
        let early = app.post(&finish).jar(&owner).json(json!({"size_bytes": 6 * MIB})).send().await;
        assert_eq!(early.code(), "upload_incomplete");
        contract.check("post", complete, &early);
        let unsized_ = app.post(&finish).jar(&owner).send().await;
        assert_eq!(unsized_.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("post", complete, &unsized_);
        contract.check("post", complete, &app.post(&format!("/v1/videos/{nobody}/complete")).jar(&owner).json(json!({"size_bytes": 1})).send().await);
        // Resuming: nothing yet, then only the parts that are whole (the last, shorter one is not listed).
        let progress = format!("/v1/videos/{id}/upload");
        let nothing_yet = app.get(&progress).jar(&owner).send().await;
        assert_eq!(nothing_yet.status, StatusCode::OK, "{:?}", nothing_yet.body);
        assert_eq!(nothing_yet.body["uploaded"], json!([]));
        contract.check("get", resume, &nothing_yet);
        for (number, size, last) in [(1u16, 5 * MIB, false), (2, MIB, true)] {
            let res = app.post(&uri).jar(&owner).json(json!({"number": number, "size": size, "last": last})).send().await;
            let url = res.body["part"]["url"].as_str().unwrap().to_owned();
            assert!(browser_put(&url, vec![7; size]).await.is_success());
        }
        let held = app.get(&progress).jar(&owner).send().await;
        assert_eq!(held.body["uploaded"], json!([1]), "{:?}", held.body);
        assert_eq!(held.body["parts"], json!([]));
        contract.check("get", resume, &held);
        contract.check("get", resume, &app.get(&format!("/v1/videos/{nobody}/upload")).jar(&owner).send().await);
        let done = app.post(&finish).jar(&owner).json(json!({"size_bytes": 6 * MIB})).send().await;
        assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
        assert_eq!(done.body["video"]["status"], "processing");
        contract.check("post", complete, &done);
        // Completing a finished upload is not an error, and the part routes then refuse.
        contract.check("post", complete, &app.post(&finish).jar(&owner).json(json!({"size_bytes": 6 * MIB})).send().await);
        let after = app.post(&uri).jar(&owner).json(json!({"number": 1, "size": 5 * MIB})).send().await;
        assert_eq!(after.code(), "not_streaming");
        contract.check("post", parts, &after);
        let finished = app.get(&progress).jar(&owner).send().await;
        assert_eq!(finished.code(), "not_uploading");
        contract.check("get", resume, &finished);

        // Running out of budget: ten unfinished uploads, then the parts one person may ask for in an hour.
        let mut open_ids = Vec::new();
        let mut capped = None;
        for _ in 0..12 {
            let res = app.post(start).jar(&owner).json(started.clone()).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                capped = Some(res);
                break;
            }
            open_ids.push(res.body["video"]["id"].as_str().unwrap().to_owned());
        }
        let capped = capped.expect("unfinished uploads are capped");
        assert_eq!(capped.code(), "too_many_uploads");
        contract.check("post", start, &capped);
        let still_open = format!("/v1/videos/{}/parts", open_ids[0]);
        let mut limited = None;
        for _ in 0..2100 {
            let res = app.post(&still_open).jar(&owner).json(json!({"number": 1, "size": 5 * MIB})).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("post", parts, &limited.expect("asking for parts runs out"));

        // No storage left: told when a recording starts, and when its next part would not fit.
        let full = signup_creator(&app, "full@example.com").await;
        let made = app.post(start).jar(&full).json(started.clone()).send().await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
        let full_id = made.body["video"]["id"].as_str().unwrap().to_owned();
        sqlx::query("update users set storage_quota_bytes = $2 where email = $1")
            .bind("full@example.com")
            .bind(5 * MIB as i64)
            .execute(&app.pool)
            .await
            .unwrap();
        let no_room = app.post(&format!("/v1/videos/{full_id}/parts")).jar(&full).json(json!({"number": 2, "size": 5 * MIB})).send().await;
        assert_eq!(no_room.status, StatusCode::PAYLOAD_TOO_LARGE, "{:?}", no_room.body);
        assert_eq!(no_room.code(), "storage_quota_exceeded");
        contract.check("post", parts, &no_room);
        sqlx::query("update users set storage_quota_bytes = 0 where email = $1")
            .bind("full@example.com")
            .execute(&app.pool)
            .await
            .unwrap();
        let cannot_start = app.post(start).jar(&full).json(started.clone()).send().await;
        assert_eq!(cannot_start.status, StatusCode::PAYLOAD_TOO_LARGE, "{:?}", cannot_start.body);
        assert_eq!(cannot_start.code(), "storage_quota_exceeded");
        contract.check("post", start, &cannot_start);

        // What can go wrong on any route.
        contract
            .provoke_common_errors(&app, "/v1/videos", Some(&owner), &[("id", &nobody.to_string())])
            .await;

        let unseen = contract.unseen("/v1/videos");
        assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
    }
}

db_test! {
    async fn the_comments_api_keeps_its_promises(pool: PgPool) {
        let app = app(pool.clone());
        let contract = Contract::new();
        let owner = signup_creator(&app, "teacher@example.com").await;
        let pupil = signup_verified_as(&app, "pupil@example.com", "student").await;
        let other = signup_verified_as(&app, "other@example.com", "student").await;
        let stranger = signup_creator(&app, "stranger@example.com").await;
        let nobody = uuid::Uuid::new_v4();

        let video = common::media::ready_video(&app, &owner, &pool).await;
        let made = app.post("/v1/recordings").jar(&owner).json(json!({"source": "extension", "title": "Week 3", "video_id": video})).send().await;
        let recording = made.body["recording"]["id"].as_str().unwrap().to_owned();
        let open = app.post(&format!("/v1/recordings/{recording}/links")).jar(&owner).json(json!({})).send().await;
        let code = open.body["link"]["code"].as_str().unwrap().to_owned();
        let locked = app.post(&format!("/v1/recordings/{recording}/links")).jar(&owner).json(json!({"password": "open sesame"})).send().await;
        let locked_code = locked.body["link"]["code"].as_str().unwrap().to_owned();
        let closed = app.post(&format!("/v1/recordings/{recording}/links")).jar(&owner).json(json!({})).send().await;
        let closed_code = closed.body["link"]["code"].as_str().unwrap().to_owned();
        let revoked = app.req(axum::http::Method::DELETE, &format!("/v1/recordings/{recording}/links/{}", closed.body["link"]["id"].as_str().unwrap())).jar(&owner).send().await;
        assert_eq!(revoked.status, StatusCode::NO_CONTENT);

        let on_link = "/v1/s/{code}/comments";
        let on_owner = "/v1/recordings/{id}/comments";
        let one = "/v1/comments/{id}";

        // Leaving one, and every way that can go wrong.
        let said = app.post(&format!("/v1/s/{code}/comments")).jar(&pupil).json(json!({"kind": "comment", "body": "Nice", "at_ms": 1500})).send().await;
        assert_eq!(said.status, StatusCode::CREATED, "{:?}", said.body);
        contract.check("post", on_link, &said);
        let reacted = app.post(&format!("/v1/s/{code}/comments")).jar(&pupil).json(json!({"kind": "reaction", "emoji": "🔥", "at_ms": 3000})).send().await;
        assert_eq!(reacted.status, StatusCode::CREATED);
        contract.check("post", on_link, &reacted);
        contract.check("post", on_link, &app.post(&format!("/v1/s/{code}/comments")).json(json!({"kind": "comment", "body": "x"})).send().await);
        contract.check("post", on_link, &app.post(&format!("/v1/s/{locked_code}/comments")).jar(&pupil).json(json!({"kind": "comment", "body": "x"})).send().await);
        contract.check("post", on_link, &app.post(&format!("/v1/s/{}/comments", "A".repeat(22))).jar(&pupil).json(json!({"kind": "comment", "body": "x"})).send().await);
        contract.check("post", on_link, &app.post(&format!("/v1/s/{closed_code}/comments")).jar(&pupil).json(json!({"kind": "comment", "body": "x"})).send().await);
        contract.check("post", on_link, &app.post(&format!("/v1/s/{code}/comments")).jar(&other).json(json!({"kind": "comment", "body": ""})).send().await);
        let id = uuid::Uuid::new_v4();
        let first = app.post(&format!("/v1/s/{code}/comments")).jar(&other).json(json!({"id": id, "kind": "comment", "body": "same"})).send().await;
        contract.check("post", on_link, &first);
        let clash = app.post(&format!("/v1/s/{code}/comments")).jar(&other).json(json!({"id": id, "kind": "comment", "body": "different"})).send().await;
        assert_eq!(clash.code(), "comment_id_conflict");
        contract.check("post", on_link, &clash);

        // Reading it, a page at a time.
        let page = app.get(&format!("/v1/s/{code}/comments?limit=1")).send().await;
        assert_eq!(page.status, StatusCode::OK, "{:?}", page.body);
        contract.check("get", on_link, &page);
        let cursor = page.body["next_cursor"].as_str().expect("a second page").to_owned();
        contract.check("get", on_link, &app.get(&format!("/v1/s/{code}/comments?limit=1&cursor={cursor}")).jar(&pupil).send().await);
        let bad = app.get(&format!("/v1/s/{code}/comments?cursor=garbage")).send().await;
        assert_eq!(bad.code(), "invalid_cursor");
        contract.check("get", on_link, &bad);
        contract.check("get", on_link, &app.get(&format!("/v1/s/{code}/comments?limit=0")).send().await);
        contract.check("get", on_link, &app.get(&format!("/v1/s/{locked_code}/comments")).send().await);
        contract.check("get", on_link, &app.get(&format!("/v1/s/{}/comments", "A".repeat(22))).send().await);
        contract.check("get", on_link, &app.get(&format!("/v1/s/{closed_code}/comments")).send().await);

        // The owner's view, and closing the conversation.
        let theirs = app.get(&format!("/v1/recordings/{recording}/comments")).jar(&owner).send().await;
        assert_eq!(theirs.status, StatusCode::OK, "{:?}", theirs.body);
        contract.check("get", on_owner, &theirs);
        contract.check("get", on_owner, &app.get(&format!("/v1/recordings/{recording}/comments")).send().await);
        contract.check("get", on_owner, &app.get(&format!("/v1/recordings/{recording}/comments")).jar(&pupil).send().await);
        contract.check("get", on_owner, &app.get(&format!("/v1/recordings/{nobody}/comments")).jar(&owner).send().await);
        contract.check("get", on_owner, &app.get(&format!("/v1/recordings/{recording}/comments")).jar(&stranger).send().await);
        contract.check("get", on_owner, &app.get(&format!("/v1/recordings/{recording}/comments?cursor=garbage")).jar(&owner).send().await);
        contract.check("get", on_owner, &app.get(&format!("/v1/recordings/{recording}/comments?limit=999")).jar(&owner).send().await);
        let version = app.get(&format!("/v1/recordings/{recording}")).jar(&owner).send().await.body["recording"]["version"].clone();
        let off = app.req(axum::http::Method::PATCH, &format!("/v1/recordings/{recording}")).jar(&owner).json(json!({"version": version, "comments_enabled": false})).send().await;
        assert_eq!(off.status, StatusCode::OK, "{:?}", off.body);
        let closed_talk = app.post(&format!("/v1/s/{code}/comments")).jar(&pupil).json(json!({"kind": "comment", "body": "x"})).send().await;
        assert_eq!(closed_talk.code(), "comments_off");
        contract.check("post", on_link, &closed_talk);
        let shut = app.get(&format!("/v1/s/{code}/comments")).send().await;
        assert_eq!(shut.body["comments_enabled"], false);
        contract.check("get", on_link, &shut);

        // Removing one.
        let mine = said.body["comment"]["id"].as_str().unwrap().to_owned();
        contract.check("delete", one, &app.req(axum::http::Method::DELETE, &format!("/v1/comments/{mine}")).send().await);
        contract.check("delete", one, &app.req(axum::http::Method::DELETE, &format!("/v1/comments/{mine}")).jar(&other).send().await);
        contract.check("delete", one, &app.req(axum::http::Method::DELETE, "/v1/comments/not-an-id").jar(&owner).send().await);
        let gone = app.req(axum::http::Method::DELETE, &format!("/v1/comments/{mine}")).jar(&pupil).send().await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        contract.check("delete", one, &gone);

        // Running out of budget: reading by address, and leaving or removing by account.
        let mut limited = None;
        for _ in 0..130 {
            let res = app.get(&format!("/v1/s/{code}/comments")).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("get", on_link, &limited.expect("reading runs out"));
        let mut limited = None;
        for _ in 0..130 {
            let res = app.get(&format!("/v1/recordings/{recording}/comments")).jar(&owner).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("get", on_owner, &limited.expect("the owner's reading runs out"));
        let mut limited = None;
        for _ in 0..15 {
            let res = app.req(axum::http::Method::DELETE, &format!("/v1/comments/{nobody}")).jar(&other).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("delete", one, &limited.expect("removing runs out"));
        // Posting runs out too: the account that has just been removing is the one with the budget spent.
        contract.check(
            "post",
            on_link,
            &app.post(&format!("/v1/s/{code}/comments")).jar(&other).json(json!({"kind": "comment", "body": "x"})).send().await,
        );

        // What can go wrong on any route.
        contract.provoke_common_errors(&app, on_link, Some(&pupil), &[("code", &code)]).await;
        contract.provoke_common_errors(&app, "/v1/comments", Some(&pupil), &[("id", &nobody.to_string())]).await;

        for prefix in [on_link, on_owner, one] {
            let unseen = contract.unseen(prefix);
            assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
        }
    }
}

db_test! {
    async fn the_account_api_keeps_its_promises(pool: PgPool) {
        let app = app(pool);
        let contract = Contract::new();
        let owner = signup_creator(&app, "keeper@example.com").await;
        let storage = "/v1/me/storage";

        let signed_out = app.get(storage).send().await;
        assert_eq!(signed_out.status, StatusCode::UNAUTHORIZED, "{:?}", signed_out.body);
        contract.check("get", storage, &signed_out);

        let shown = app.get(storage).jar(&owner).send().await;
        assert_eq!(shown.status, StatusCode::OK, "{:?}", shown.body);
        assert_eq!(shown.body["storage"]["used_bytes"], 0);
        contract.check("get", storage, &shown);

        // The library asks after every upload and delete, but not without end.
        let mut limited = None;
        for _ in 0..130 {
            let res = app.get(storage).jar(&owner).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("get", storage, &limited.expect("reading it runs out"));

        contract.provoke_common_errors(&app, storage, Some(&owner), &[]).await;
        let unseen = contract.unseen(storage);
        assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
    }
}

db_test! {
    async fn the_workspaces_api_keeps_its_promises(pool: PgPool) {
        use axum::http::Method;
        let app = app(pool.clone());
        let contract = Contract::new();
        let (list, one, member, invites, one_invite, accept) = (
            "/v1/workspaces",
            "/v1/workspaces/{id}",
            "/v1/workspaces/{id}/members/{user}",
            "/v1/workspaces/{id}/invites",
            "/v1/workspaces/{id}/invites/{invite}",
            "/v1/invites/accept",
        );
        let nobody = uuid::Uuid::new_v4();
        let owner = signup_creator(&app, "owner@example.com").await;
        let pupil = signup_verified_as(&app, "pupil@example.com", "student").await;
        let stranger = signup_verified_as(&app, "stranger@example.com", "student").await;
        let owner_id: uuid::Uuid = sqlx::query_scalar("select id from users where email = 'owner@example.com'").fetch_one(&pool).await.unwrap();
        let pupil_id: uuid::Uuid = sqlx::query_scalar("select id from users where email = 'pupil@example.com'").fetch_one(&pool).await.unwrap();

        // The shape each route takes, for the probes that need a body.
        let body_of = |path: &str, method: &str| -> Option<Value> {
            match (method, path) {
                ("post", "/v1/workspaces") | ("patch", "/v1/workspaces/{id}") => Some(json!({"name": "Maths 9A"})),
                ("patch", "/v1/workspaces/{id}/members/{user}") => Some(json!({"role": "member"})),
                ("post", "/v1/workspaces/{id}/invites") => Some(json!({"email": "someone@example.com", "role": "member"})),
                ("post", "/v1/invites/accept") => Some(json!({"token": "x"})),
                _ => None,
            }
        };
        let send = |method: &str, path: &str, jar: Option<&Jar>, fill: &[(&str, String)], body: Option<Value>| {
            let uri = fill.iter().fold(path.to_owned(), |uri, (name, value)| uri.replace(&format!("{{{name}}}"), value));
            let mut req = app.req(method.to_uppercase().parse::<Method>().unwrap(), &uri);
            if let Some(jar) = jar {
                req = req.jar(jar);
            }
            if let Some(body) = body {
                req = req.json(body);
            }
            let (method, path) = (method.to_owned(), path.to_owned());
            async move {
                let res = req.send().await;
                (method, path, res)
            }
        };
        let id_of = |res: &Res| res.body["workspace"]["id"].as_str().unwrap().to_owned();
        let latest_token = |to: &'static str| {
            let app = &app;
            async move {
                app.settle().await;
                let mail = app.mailer.sent().into_iter().rev().find(|m| m.to == to && m.subject.contains("invited you")).expect("the invitation");
                token_from(&mail)
            }
        };

        // Signed out: every route says so.
        for (method, path, _) in contract.operations("/v1/workspaces").into_iter().chain(contract.operations("/v1/invites")) {
            let fill = [("id", nobody.to_string()), ("user", nobody.to_string()), ("invite", nobody.to_string())];
            let (m, p, res) = send(&method, &path, None, &fill, body_of(&path, &method)).await;
            assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{m} {p}: {:?}", res.body);
            contract.check(&m, &p, &res);
        }

        // Starting one, and reading it.
        let made = app.post(list).jar(&owner).json(json!({"name": "Maths 9A"})).send().await;
        assert_eq!(made.status, StatusCode::CREATED, "{:?}", made.body);
        contract.check("post", list, &made);
        let ws = id_of(&made);
        let at = [("id", ws.clone()), ("user", pupil_id.to_string()), ("invite", nobody.to_string())];
        let (_, _, listed) = send("get", list, Some(&owner), &at, None).await;
        contract.check("get", list, &listed);
        let (_, _, shown) = send("get", one, Some(&owner), &at, None).await;
        assert_eq!(shown.status, StatusCode::OK, "{:?}", shown.body);
        contract.check("get", one, &shown);
        let (_, _, hidden) = send("get", one, Some(&stranger), &at, None).await;
        assert_eq!(hidden.status, StatusCode::NOT_FOUND);
        contract.check("get", one, &hidden);

        // Inviting, and every way that can go wrong.
        let (_, _, sent) = send("post", invites, Some(&owner), &at, Some(json!({"email": "pupil@example.com", "role": "member"}))).await;
        assert_eq!(sent.status, StatusCode::CREATED, "{:?}", sent.body);
        contract.check("post", invites, &sent);
        let invite_id = sent.body["invite"]["id"].as_str().unwrap().to_owned();
        let (_, _, twice) = send("post", invites, Some(&owner), &at, Some(json!({"email": "pupil@example.com", "role": "member"}))).await;
        assert_eq!(twice.code(), "already_invited");
        contract.check("post", invites, &twice);
        let (_, _, bad) = send("post", invites, Some(&owner), &at, Some(json!({"email": "nope", "role": "member"}))).await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("post", invites, &bad);
        let (_, _, away) = send("post", invites, Some(&stranger), &at, Some(json!({"email": "x@example.com", "role": "member"}))).await;
        contract.check("post", invites, &away);
        let (_, _, seen) = send("get", invites, Some(&owner), &at, None).await;
        assert_eq!(seen.body["invites"].as_array().unwrap().len(), 1);
        contract.check("get", invites, &seen);
        contract.check("get", invites, &send("get", invites, Some(&stranger), &at, None).await.2);

        // Accepting: not a good link, somebody else's, and the right person's.
        let (_, _, wrong_token) = send("post", accept, Some(&pupil), &at, Some(json!({"token": "not-a-real-token"}))).await;
        assert_eq!(wrong_token.code(), "invalid_token");
        contract.check("post", accept, &wrong_token);
        let token = latest_token("pupil@example.com").await;
        let (_, _, not_yours) = send("post", accept, Some(&stranger), &at, Some(json!({"token": token}))).await;
        assert_eq!(not_yours.code(), "wrong_account");
        contract.check("post", accept, &not_yours);
        let (_, _, joined) = send("post", accept, Some(&pupil), &at, Some(json!({"token": token}))).await;
        assert_eq!(joined.status, StatusCode::OK, "{:?}", joined.body);
        contract.check("post", accept, &joined);

        // A workspace with fifty in it has no room for one more.
        let full = id_of(&app.post(list).jar(&owner).json(json!({"name": "Full house"})).send().await);
        let (_, _, to_full) = send("post", invites, Some(&owner), &[("id", full.clone())], Some(json!({"email": "pupil@example.com", "role": "member"}))).await;
        assert_eq!(to_full.status, StatusCode::CREATED, "{:?}", to_full.body);
        let full_token = latest_token("pupil@example.com").await;
        for n in 0..49 {
            let filler = uuid::Uuid::new_v4();
            sqlx::query("insert into users (id, name, email, password_hash, role) values ($1, 'Filler', $2, 'x', 'student')")
                .bind(filler).bind(format!("filler{n}@example.com")).execute(&pool).await.unwrap();
            sqlx::query("insert into workspace_members (workspace_id, user_id, role) values ($1::uuid, $2, 'member')")
                .bind(&full).bind(filler).execute(&pool).await.unwrap();
        }
        let (_, _, no_room) = send("post", accept, Some(&pupil), &at, Some(json!({"token": full_token}))).await;
        assert_eq!(no_room.status, StatusCode::CONFLICT, "{:?}", no_room.body);
        contract.check("post", accept, &no_room);

        // Renaming.
        let (_, _, renamed) = send("patch", one, Some(&owner), &at, Some(json!({"name": "Maths 9B"}))).await;
        assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
        contract.check("patch", one, &renamed);
        for (jar, want) in [(&pupil, 403), (&stranger, 404)] {
            let (_, _, res) = send("patch", one, Some(jar), &at, Some(json!({"name": "Mine"}))).await;
            assert_eq!(res.status.as_u16(), want, "{:?}", res.body);
            contract.check("patch", one, &res);
        }
        let (_, _, blank) = send("patch", one, Some(&owner), &at, Some(json!({"name": " "}))).await;
        assert_eq!(blank.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("patch", one, &blank);

        // Roles and leaving.
        let (_, _, promoted) = send("patch", member, Some(&owner), &at, Some(json!({"role": "admin"}))).await;
        assert_eq!(promoted.status, StatusCode::NO_CONTENT, "{:?}", promoted.body);
        contract.check("patch", member, &promoted);
        let (_, _, refused) = send("patch", member, Some(&pupil), &at, Some(json!({"role": "owner"}))).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN);
        contract.check("patch", member, &refused);
        let (_, _, unknown) = send("patch", member, Some(&owner), &[("id", ws.clone()), ("user", nobody.to_string())], Some(json!({"role": "member"}))).await;
        assert_eq!(unknown.status, StatusCode::NOT_FOUND);
        contract.check("patch", member, &unknown);
        let (_, _, odd) = send("patch", member, Some(&owner), &at, Some(json!({"role": "king"}))).await;
        assert_eq!(odd.status, StatusCode::UNPROCESSABLE_ENTITY);
        contract.check("patch", member, &odd);
        let mine = [("id", ws.clone()), ("user", owner_id.to_string())];
        let (_, _, last) = send("patch", member, Some(&owner), &mine, Some(json!({"role": "member"}))).await;
        assert_eq!(last.code(), "last_owner");
        contract.check("patch", member, &last);
        let (_, _, stuck) = send("delete", member, Some(&owner), &mine, None).await;
        assert_eq!(stuck.code(), "last_owner");
        contract.check("delete", member, &stuck);
        let (_, _, not_allowed) = send("delete", member, Some(&pupil), &mine, None).await;
        assert_eq!(not_allowed.status, StatusCode::FORBIDDEN);
        contract.check("delete", member, &not_allowed);
        let (_, _, not_there) = send("delete", member, Some(&owner), &[("id", ws.clone()), ("user", nobody.to_string())], None).await;
        assert_eq!(not_there.status, StatusCode::NOT_FOUND);
        contract.check("delete", member, &not_there);

        // Taking an invitation back, in a workspace with one still open.
        let (_, _, again) = send("post", invites, Some(&owner), &at, Some(json!({"email": "later@example.com", "role": "member"}))).await;
        assert_eq!(again.status, StatusCode::CREATED);
        let open = again.body["invite"]["id"].as_str().unwrap().to_owned();
        let with = |invite: &str| [("id", ws.clone()), ("user", nobody.to_string()), ("invite", invite.to_owned())];
        let (_, _, gone) = send("delete", one_invite, Some(&owner), &with(&open), None).await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT, "{:?}", gone.body);
        contract.check("delete", one_invite, &gone);
        for (jar, invite, want) in [(&pupil, invite_id.clone(), 404), (&owner, nobody.to_string(), 404), (&stranger, open.clone(), 404)] {
            let (_, _, res) = send("delete", one_invite, Some(jar), &with(&invite), None).await;
            assert_eq!(res.status.as_u16(), want, "{:?}", res.body);
            contract.check("delete", one_invite, &res);
        }

        // Removing: an owner removes the pupil, who is out.
        let (_, _, removed) = send("delete", member, Some(&owner), &at, None).await;
        assert_eq!(removed.status, StatusCode::NO_CONTENT, "{:?}", removed.body);
        contract.check("delete", member, &removed);

        // What can go wrong on any route.
        let fills = [("id", ws.clone()), ("user", nobody.to_string()), ("invite", nobody.to_string())];
        let fill_refs: Vec<(&str, &str)> = fills.iter().map(|(a, b)| (*a, b.as_str())).collect();
        contract.provoke_common_errors(&app, "/v1/workspaces", Some(&owner), &fill_refs).await;
        contract.provoke_common_errors(&app, "/v1/invites", Some(&owner), &fill_refs).await;

        // Ending it.
        for (jar, want) in [(&pupil, 404), (&stranger, 404)] {
            let (_, _, res) = send("delete", one, Some(jar), &at, None).await;
            assert_eq!(res.status.as_u16(), want, "{:?}", res.body);
            contract.check("delete", one, &res);
        }
        // A person who is only a member may not (on a workspace where the pupil is one).
        let other = id_of(&app.post(list).jar(&owner).json(json!({"name": "Second"})).send().await);
        send("post", invites, Some(&owner), &[("id", other.clone())], Some(json!({"email": "pupil@example.com", "role": "member"}))).await;
        let second = latest_token("pupil@example.com").await;
        send("post", accept, Some(&pupil), &at, Some(json!({"token": second}))).await;
        let (_, _, only_member) = send("delete", one, Some(&pupil), &[("id", other.clone())], None).await;
        assert_eq!(only_member.status, StatusCode::FORBIDDEN, "{:?}", only_member.body);
        contract.check("delete", one, &only_member);
        let in_other = [("id", other.clone()), ("user", nobody.to_string()), ("invite", nobody.to_string())];
        let (_, _, may_not_see) = send("get", invites, Some(&pupil), &in_other, None).await;
        assert_eq!(may_not_see.status, StatusCode::FORBIDDEN, "{:?}", may_not_see.body);
        contract.check("get", invites, &may_not_see);
        let (_, _, may_not_ask) = send("post", invites, Some(&pupil), &in_other, Some(json!({"email": "x@example.com", "role": "member"}))).await;
        assert_eq!(may_not_ask.status, StatusCode::FORBIDDEN, "{:?}", may_not_ask.body);
        contract.check("post", invites, &may_not_ask);
        let (_, _, may_not_revoke) = send("delete", one_invite, Some(&pupil), &in_other, None).await;
        assert_eq!(may_not_revoke.status, StatusCode::FORBIDDEN, "{:?}", may_not_revoke.body);
        contract.check("delete", one_invite, &may_not_revoke);
        let (_, _, ended) = send("delete", one, Some(&owner), &at, None).await;
        assert_eq!(ended.status, StatusCode::NO_CONTENT, "{:?}", ended.body);
        contract.check("delete", one, &ended);

        // Owning at most five.
        let mut too_many = None;
        for n in 0..7 {
            let res = app.post(list).jar(&stranger).json(json!({"name": format!("Stranger {n}")})).send().await;
            if res.status == StatusCode::CONFLICT {
                too_many = Some(res);
                break;
            }
        }
        let too_many = too_many.expect("an account owns at most five");
        assert_eq!(too_many.code(), "too_many_workspaces");
        contract.check("post", list, &too_many);

        // The budgets run out: writes a few a minute, reads more. Once spent, every route of that kind says so.
        let mut limited = None;
        for _ in 0..40 {
            let res = app.post(list).jar(&owner).json(json!({"name": "x"})).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("post", list, &limited.expect("changing runs out"));
        for (method, path) in [("post", list), ("patch", one), ("delete", one), ("patch", member), ("delete", member), ("post", invites), ("delete", one_invite), ("post", accept)] {
            let (_, _, res) = send(method, path, Some(&owner), &at, body_of(path, method)).await;
            assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{method} {path}: {:?}", res.body);
            contract.check(method, path, &res);
        }
        let mut limited = None;
        for _ in 0..130 {
            let res = app.get(list).jar(&owner).send().await;
            if res.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(res);
                break;
            }
        }
        contract.check("get", list, &limited.expect("reading runs out"));
        for path in [one, invites] {
            let (_, _, res) = send("get", path, Some(&owner), &at, None).await;
            assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "get {path}");
            contract.check("get", path, &res);
        }

        for prefix in ["/v1/workspaces", "/v1/invites"] {
            let unseen = contract.unseen(prefix);
            assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
        }
    }
}
