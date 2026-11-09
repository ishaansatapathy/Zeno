//! The authorization matrix: every route of the API, called by every kind of caller, with the
//! outcome each one is allowed to get.
//!
//! This is the guard against forgetting an access check. It has two halves:
//!
//! * `every_route_gives_every_caller_the_intended_outcome` calls each route as an anonymous
//!   visitor, an enrolled student, a student who is not enrolled, the owning creator, another
//!   creator and an admin, and compares the status with the table below. Every probe gets a fresh
//!   set of resources, so destructive routes (delete, revoke, abort) cannot disturb later probes.
//! * `no_route_is_missing_from_the_matrix` reads the router source, collects every `.route(...)`
//!   with its methods, and fails if one is not in the table (a new endpoint with no declared
//!   policy) or if a row of the table no longer exists.
//!
//! Adding an endpoint means adding a row here and choosing, deliberately, who may call it.
//!
//! "Hidden" (404) is how the API answers a caller who is allowed to ask but not to know: someone
//! else's course looks exactly like one that does not exist.

mod common;

use axum::http::Method;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::playback::PlaybackClaims;

macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident($pool:ident: PgPool) $body:block) => {
        #[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
        $(#[$meta])*
        async fn $name($pool: PgPool) $body
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Actor {
    Anonymous,
    /// Enrolled in the course under test.
    Student,
    /// A student who is not.
    Outsider,
    /// The creator who owns the course.
    Owner,
    OtherCreator,
    Admin,
}

const ACTORS: [Actor; 6] = [
    Actor::Anonymous,
    Actor::Student,
    Actor::Outsider,
    Actor::Owner,
    Actor::OtherCreator,
    Actor::Admin,
];

/// What a caller may get back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Out {
    /// Anything but "missing", "wrong method" or a crash: the route is reachable.
    Open,
    /// Past the access checks: not 401, 403, 404, 405 or a crash. (A malformed request body or a
    /// state conflict is fine; it still means the caller was let in.)
    Allowed,
    Unauthorized,
    Forbidden,
    Hidden,
    NotBuilt,
    /// The playback token was accepted (whatever the playlist lookup then did).
    TokenAccepted,
}

impl Out {
    fn holds(self, status: u16) -> bool {
        let reachable = status < 500 && !matches!(status, 401 | 403 | 404 | 405);
        match self {
            Out::Open => reachable,
            Out::Allowed => reachable && status != 401 && status != 403,
            Out::Unauthorized => status == 401,
            Out::Forbidden => status == 403,
            Out::Hidden => status == 404,
            Out::NotBuilt => status == 501,
            Out::TokenAccepted => status != 401 && status != 403 && status != 405,
        }
    }
}

#[test]
fn public_route_expectation_rejects_auth_gates() {
    assert!(!Out::Open.holds(401));
    assert!(!Out::Open.holds(403));
    assert!(Out::Open.holds(200));
}

/// Who may call a route, as a policy.
#[derive(Clone, Copy, Debug)]
enum Class {
    /// Anyone, signed in or not.
    Public,
    /// Public auth endpoints probed with deliberately invalid credentials: rejection is about
    /// those credentials, not an account gate on the route.
    InvalidCredential,
    /// Any signed-in account.
    Signed,
    /// Creators and admins; the route is not about one particular resource.
    Creator,
    /// A creator's own resource (a course, lesson or video): the owner and admins. Other
    /// creators see it as missing; students are turned away by role.
    Owned,
    /// Only the creator who made it. Admins cannot yet delete someone's video or finish their
    /// upload (a moderation takedown is planned for the admin console); everyone else sees
    /// nothing, and students are turned away by role first.
    OwnerOnly,
    /// Reading a video: its owner and admins; anyone else, students included, is told it is
    /// missing (the video API does not reveal which videos exist).
    VideoRead,
    /// Like `Owned`, but a creator who is not the owner is refused (403) rather than told the
    /// resource is missing, because the route names a video the caller may know exists.
    OwnedRefused,
    /// Playing a lesson: the owner, admins and enrolled students.
    LessonPlay,
    /// A playback session's own viewer.
    SessionViewer,
    /// Enrolled students only; everyone else, creators included, sees nothing.
    EnrolledOnly,
    /// The person who placed the order (in these probes, the student who is not enrolled);
    /// everyone else, the course's creator and admins included, sees nothing.
    Buyer,
    /// A server-to-server call authorised by a signature, not by who is signed in: every caller
    /// here, whatever they hold, is refused without one.
    Webhook,
    /// Not built yet: answers 501 to everyone, without identifying them.
    NotBuilt,
    /// Authorised by the playback token alone (players send no cookies).
    TokenValid,
    /// No token, or one that does not belong to the session.
    TokenRefused,
}

fn expect(class: Class, actor: Actor) -> Out {
    use Actor::*;
    match (class, actor) {
        (Class::Public, _) => Out::Open,
        (Class::InvalidCredential, _) => Out::Unauthorized,
        (Class::Signed, Anonymous) => Out::Unauthorized,
        (Class::Signed, _) => Out::Allowed,
        (Class::Creator, Anonymous) => Out::Unauthorized,
        (Class::Creator, Student | Outsider) => Out::Forbidden,
        (Class::Creator, _) => Out::Allowed,
        (Class::Owned, Anonymous) => Out::Unauthorized,
        (Class::Owned, Student | Outsider) => Out::Forbidden,
        (Class::Owned, OtherCreator) => Out::Hidden,
        (Class::Owned, Owner | Admin) => Out::Allowed,
        (Class::OwnerOnly, Anonymous) => Out::Unauthorized,
        (Class::OwnerOnly, Student | Outsider) => Out::Forbidden,
        (Class::OwnerOnly, OtherCreator | Admin) => Out::Hidden,
        (Class::OwnerOnly, Owner) => Out::Allowed,
        (Class::VideoRead, Anonymous) => Out::Unauthorized,
        (Class::VideoRead, Student | Outsider | OtherCreator) => Out::Hidden,
        (Class::VideoRead, Owner | Admin) => Out::Allowed,
        (Class::OwnedRefused, Anonymous) => Out::Unauthorized,
        (Class::OwnedRefused, Student | Outsider | OtherCreator) => Out::Forbidden,
        (Class::OwnedRefused, Owner | Admin) => Out::Allowed,
        (Class::LessonPlay, Anonymous) => Out::Unauthorized,
        (Class::LessonPlay, Student | Owner | Admin) => Out::Allowed,
        (Class::LessonPlay, Outsider | OtherCreator) => Out::Forbidden,
        (Class::SessionViewer, Anonymous) => Out::Unauthorized,
        (Class::SessionViewer, Student) => Out::Allowed,
        (Class::SessionViewer, _) => Out::Hidden,
        (Class::EnrolledOnly, Anonymous) => Out::Unauthorized,
        (Class::EnrolledOnly, Student) => Out::Allowed,
        (Class::EnrolledOnly, _) => Out::Hidden,
        (Class::Buyer, Anonymous) => Out::Unauthorized,
        (Class::Buyer, Outsider) => Out::Allowed,
        (Class::Buyer, _) => Out::Hidden,
        (Class::Webhook, _) => Out::Unauthorized,
        (Class::NotBuilt, _) => Out::NotBuilt,
        (Class::TokenValid, _) => Out::TokenAccepted,
        (Class::TokenRefused, _) => Out::Forbidden,
    }
}

#[derive(Clone, Copy)]
enum Token {
    None,
    Valid,
    Missing,
    /// Signed by the right key, but for a different session.
    OtherSession,
}

#[derive(Clone, Copy)]
enum Body {
    None,
    Json(fn(&World, Actor) -> Value),
    /// Not a decodable image, but declared as one: gets past the content-type filter so the
    /// caller's rights are what decides the outcome.
    Image,
}

struct Row {
    method: Method,
    path: &'static str,
    class: Class,
    body: Body,
    token: Token,
    needs_upload: bool,
    /// Sent without the caller's cookies. For calls that would end the shared session the other
    /// probes rely on (sign out, token rotation); what they do to a signed-in caller is covered
    /// by the auth suites.
    cookieless: bool,
}

fn row(method: Method, path: &'static str, class: Class) -> Row {
    Row {
        method,
        path,
        class,
        body: Body::None,
        token: Token::None,
        needs_upload: false,
        cookieless: false,
    }
}

impl Row {
    fn json(mut self, body: fn(&World, Actor) -> Value) -> Self {
        self.body = Body::Json(body);
        self
    }

    fn image(mut self) -> Self {
        self.body = Body::Image;
        self
    }

    fn token(mut self, token: Token) -> Self {
        self.token = token;
        self
    }

    fn upload(mut self) -> Self {
        self.needs_upload = true;
        self
    }

    fn cookieless(mut self) -> Self {
        self.cookieless = true;
        self
    }
}

fn actor_name(actor: Actor) -> String {
    format!("{actor:?}").to_lowercase()
}

/// Everything one probe may point at, created fresh for that probe.
struct World {
    course: Uuid,
    lesson: Uuid,
    video: Uuid,
    /// A real, unfinished multipart upload owned by the owner (only when a route needs one).
    upload: Uuid,
    session: Uuid,
    token: String,
    other_token: String,
    student: Uuid,
    /// An order placed by the outsider.
    order: Uuid,
    slug: String,
    handle: &'static str,
    outsider_email: String,
}

struct Actors {
    owner: Jar,
    other: Jar,
    student: Jar,
    outsider: Jar,
    admin: Jar,
    anonymous: Jar,
    owner_id: Uuid,
    student_id: Uuid,
    outsider_id: Uuid,
}

impl Actors {
    fn jar(&self, actor: Actor) -> &Jar {
        match actor {
            Actor::Anonymous => &self.anonymous,
            Actor::Student => &self.student,
            Actor::Outsider => &self.outsider,
            Actor::Owner => &self.owner,
            Actor::OtherCreator => &self.other,
            Actor::Admin => &self.admin,
        }
    }
}

async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("select id from users where email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn actors(app: &App, pool: &PgPool) -> Actors {
    let owner = signup_creator(app, "owner@matrix.in").await;
    let other = signup_creator(app, "other@matrix.in").await;
    let student = signup_verified_as(app, "student@matrix.in", "student").await;
    let outsider = signup_verified_as(app, "outsider@matrix.in", "student").await;
    // Admins cannot register themselves: promote an account, then sign in again so the new role
    // is in the session.
    signup_creator(app, "admin@matrix.in").await;
    sqlx::query("update users set role = 'admin' where email = 'admin@matrix.in'")
        .execute(pool)
        .await
        .unwrap();
    let admin = Jar::of(&login(app, "admin@matrix.in", PASSWORD).await);
    let owner_id = user_id(pool, "owner@matrix.in").await;
    sqlx::query(
        "insert into creator_profiles (user_id, handle, display_name) \
         values ($1, 'matrix-owner', 'Matrix Owner')",
    )
    .bind(owner_id)
    .execute(pool)
    .await
    .unwrap();
    Actors {
        owner,
        other,
        student,
        outsider,
        admin,
        anonymous: Jar::default(),
        owner_id,
        student_id: user_id(pool, "student@matrix.in").await,
        outsider_id: user_id(pool, "outsider@matrix.in").await,
    }
}

async fn world(app: &App, pool: &PgPool, actors: &Actors, needs_upload: bool) -> World {
    let (course, video, lesson, session) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let slug = format!("matrix-{}", &course.simple().to_string()[..12]);
    sqlx::query(
        "insert into courses (id, creator_id, title, slug, price_paise, status, published_at) \
         values ($1, $2, 'Matrix course', $3, 0, 'published', now())",
    )
    .bind(course)
    .bind(actors.owner_id)
    .bind(&slug)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into videos \
            (id, owner_id, title, original_filename, content_type, size_bytes, part_size, \
             source_key, status, duration_seconds) \
         values ($1,$2,'Lesson','lesson.mp4','video/mp4',1,5242880,$3,'ready',600)",
    )
    .bind(video)
    .bind(actors.owner_id)
    .bind(format!("uploads/{video}"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("insert into lessons (id, course_id, position, video_id, title) values ($1,$2,1,$3,'Lesson')")
        .bind(lesson)
        .bind(course)
        .bind(video)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("insert into enrollments (user_id, course_id, source) values ($1, $2, 'free')")
        .bind(actors.student_id)
        .bind(course)
        .execute(pool)
        .await
        .unwrap();
    let expires_at =
        chrono::DateTime::from_timestamp(chrono::Utc::now().timestamp() + 7200, 0).unwrap();
    sqlx::query(
        "insert into playback_sessions (id, user_id, video_id, expires_at, lesson_id) \
         values ($1, $2, $3, $4, $5)",
    )
    .bind(session)
    .bind(actors.student_id)
    .bind(video)
    .bind(expires_at)
    .bind(lesson)
    .execute(pool)
    .await
    .unwrap();
    let order = Uuid::new_v4();
    sqlx::query(
        "insert into orders (id, user_id, course_id, amount_paise, price_snapshot, status,             provider, provider_order_id, expires_at)          values ($1, $2, $3, 49900,             jsonb_build_object('v', 1, 'course_id', $3::text, 'course_title', 'Matrix course',                 'course_slug', $4::text, 'creator_id', $5::text, 'list_price_paise', 49900,                 'discount_paise', 0, 'amount_paise', 49900, 'currency', 'INR'),             'pending', 'fake', $6, now() + interval '30 minutes')",
    )
    .bind(order)
    .bind(actors.outsider_id)
    .bind(course)
    .bind(&slug)
    .bind(actors.owner_id)
    .bind(format!("order_matrix_{}", order.simple()))
    .execute(pool)
    .await
    .unwrap();
    let sign = |session_id: Uuid| {
        app.state.config.playback.sign(&PlaybackClaims {
            session_id,
            video_id: video,
            user_id: actors.student_id,
            expires_at,
        })
    };
    // The API caps unfinished uploads per account; earlier probes may have left some.
    sqlx::query("delete from videos where status = 'uploading'")
        .execute(pool)
        .await
        .unwrap();
    let upload = if needs_upload {
        let created = app
            .post("/v1/videos")
            .jar(&actors.owner)
            .json(
                json!({"filename": "m.mp4", "size_bytes": 1_048_576, "content_type": "video/mp4"}),
            )
            .send()
            .await;
        assert_eq!(created.status.as_u16(), 201, "{:?}", created.body);
        Uuid::parse_str(created.body["video"]["id"].as_str().unwrap()).unwrap()
    } else {
        Uuid::nil()
    };
    World {
        course,
        lesson,
        video,
        upload,
        session,
        token: sign(session),
        other_token: sign(Uuid::new_v4()),
        student: actors.student_id,
        order,
        slug,
        handle: "matrix-owner",
        outsider_email: "outsider@matrix.in".into(),
    }
}

fn fill(path: &str, world: &World) -> String {
    path.replace("{course}", &world.course.to_string())
        .replace("{lesson}", &world.lesson.to_string())
        .replace("{video}", &world.video.to_string())
        .replace("{upload}", &world.upload.to_string())
        .replace("{session}", &world.session.to_string())
        .replace("{student}", &world.student.to_string())
        .replace("{order}", &world.order.to_string())
        .replace("{slug}", &world.slug)
        .replace("{handle}", world.handle)
        .replace("{file}", "144p.m3u8")
}

fn rows() -> Vec<Row> {
    use Class::*;
    let (get, post, put, patch, delete) = (
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    );
    vec![
        // Platform
        row(get.clone(), "/health", Public),
        row(get.clone(), "/ready", Public),
        // Accounts: reachable by anyone; what they do with a stranger is tested in the auth suites.
        row(post.clone(), "/v1/auth/register", Public).json(|_, a| {
            json!({"name": "Matrix", "email": format!("new-{}@matrix.in", actor_name(a)), "password": PASSWORD})
        }),
        row(post.clone(), "/v1/auth/verify-email", Public).json(|_, _| json!({"token": "nope"})),
        row(post.clone(), "/v1/auth/resend-verification", Public).json(|_, _| json!({"email": "nobody@matrix.in"})),
        row(post.clone(), "/v1/auth/login", InvalidCredential).json(|_, actor| json!({"email": format!("nobody-{}@matrix.in", actor_name(actor)), "password": PASSWORD})),
        row(post.clone(), "/v1/auth/refresh", InvalidCredential).cookieless(),
        row(post.clone(), "/v1/auth/logout", Public).cookieless(),
        row(post.clone(), "/v1/auth/forgot-password", Public).json(|_, _| json!({"email": "nobody@matrix.in"})),
        row(post.clone(), "/v1/auth/reset-password", Public).json(|_, _| json!({"token": "nope", "password": PASSWORD})),
        row(get.clone(), "/v1/auth/me", Signed),
        row(get.clone(), "/v1/auth/activity", Signed),
        // Public catalogue
        row(get.clone(), "/v1/public/courses", Public),
        row(get.clone(), "/v1/public/courses/{slug}", Public),
        row(get.clone(), "/v1/public/creators/{handle}", Public),
        // Courses
        row(post.clone(), "/v1/courses", Creator).json(|_, a| json!({"title": format!("Course by {}", actor_name(a))})),
        row(get.clone(), "/v1/courses", Creator),
        row(get.clone(), "/v1/courses/{course}", Owned),
        row(patch.clone(), "/v1/courses/{course}", Owned).json(|_, _| json!({"version": 1, "title": "Renamed course"})),
        row(delete.clone(), "/v1/courses/{course}", Owned),
        row(post.clone(), "/v1/courses/{course}/publish", Owned).json(|_, _| json!({"version": 1})),
        row(post.clone(), "/v1/courses/{course}/unpublish", Owned).json(|_, _| json!({"version": 1})),
        row(put.clone(), "/v1/courses/{course}/thumbnail", Owned).image(),
        row(delete.clone(), "/v1/courses/{course}/thumbnail", Owned),
        // Lessons
        row(get.clone(), "/v1/courses/{course}/lessons", Owned),
        row(post.clone(), "/v1/courses/{course}/lessons", Owned).json(|w, _| json!({"title": "Another", "video_id": w.video})),
        row(put.clone(), "/v1/courses/{course}/lessons/order", Owned).json(|w, _| json!({"lesson_ids": [w.lesson]})),
        row(patch.clone(), "/v1/lessons/{lesson}", Owned).json(|_, _| json!({"title": "Edited"})),
        row(delete.clone(), "/v1/lessons/{lesson}", Owned),
        row(post.clone(), "/v1/lessons/{lesson}/play", LessonPlay),
        // Enrollment
        row(post.clone(), "/v1/courses/{course}/enroll", Signed),
        // Payments: anyone signed in may start checkout (what comes of it depends on the course,
        // not the person); an order is its buyer's alone.
        row(post.clone(), "/v1/checkout/{course}", Signed),
        row(get.clone(), "/v1/orders/{order}", Buyer),
        row(post.clone(), "/v1/courses/{course}/grants", Owned).json(|w, _| json!({"user": w.outsider_email})),
        row(delete.clone(), "/v1/courses/{course}/grants/{student}?reason=matrix", Owned),
        row(get.clone(), "/v1/courses/{course}/students", Owned),
        row(get.clone(), "/v1/me/courses", Signed),
        // Progress
        row(get.clone(), "/v1/courses/{course}/progress", EnrolledOnly),
        row(get.clone(), "/v1/me/continue-watching", Signed),
        // Profile
        row(get.clone(), "/v1/me/profile", Creator),
        row(put.clone(), "/v1/me/profile", Creator).json(|_, a| {
            json!({"handle": format!("matrix-{}", actor_name(a)), "display_name": "Matrix Person"})
        }),
        row(put.clone(), "/v1/me/profile/avatar", Creator).image(),
        row(delete.clone(), "/v1/me/profile/avatar", Creator),
        // Videos
        row(post.clone(), "/v1/videos", Creator)
            .json(|_, _| json!({"filename": "m.mp4", "size_bytes": 1_048_576, "content_type": "video/mp4"})),
        row(get.clone(), "/v1/videos/{video}", VideoRead),
        row(delete.clone(), "/v1/videos/{video}", OwnerOnly),
        row(get.clone(), "/v1/videos/{upload}/upload", OwnerOnly).upload(),
        row(post.clone(), "/v1/videos/{upload}/complete", OwnerOnly).upload(),
        row(post.clone(), "/v1/videos/{upload}/abort", OwnerOnly).upload(),
        row(post.clone(), "/v1/videos/{video}/play", OwnedRefused),
        // Playback
        row(delete.clone(), "/v1/play/{session}", SessionViewer),
        row(post.clone(), "/v1/play/{session}/heartbeat", SessionViewer).json(|_, _| json!({"position_seconds": 5})),
        row(get.clone(), "/v1/play/{session}/master.m3u8", TokenValid).token(Token::Valid),
        row(get.clone(), "/v1/play/{session}/master.m3u8", TokenRefused).token(Token::Missing),
        row(get.clone(), "/v1/play/{session}/master.m3u8", TokenRefused).token(Token::OtherSession),
        row(get.clone(), "/v1/play/{session}/{file}", TokenValid).token(Token::Valid),
        row(get.clone(), "/v1/play/{session}/{file}", TokenRefused).token(Token::Missing),
        // The provider's webhook: refused for everyone, whoever they are signed in as, without a valid signature
        row(post.clone(), "/v1/webhooks/razorpay", Webhook).json(|_, _| json!({"event": "payment.captured"})),
        // Planned, not built
        row(get.clone(), "/v1/analytics/courses/{course}", NotBuilt),
    ]
}

async fn probe(app: &App, actors: &Actors, world: &World, row: &Row, actor: Actor) -> u16 {
    let mut uri = fill(row.path, world);
    match row.token {
        Token::None => {}
        Token::Valid => uri = format!("{uri}?t={}", world.token),
        Token::Missing => {}
        Token::OtherSession => uri = format!("{uri}?t={}", world.other_token),
    }
    let jar = if row.cookieless {
        &actors.anonymous
    } else {
        actors.jar(actor)
    };
    let mut request = app.req(row.method.clone(), &uri).jar(jar);
    match row.body {
        Body::None => {}
        Body::Json(body) => request = request.json(body(world, actor)),
        Body::Image => request = request.raw_body(vec![0x89, b'P', b'N', b'G'], Some("image/png")),
    }
    request.send().await.status.as_u16()
}

db_test! {
    async fn every_route_gives_every_caller_the_intended_outcome(pool: PgPool) {
        let app = app(pool.clone());
        let actors = actors(&app, &pool).await;
        let mut failures = Vec::new();
        let mut probes = 0;
        for row in rows() {
            for actor in ACTORS {
                let world = world(&app, &pool, &actors, row.needs_upload).await;
                let status = probe(&app, &actors, &world, &row, actor).await;
                let want = expect(row.class, actor);
                probes += 1;
                if !want.holds(status) {
                    failures.push(format!(
                        "{:>6} {:<48} as {:<12} got {status}, wanted {want:?}",
                        row.method.as_str(),
                        row.path,
                        format!("{actor:?}"),
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {probes} probes broke the policy:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

// ---- completeness: no route without a declared policy ----

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct RouteSig {
    /// Segments with every `:name` / `{name}` replaced by `:`.
    segments: Vec<String>,
    method: String,
}

fn segments(path: &str) -> Vec<String> {
    path.split('?')
        .next()
        .unwrap()
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.starts_with(':') || s.starts_with('{') {
                ":".to_owned()
            } else {
                s.to_owned()
            }
        })
        .collect()
}

/// Every `.route("path", handlers)` in `src`, as (path literal, methods).
fn scan_routes(source: &str) -> Vec<(String, Vec<String>)> {
    let bytes: Vec<char> = source.chars().collect();
    let text: String = bytes.iter().collect();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(".route(") {
        let start = from + at + ".route(".len();
        from = start;
        let rest: Vec<char> = text[start..].chars().collect();
        let mut i = 0;
        while i < rest.len() && rest[i].is_whitespace() {
            i += 1;
        }
        if i >= rest.len() || rest[i] != '"' {
            continue;
        }
        i += 1;
        let mut path = String::new();
        while i < rest.len() && rest[i] != '"' {
            path.push(rest[i]);
            i += 1;
        }
        i += 1;
        // The handler expression runs to the closing parenthesis of `.route(`.
        let mut depth = 0i32;
        let mut ident = String::new();
        let mut methods = Vec::new();
        while i < rest.len() {
            let c = rest[i];
            if c == '(' {
                if depth == 0
                    && matches!(ident.as_str(), "get" | "post" | "put" | "patch" | "delete")
                {
                    methods.push(ident.to_uppercase());
                }
                depth += 1;
                ident.clear();
            } else if c == ')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                ident.clear();
            } else if c.is_alphanumeric() || c == '_' {
                ident.push(c);
            } else {
                // `::` in a path (`axum::routing::post(`) starts a new segment; so does anything
                // else that is not part of a name.
                ident.clear();
            }
            i += 1;
        }
        found.push((path, methods));
    }
    found
}

fn source_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_route_is_missing_from_the_matrix() {
    let mut files = Vec::new();
    source_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut scanned: Vec<(String, RouteSig)> = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (path, methods) in scan_routes(&text) {
            for method in methods {
                scanned.push((
                    path.clone(),
                    RouteSig {
                        segments: segments(&path),
                        method,
                    },
                ));
            }
        }
    }
    assert!(
        scanned.len() > 40,
        "the scanner found only {} routes; has the router moved?",
        scanned.len()
    );

    let table: Vec<RouteSig> = rows()
        .iter()
        .map(|r| RouteSig {
            segments: segments(r.path),
            method: r.method.as_str().to_owned(),
        })
        .collect();

    // A router declares a path relative to where it is mounted, so a declared path matches a table
    // row whose path ends with it. A bare "/" is a mount point: it means the collection itself.
    let is_mount =
        |segs: &[String]| matches!(segs, [v, c] if v == "v1" && (c == "courses" || c == "videos"));
    let covers = |declared: &RouteSig, row: &RouteSig| {
        declared.method == row.method
            && if declared.segments.is_empty() {
                is_mount(&row.segments)
            } else {
                row.segments.ends_with(&declared.segments)
            }
    };

    let uncovered: Vec<String> = scanned
        .iter()
        .filter(|(_, declared)| !table.iter().any(|row| covers(declared, row)))
        .map(|(path, declared)| format!("{} {path}", declared.method))
        .collect();
    assert!(
        uncovered.is_empty(),
        "routes with no row in the authorization matrix (add one, and choose who may call it):\n{}",
        uncovered.join("\n")
    );

    let stale: Vec<String> = table
        .iter()
        .filter(|row| !scanned.iter().any(|(_, declared)| covers(declared, row)))
        .map(|row| format!("{} /{}", row.method, row.segments.join("/")))
        .collect();
    assert!(
        stale.is_empty(),
        "matrix rows for routes that no longer exist:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_scanner_understands_the_shapes_the_routers_use() {
    let source = r#"
        Router::new()
            .route("/", post(create).get(list))
            .route("/:id", axum::routing::patch(update).delete(remove))
            .route(
                "/:id/thumbnail",
                put(set).delete(clear).layer(images::upload_limit()),
            )
            .route("/:id/play", axum::routing::post(crate::playback::routes::start))
            .route("/health", get(|| async { "ok" }))
    "#;
    let found = scan_routes(source);
    let flat: Vec<(String, Vec<String>)> = found;
    assert_eq!(
        flat[0],
        ("/".to_owned(), vec!["POST".to_owned(), "GET".to_owned()])
    );
    assert_eq!(
        flat[1],
        (
            "/:id".to_owned(),
            vec!["PATCH".to_owned(), "DELETE".to_owned()]
        )
    );
    assert_eq!(
        flat[2],
        (
            "/:id/thumbnail".to_owned(),
            vec!["PUT".to_owned(), "DELETE".to_owned()]
        )
    );
    assert_eq!(flat[3], ("/:id/play".to_owned(), vec!["POST".to_owned()]));
    assert_eq!(flat[4], ("/health".to_owned(), vec!["GET".to_owned()]));
}
