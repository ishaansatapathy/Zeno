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
];

/// Routes that exist and are not in the document yet. The list is explicit so that a new route
/// cannot be forgotten silently, and it can only shrink: a route here that is documented, or that
/// no longer exists, fails the test below.
const NOT_DESCRIBED_YET: &[&str] = &[
    "put /v1/recordings/{id}/video",
    "get /v1/recordings/{id}/views",
    "get /v1/s/{code}",
    "post /v1/s/{code}/play",
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

        contract.provoke_common_errors(&app, "/v1/auth/", None, &[]).await;

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

        let unseen = contract.unseen("/v1/recordings");
        assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
    }
}
