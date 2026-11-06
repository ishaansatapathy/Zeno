#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use zeno_api::auth::counters::MemoryCounters;
use zeno_api::auth::password::{PasswordParams, Passwords};
use zeno_api::config::ApiConfig;
use zeno_api::mail::{Mailer, MemoryMailer, OutgoingEmail};
use zeno_api::state::AppState;
use zeno_core::config::{Config, Environment};
use zeno_core::storage::{S3Store, StorageConfig};

pub const ORIGIN: &str = "http://localhost:3000";
pub const PASSWORD: &str = "correct horse battery staple";
pub const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789ab";

pub struct App {
    pub router: Router,
    pub mailer: MemoryMailer,
    pub pool: PgPool,
    pub state: AppState,
}

impl App {
    /// Waits for fire-and-forget work (emails, follow-up jobs) started by earlier requests.
    pub async fn settle(&self) {
        self.state.background.idle().await;
    }
}

pub fn app(pool: PgPool) -> App {
    build(pool, false, 0)
}

pub fn build(pool: PgPool, cookie_secure: bool, trusted_proxy_hops: usize) -> App {
    let mailer = MemoryMailer::default();
    let state = make_state(
        pool.clone(),
        cookie_secure,
        trusted_proxy_hops,
        Arc::new(mailer.clone()),
    );
    App {
        router: zeno_api::build_app(state.clone()),
        mailer,
        pool,
        state,
    }
}

/// An app whose real mail transport is `send`, while `record` is what tests inspect.
pub fn build_with_mailer(pool: PgPool, send: Arc<dyn Mailer>, record: MemoryMailer) -> App {
    let state = make_state(pool.clone(), false, 0, send);
    App {
        router: zeno_api::build_app(state.clone()),
        mailer: record,
        pool,
        state,
    }
}

pub fn make_state(
    pool: PgPool,
    cookie_secure: bool,
    trusted_proxy_hops: usize,
    mailer: Arc<dyn Mailer>,
) -> AppState {
    let config = ApiConfig {
        core: Config {
            env: Environment::Development,
            database_url: String::new(),
            redis_url: String::new(),
            bind_addr: "127.0.0.1:0".parse().unwrap(),
        },
        client_origin: ORIGIN.into(),
        jwt_access_secret: SECRET.into(),
        cookie_secure,
        trusted_proxy_hops,
        storage: storage_config(),
        max_upload_bytes: MAX_UPLOAD_BYTES,
    };
    let store = S3Store::new(&config.storage).unwrap();
    AppState::new(
        pool,
        config,
        Arc::new(MemoryCounters::default()),
        mailer,
        Passwords::new(PasswordParams::for_tests()).unwrap(),
        store,
    )
}

pub const MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;

pub fn storage_config() -> StorageConfig {
    let _ = dotenvy::dotenv();
    StorageConfig::from_env().expect("S3_* settings are required for API tests; see .env.example")
}

pub struct Res {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Res {
    pub fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect()
    }
}

/// Behaves like a browser cookie jar closely enough to catch scoping mistakes: it honours
/// `Path` and removal via `Max-Age=0`.
#[derive(Default, Clone)]
pub struct Jar {
    cookies: Vec<(String, String, String)>,
}

impl Jar {
    pub fn apply(&mut self, res: &Res) {
        for raw in res.set_cookies() {
            let mut parts = raw.split(';').map(str::trim);
            let (name, value) = parts.next().unwrap().split_once('=').unwrap();
            let mut path = "/".to_owned();
            let mut remove = value.is_empty();
            for attr in parts {
                let lower = attr.to_ascii_lowercase();
                if let Some(p) = lower.strip_prefix("path=") {
                    path = p.to_owned();
                }
                if lower == "max-age=0" {
                    remove = true;
                }
            }
            self.cookies.retain(|(n, _, p)| !(n == name && *p == path));
            if !remove {
                self.cookies.push((name.to_owned(), value.to_owned(), path));
            }
        }
    }

    pub fn of(res: &Res) -> Jar {
        let mut jar = Jar::default();
        jar.apply(res);
        jar
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, v, _)| v.as_str())
    }

    pub fn header_for(&self, uri: &str) -> Option<String> {
        let sent: Vec<String> = self
            .cookies
            .iter()
            .filter(|(_, _, path)| uri.starts_with(path.as_str()))
            .map(|(n, v, _)| format!("{n}={v}"))
            .collect();
        (!sent.is_empty()).then(|| sent.join("; "))
    }

    pub fn set(&mut self, name: &str, value: &str, path: &str) {
        self.cookies.retain(|(n, _, _)| n != name);
        self.cookies
            .push((name.to_owned(), value.to_owned(), path.to_owned()));
    }
}

pub struct Req {
    router: Router,
    method: Method,
    uri: String,
    body: Option<Vec<u8>>,
    content_type: Option<&'static str>,
    cookie: Option<String>,
    origin: Option<String>,
    headers: Vec<(String, String)>,
}

impl App {
    pub fn get(&self, uri: &str) -> Req {
        self.req(Method::GET, uri)
    }

    pub fn post(&self, uri: &str) -> Req {
        self.req(Method::POST, uri)
    }

    pub fn req(&self, method: Method, uri: &str) -> Req {
        let is_safe = matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);
        Req {
            router: self.router.clone(),
            method,
            uri: uri.to_owned(),
            body: None,
            content_type: None,
            cookie: None,
            origin: (!is_safe).then(|| ORIGIN.to_owned()),
            headers: vec![],
        }
    }
}

impl Req {
    pub fn json(mut self, value: Value) -> Self {
        self.body = Some(serde_json::to_vec(&value).unwrap());
        self.content_type = Some("application/json");
        self
    }

    pub fn raw_body(mut self, body: Vec<u8>, content_type: Option<&'static str>) -> Self {
        self.body = Some(body);
        self.content_type = content_type;
        self
    }

    pub fn jar(mut self, jar: &Jar) -> Self {
        self.cookie = jar.header_for(&self.uri);
        self
    }

    pub fn cookie_header(mut self, value: &str) -> Self {
        self.cookie = Some(value.to_owned());
        self
    }

    pub fn origin(mut self, origin: &str) -> Self {
        self.origin = Some(origin.to_owned());
        self
    }

    pub fn no_origin(mut self) -> Self {
        self.origin = None;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub async fn send(self) -> Res {
        let mut builder = Request::builder().method(self.method).uri(&self.uri);
        if let Some(ct) = self.content_type {
            builder = builder.header(header::CONTENT_TYPE, ct);
        }
        if let Some(cookie) = &self.cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        if let Some(origin) = &self.origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        for (k, v) in &self.headers {
            builder = builder.header(k, v);
        }
        let request = builder
            .body(Body::from(self.body.unwrap_or_default()))
            .unwrap();
        let response = self.router.oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        Res {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }
}

pub fn token_from(email: &OutgoingEmail) -> String {
    email
        .text
        .lines()
        .find(|line| line.starts_with(ORIGIN))
        .and_then(|line| line.trim().rsplit('/').next())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no link in email:\n{}", email.text))
}

pub fn register_body(email: &str) -> Value {
    json!({ "name": "Asha Rao", "email": email, "password": PASSWORD })
}

/// Registers and returns the verification token from the email that was sent.
pub async fn signup(app: &App, email: &str) -> String {
    let before = app.mailer.sent().len();
    let res = app
        .post("/v1/auth/register")
        .json(register_body(email))
        .send()
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
    let mails = app.mailer.wait_for(before + 1).await;
    token_from(&mails[before])
}

pub async fn verify(app: &App, token: &str) -> Res {
    app.post("/v1/auth/verify-email")
        .json(json!({ "token": token }))
        .send()
        .await
}

/// Registers, verifies and returns the logged-in jar.
pub async fn signup_verified(app: &App, email: &str) -> Jar {
    let token = signup(app, email).await;
    let res = verify(app, &token).await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    Jar::of(&res)
}

pub async fn login(app: &App, email: &str, password: &str) -> Res {
    app.post("/v1/auth/login")
        .json(json!({ "email": email, "password": password }))
        .send()
        .await
}

pub async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// Registers with the given role, verifies, and returns the logged-in jar.
pub async fn signup_verified_as(app: &App, email: &str, role: &str) -> Jar {
    let before = app.mailer.sent().len();
    let res = app
        .post("/v1/auth/register")
        .json(json!({ "name": "Asha Rao", "email": email, "password": PASSWORD, "role": role }))
        .send()
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{:?}", res.body);
    let mails = app.mailer.wait_for(before + 1).await;
    let verified = verify(app, &token_from(&mails[before])).await;
    assert_eq!(verified.status, StatusCode::OK, "{:?}", verified.body);
    Jar::of(&verified)
}

pub async fn signup_creator(app: &App, email: &str) -> Jar {
    signup_verified_as(app, email, "creator").await
}

/// What a browser does with a presigned part URL.
pub async fn browser_put(url: &str, body: Vec<u8>) -> StatusCode {
    let status = reqwest::Client::new()
        .put(url)
        .body(body)
        .send()
        .await
        .unwrap()
        .status();
    StatusCode::from_u16(status.as_u16()).unwrap()
}
