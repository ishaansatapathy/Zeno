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

/// The routes the auth router declares, read from its source: `.route("/login", post(login))`.
fn declared_auth_routes() -> BTreeSet<String> {
    let source = std::fs::read_to_string(repo_file("crates/api/src/auth/routes.rs")).unwrap();
    let mut found = BTreeSet::new();
    for line in source
        .lines()
        .filter(|l| l.trim_start().starts_with(".route(\""))
    {
        let rest = line.trim_start().trim_start_matches(".route(\"");
        let path = rest.split('"').next().unwrap();
        for method in ["get", "post", "put", "patch", "delete"] {
            if rest.contains(&format!("{method}(")) {
                found.insert(format!("{method} /v1/auth{path}"));
            }
        }
    }
    found
}

#[test]
fn every_auth_route_is_documented_and_every_documented_route_exists() {
    let doc = document();
    let mut documented = BTreeSet::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        if !path.starts_with("/v1/auth/") {
            continue;
        }
        for method in ["get", "post", "put", "patch", "delete"] {
            if item.get(method).is_some() {
                documented.insert(format!("{method} {path}"));
            }
        }
    }
    let declared = declared_auth_routes();
    assert!(declared.len() >= 10, "the source scan found {declared:?}");
    assert_eq!(
        declared, documented,
        "routes and the OpenAPI document disagree (left: declared in the router, right: documented)"
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

/// Checks real responses against the document and remembers which ones it has seen.
struct Contract {
    doc: Value,
    seen: std::sync::Mutex<BTreeSet<String>>,
}

impl Contract {
    fn new() -> Self {
        Self {
            doc: document(),
            seen: Default::default(),
        }
    }

    fn check(&self, method: &str, path: &str, res: &Res) {
        let who = format!("{} {path} -> {}", method.to_uppercase(), res.status);
        let operation = &self.doc["paths"][path][method];
        assert!(operation.is_object(), "{who}: not in the document");
        let status = res.status.as_u16().to_string();
        let response = &operation["responses"][&status];
        assert!(
            response.is_object(),
            "{who}: the document does not list this status ({:?})",
            res.body
        );
        for name in response["headers"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(n, _)| n)
        {
            assert!(
                res.headers.contains_key(name.as_str()),
                "{who}: missing header {name}"
            );
        }
        match response["content"]["application/json"].get("schema") {
            Some(schema) => self.validate(&who, schema, &res.body),
            None => assert!(
                res.body.is_null(),
                "{who}: the document lists no body, got {:?}",
                res.body
            ),
        }
        self.seen
            .lock()
            .unwrap()
            .insert(format!("{method} {path} {status}"));
    }

    fn validate(&self, who: &str, schema: &Value, body: &Value) {
        // A schema that points into `components` needs `components` beside it to resolve.
        let mut root = schema.clone();
        root["components"] = self.doc["components"].clone();
        let validator = jsonschema::validator_for(&root).expect("the schema compiles");
        let problems: Vec<String> = validator.iter_errors(body).map(|e| e.to_string()).collect();
        assert!(
            problems.is_empty(),
            "{who}: the body does not match its schema: {problems:?}\n{body}"
        );
    }

    /// The responses the document lists that no test provoked.
    fn unseen(&self) -> Vec<String> {
        let seen = self.seen.lock().unwrap();
        let mut missing = Vec::new();
        for (path, item) in self.doc["paths"].as_object().unwrap() {
            for method in ["get", "post", "put", "patch", "delete"] {
                let Some(operation) = item.get(method) else {
                    continue;
                };
                for status in operation["responses"].as_object().unwrap().keys() {
                    // Our own bug, and the limiter being down: not things a test can ask for.
                    if status == "500" || status == "503" {
                        continue;
                    }
                    let key = format!("{method} {path} {status}");
                    if !seen.contains(&key) {
                        missing.push(key);
                    }
                }
            }
        }
        missing
    }
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

        // What can go wrong on any route is documented once, so it is provoked on every route.
        let operations: Vec<(String, String, bool)> = contract.doc["paths"]
            .as_object()
            .unwrap()
            .iter()
            .flat_map(|(path, item)| {
                ["get", "post", "put", "patch", "delete"]
                    .into_iter()
                    .filter_map(|method| {
                        item.get(method)
                            .map(|op| (method.to_owned(), path.clone(), op.get("requestBody").is_some()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        for (method, path, has_body) in operations {
            let send = |body: Option<(Vec<u8>, &'static str)>, origin: Option<&'static str>| {
                let mut req = app.req(method.to_uppercase().parse().unwrap(), &path);
                if let Some((bytes, content_type)) = body {
                    req = req.raw_body(bytes, Some(content_type));
                }
                if let Some(origin) = origin {
                    req = req.origin(origin);
                }
                req.send()
            };
            if method != "get" {
                let res = send(None, Some("https://evil.example")).await;
                assert_eq!(res.status, StatusCode::FORBIDDEN, "{method} {path}");
                assert_eq!(res.code(), "forbidden_origin");
                contract.check(&method, &path, &res);
            }
            if has_body {
                for (body, content_type, expected) in [
                    (b"{".to_vec(), "application/json", StatusCode::BAD_REQUEST),
                    (b"[]".to_vec(), "application/json", StatusCode::UNPROCESSABLE_ENTITY),
                    (b"x".to_vec(), "text/plain", StatusCode::UNSUPPORTED_MEDIA_TYPE),
                    (vec![b' '; 3_000_000], "application/json", StatusCode::PAYLOAD_TOO_LARGE),
                ] {
                    let res = send(Some((body, content_type)), None).await;
                    assert_eq!(res.status, expected, "{method} {path}");
                    contract.check(&method, &path, &res);
                }
                // A well-shaped body with invalid values is a `validation_failed` where the route
                // validates fields; that, and the other documented outcomes, are covered above.
            }
        }

        let unseen = contract.unseen();
        assert!(unseen.is_empty(), "the document promises responses no test has seen: {unseen:#?}");
    }
}
