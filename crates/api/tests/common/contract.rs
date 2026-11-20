//! Holds real responses to the OpenAPI document: the status must be one it lists, the headers it
//! declares must be there, and the body must validate against the schema it gives. Remembers what
//! it has seen, so a test can insist that every promise in the document was observed at least once.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::sync::Mutex;

use axum::http::{Method, StatusCode};
use serde_json::Value;

use super::*;

pub const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

pub struct Contract {
    pub doc: Value,
    seen: Mutex<BTreeSet<String>>,
}

impl Contract {
    pub fn new() -> Self {
        Self {
            doc: serde_json::to_value(zeno_api::openapi::document()).unwrap(),
            seen: Default::default(),
        }
    }

    /// `method` is lower case; `path` is the template as the document writes it (`/v1/recordings/{id}`).
    pub fn check(&self, method: &str, path: &str, res: &Res) {
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
            .map(|(name, _)| name)
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
        let problems: Vec<String> = validator
            .iter_errors(body)
            .map(|error| error.to_string())
            .collect();
        assert!(
            problems.is_empty(),
            "{who}: the body does not match its schema: {problems:?}\n{body}"
        );
    }

    /// Every operation whose path starts with `prefix`: (method, path template, takes a body).
    pub fn operations(&self, prefix: &str) -> Vec<(String, String, bool)> {
        let mut found = Vec::new();
        for (path, item) in self.doc["paths"].as_object().unwrap() {
            if !path.starts_with(prefix) {
                continue;
            }
            for method in METHODS {
                if let Some(operation) = item.get(method) {
                    found.push((
                        method.to_owned(),
                        path.clone(),
                        operation.get("requestBody").is_some(),
                    ));
                }
            }
        }
        found
    }

    /// The responses the document lists under `prefix` that no test provoked. Our own bug (`500`)
    /// and the limiter being down (`503`) are not things a test can ask for.
    pub fn unseen(&self, prefix: &str) -> Vec<String> {
        let seen = self.seen.lock().unwrap();
        let mut missing = Vec::new();
        for (method, path, _) in self.operations(prefix) {
            let operation = &self.doc["paths"][&path][&method];
            for status in operation["responses"].as_object().unwrap().keys() {
                if status == "500" || status == "503" {
                    continue;
                }
                let key = format!("{method} {path} {status}");
                if !seen.contains(&key) {
                    missing.push(key);
                }
            }
        }
        missing
    }

    /// What can go wrong on any route is documented once, so it is provoked on every route under
    /// `prefix`: a request from another site, unreadable JSON, JSON of the wrong shape, the wrong
    /// media type and a body that is too large. `fill` replaces path parameters (`("id", "...")`).
    pub async fn provoke_common_errors(
        &self,
        app: &App,
        prefix: &str,
        jar: Option<&Jar>,
        fill: &[(&str, &str)],
    ) {
        for (method, path, has_body) in self.operations(prefix) {
            let uri = fill.iter().fold(path.clone(), |uri, (name, value)| {
                uri.replace(&format!("{{{name}}}"), value)
            });
            let send = |body: Option<(Vec<u8>, &'static str)>, origin: Option<&'static str>| {
                let mut req = app.req(method.to_uppercase().parse::<Method>().unwrap(), &uri);
                if let Some(jar) = jar {
                    req = req.jar(jar);
                }
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
                assert_eq!(res.code(), "forbidden_origin", "{method} {path}");
                self.check(&method, &path, &res);
            }
            if has_body {
                for (body, content_type, expected) in [
                    (b"{".to_vec(), "application/json", StatusCode::BAD_REQUEST),
                    (
                        b"[]".to_vec(),
                        "application/json",
                        StatusCode::UNPROCESSABLE_ENTITY,
                    ),
                    (
                        b"x".to_vec(),
                        "text/plain",
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    ),
                    (
                        vec![b' '; 3_000_000],
                        "application/json",
                        StatusCode::PAYLOAD_TOO_LARGE,
                    ),
                ] {
                    let res = send(Some((body, content_type)), None).await;
                    assert_eq!(res.status, expected, "{method} {path}");
                    self.check(&method, &path, &res);
                }
            }
        }
    }
}
