//! The public catalogue, creator profiles and images: what visitors can see, and what
//! creators can and cannot put there.

mod common;

use axum::http::{header, Method, StatusCode};
use common::*;
use image::{ImageFormat, RgbImage};
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

/// A PNG of pseudo-random noise: large enough (hundreds of KB) to prove the image routes accept
/// bodies the rest of the API would refuse, and not compressible to nothing.
fn noisy_png(width: u32, height: u32) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let image = RgbImage::from_fn(width, height, |_, _| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        image::Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8])
    });
    let mut out = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

async fn course_owner(pool: &PgPool, course: Uuid) -> Uuid {
    sqlx::query_scalar("select creator_id from courses where id=$1")
        .bind(course)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn ready_video(pool: &PgPool, owner: Uuid, duration: i32) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into videos \
            (id, owner_id, title, original_filename, content_type, size_bytes, part_size, \
             source_key, status, duration_seconds) \
         values ($1,$2,'Lesson','lesson.mp4','video/mp4',1,5242880,$3,'ready',$4)",
    )
    .bind(id)
    .bind(owner)
    .bind(format!("uploads/{id}"))
    .bind(duration)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn draft_course(app: &App, jar: &Jar, body: Value) -> Uuid {
    let response = app.post("/v1/courses").jar(jar).json(body).send().await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    Uuid::parse_str(response.body["course"]["id"].as_str().unwrap()).unwrap()
}

/// A published course with two lessons, the second a free preview. Returns `(id, slug)`.
async fn published(app: &App, pool: &PgPool, jar: &Jar, body: Value) -> (Uuid, String) {
    let id = draft_course(app, jar, body).await;
    let owner = course_owner(pool, id).await;
    let mut version = 1;
    for (title, preview, seconds) in [("Lesson one", false, 600), ("Free preview", true, 300)] {
        let video = ready_video(pool, owner, seconds).await;
        let response = app
            .post(&format!("/v1/courses/{id}/lessons"))
            .jar(jar)
            .json(json!({"title": title, "video_id": video, "is_free_preview": preview}))
            .send()
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
        version = response.body["course_version"].as_i64().unwrap();
    }
    let response = app
        .post(&format!("/v1/courses/{id}/publish"))
        .jar(jar)
        .json(json!({"version": version}))
        .send()
        .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    let slug = response.body["course"]["slug"].as_str().unwrap().to_owned();
    (id, slug)
}

fn course_body(title: &str) -> Value {
    json!({"title": title, "description": "A complete course", "price_paise": 49900,
           "language": "en", "level": "beginner"})
}

fn sorted_keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

fn titles(response: &Res) -> Vec<&str> {
    response.body["courses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|course| course["title"].as_str().unwrap())
        .collect()
}

async fn search(app: &App, query: &str) -> Res {
    let response = app.get(&format!("/v1/public/courses?{query}")).send().await;
    assert_eq!(
        response.status,
        StatusCode::OK,
        "{query}: {:?}",
        response.body
    );
    response
}

async fn put_profile(app: &App, jar: &Jar, handle: &str) -> Res {
    app.req(Method::PUT, "/v1/me/profile")
        .jar(jar)
        .json(json!({"handle": handle, "display_name": "Asha Rao",
                      "bio": "Physics for JEE.", "links": [
                        {"label": "YouTube", "url": "https://youtube.com/@asha"}]}))
        .send()
        .await
}

async fn upload(
    app: &App,
    jar: &Jar,
    uri: &str,
    bytes: Vec<u8>,
    content_type: &'static str,
) -> Res {
    app.req(Method::PUT, uri)
        .jar(jar)
        .raw_body(bytes, Some(content_type))
        .send()
        .await
}

/// What a browser does with `thumbnail_url` / `avatar_url`.
async fn fetch(url: &str) -> (u16, Vec<u8>) {
    let response = reqwest::get(url).await.unwrap();
    let status = response.status().as_u16();
    (status, response.bytes().await.unwrap().to_vec())
}

db_test! {
    async fn a_course_page_shows_only_what_a_visitor_may_see(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_creator(&app, "page@x.in").await;
        let (id, slug) = published(&app, &pool, &creator, course_body("Physics for JEE")).await;

        let page = app.get(&format!("/v1/public/courses/{slug}")).send().await;
        assert_eq!(page.status, StatusCode::OK, "{:?}", page.body);
        let course = &page.body["course"];
        assert_eq!(
            sorted_keys(course),
            ["creator", "currency", "description", "language", "lesson_count", "lessons",
             "level", "price_paise", "published_at", "short_description", "slug",
             "thumbnail_url", "title", "total_duration_seconds"],
            "an exact whitelist: no ids, versions, status or storage keys"
        );
        assert_eq!(course["price_paise"], 49900);
        assert_eq!((course["lesson_count"].as_i64(), course["total_duration_seconds"].as_i64()),
                   (Some(2), Some(900)));
        assert_eq!(sorted_keys(&course["creator"]), ["avatar_url", "display_name", "handle"]);
        assert_eq!(course["creator"]["display_name"], "Asha Rao", "falls back to the account name");
        assert!(course["creator"]["handle"].is_null());

        let lessons = course["lessons"].as_array().unwrap();
        assert_eq!(lessons.len(), 2);
        assert!(lessons[0].get("lesson_id").is_none(), "a paid lesson has no id to try");
        assert!(lessons[1]["lesson_id"].is_string(), "a free preview can be played");
        for lesson in lessons {
            assert!(lesson.get("video_id").is_none() && lesson.get("description").is_none());
        }
        assert!(!page.body.to_string().contains(&id.to_string()), "the course id never leaks");

        assert_eq!(page.headers[header::CACHE_CONTROL], "public, max-age=60, stale-while-revalidate=300");
        let etag = page.headers[header::ETAG].to_str().unwrap().to_owned();
        let again = app.get(&format!("/v1/public/courses/{slug}"))
            .header("if-none-match", &etag).send().await;
        assert_eq!(again.status, StatusCode::NOT_MODIFIED);
        assert!(again.body.is_null());
        let changed = app.get(&format!("/v1/public/courses/{slug}"))
            .header("if-none-match", "W/\"0000\"").send().await;
        assert_eq!(changed.status, StatusCode::OK);
    }
}

db_test! {
    async fn drafts_unpublished_deleted_and_nonsense_slugs_do_not_exist(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_creator(&app, "hidden@x.in").await;
        let draft = draft_course(&app, &creator, course_body("Only a draft")).await;
        let draft_slug: String = sqlx::query_scalar("select slug from courses where id=$1")
            .bind(draft).fetch_one(&pool).await.unwrap();
        let (live, live_slug) = published(&app, &pool, &creator, course_body("Soon hidden")).await;

        let url = |slug: &str| format!("/v1/public/courses/{slug}");
        assert_eq!(app.get(&url(&draft_slug)).send().await.status, StatusCode::NOT_FOUND);
        assert_eq!(app.get(&url(&live_slug)).send().await.status, StatusCode::OK);

        let version: i32 = sqlx::query_scalar("select version from courses where id=$1")
            .bind(live).fetch_one(&pool).await.unwrap();
        let unpublished = app.post(&format!("/v1/courses/{live}/unpublish"))
            .jar(&creator).json(json!({"version": version})).send().await;
        assert_eq!(unpublished.status, StatusCode::OK);
        assert_eq!(app.get(&url(&live_slug)).send().await.status, StatusCode::NOT_FOUND);
        assert!(titles(&search(&app, "").await).is_empty(), "and not in the listing either");

        let (gone, gone_slug) = published(&app, &pool, &creator, course_body("Deleted soon")).await;
        assert_eq!(app.get(&url(&gone_slug)).send().await.status, StatusCode::OK);
        let deleted = app.req(Method::DELETE, &format!("/v1/courses/{gone}"))
            .jar(&creator).send().await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        assert_eq!(app.get(&url(&gone_slug)).send().await.status, StatusCode::NOT_FOUND);

        for nonsense in ["Upper-Case", "a", "../etc/passwd", "x%00y", "a%20b", &"z".repeat(300)] {
            let response = app.get(&format!("/v1/public/courses/{nonsense}")).send().await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{nonsense}");
        }
        // The 404 body is the same for "never existed" and "exists but hidden".
        let a = app.get(&url(&draft_slug)).send().await;
        let b = app.get(&url("never-existed-anywhere")).send().await;
        assert_eq!(a.body, b.body);
    }
}

db_test! {
    async fn profiles_have_unique_permanent_handles_and_safe_links(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_creator(&app, "asha@x.in").await;
        let ravi = signup_creator(&app, "ravi@x.in").await;
        let student = signup_verified_as(&app, "student@x.in", "student").await;

        assert!(app.get("/v1/me/profile").jar(&asha).send().await.body["profile"].is_null());
        let made = put_profile(&app, &asha, "Asha-Physics").await;
        assert_eq!(made.status, StatusCode::OK, "{:?}", made.body);
        assert_eq!(made.body["profile"]["handle"], "asha-physics", "lower-cased");
        assert_eq!(made.body["profile"]["links"][0]["url"], "https://youtube.com/@asha");
        assert!(made.body["profile"]["avatar_url"].is_null());

        let taken = put_profile(&app, &ravi, "asha-physics").await;
        assert_eq!((taken.status, taken.code()), (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"));
        let relocked = put_profile(&app, &asha, "another-handle").await;
        assert_eq!((relocked.status, relocked.code()), (StatusCode::CONFLICT, "handle_locked"));

        for bad in ["admin", "ab", "-x-", "a--b", "Has Space", "आशा"] {
            let response = put_profile(&app, &ravi, bad).await;
            assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        }
        let hostile = app.req(Method::PUT, "/v1/me/profile").jar(&ravi)
            .json(json!({"handle": "ravi-chem", "display_name": "Ravi",
                         "links": [{"label": "x", "url": "javascript:alert(1)"}]}))
            .send().await;
        assert_eq!(hostile.status, StatusCode::UNPROCESSABLE_ENTITY);
        let extra = app.req(Method::PUT, "/v1/me/profile").jar(&ravi)
            .json(json!({"handle": "ravi-chem", "display_name": "Ravi", "role": "admin"}))
            .send().await;
        assert_eq!(extra.status, StatusCode::UNPROCESSABLE_ENTITY, "unknown fields are refused");

        // Editing keeps the handle and changes the rest.
        let edited = app.req(Method::PUT, "/v1/me/profile").jar(&asha)
            .json(json!({"handle": "asha-physics", "display_name": "Asha R.", "bio": "", "links": []}))
            .send().await;
        assert_eq!(edited.body["profile"]["display_name"], "Asha R.");

        for who in [&student, &Jar::default()] {
            let response = app.get("/v1/me/profile").jar(who).send().await;
            assert!(matches!(response.status, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED));
        }
    }
}

db_test! {
    async fn avatars_are_validated_re_encoded_stored_by_content_and_replaced(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_creator(&app, "avatar@x.in").await;
        let uri = "/v1/me/profile/avatar";

        let early = upload(&app, &asha, uri, noisy_png(600, 600), "image/png").await;
        assert_eq!((early.status, early.code()), (StatusCode::CONFLICT, "profile_required"));
        put_profile(&app, &asha, "avatar-asha").await;

        // A body far over the API's usual 64 KiB limit is fine on this route...
        let png = noisy_png(900, 700);
        assert!(png.len() > 128 * 1024, "{}", png.len());
        let set = upload(&app, &asha, uri, png, "image/png").await;
        assert_eq!(set.status, StatusCode::OK, "{:?}", set.body);
        let url = set.body["profile"]["avatar_url"].as_str().unwrap().to_owned();
        let (status, stored) = fetch(&url).await;
        assert_eq!(status, 200);
        assert_eq!(&stored[..3], [0xFF, 0xD8, 0xFF], "stored as a fresh JPEG, not as sent");
        let decoded = image::load_from_memory_with_format(&stored, ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (512, 512));
        let first_key: String = sqlx::query_scalar("select avatar_key from creator_profiles")
            .fetch_one(&pool).await.unwrap();
        assert!(first_key.starts_with("images/avatars/") && first_key.ends_with(".jpg"));

        // ...a different picture replaces it and the old object is deleted.
        let other = upload(&app, &asha, uri, noisy_png(700, 700), "image/png").await;
        assert_eq!(other.status, StatusCode::OK);
        let second_key: String = sqlx::query_scalar("select avatar_key from creator_profiles")
            .fetch_one(&pool).await.unwrap();
        assert_ne!(first_key, second_key);
        assert_eq!(app.state.store.head(&first_key).await.unwrap(), None, "old image removed");
        assert!(app.state.store.head(&second_key).await.unwrap().is_some());

        // Uploading the same picture again keeps it.
        let same = upload(&app, &asha, uri, noisy_png(700, 700), "image/png").await;
        assert_eq!(same.status, StatusCode::OK);
        assert!(app.state.store.head(&second_key).await.unwrap().is_some());

        let cleared = app.req(Method::DELETE, uri).jar(&asha).send().await;
        assert_eq!(cleared.status, StatusCode::OK);
        assert!(cleared.body["profile"]["avatar_url"].is_null());
        assert_eq!(app.state.store.head(&second_key).await.unwrap(), None);
    }
}

db_test! {
    async fn hostile_uploads_are_refused_and_leave_nothing_behind(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_creator(&app, "hostile@x.in").await;
        put_profile(&app, &asha, "hostile-asha").await;
        let uri = "/v1/me/profile/avatar";

        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        let cases: Vec<(&str, Vec<u8>, &'static str, StatusCode, &str)> = vec![
            ("svg as svg", svg.to_vec(), "image/svg+xml", StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_image"),
            ("svg claiming png", svg.to_vec(), "image/png", StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_image"),
            ("exe claiming jpeg", b"MZ\x90\x00 not an image".to_vec(), "image/jpeg", StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_image"),
            ("gif", b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec(), "image/png", StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_image"),
            ("empty", Vec::new(), "image/png", StatusCode::UNPROCESSABLE_ENTITY, "invalid_image"),
            ("truncated header", noisy_png(300, 300)[..30].to_vec(), "image/png", StatusCode::UNPROCESSABLE_ENTITY, "invalid_image"),
            ("too small", noisy_png(64, 64), "image/png", StatusCode::UNPROCESSABLE_ENTITY, "image_too_small"),
            ("too many pixels", noisy_png(4200, 40), "image/png", StatusCode::UNPROCESSABLE_ENTITY, "invalid_image"),
            ("too many bytes", vec![0u8; 5 * 1024 * 1024 + 1], "image/png", StatusCode::PAYLOAD_TOO_LARGE, "image_too_large"),
        ];
        for (name, bytes, content_type, status, code) in cases {
            let response = upload(&app, &asha, uri, bytes, content_type).await;
            assert_eq!((response.status, response.code()), (status, code), "{name}: {:?}", response.body);
        }
        let none = app.req(Method::PUT, uri).jar(&asha).raw_body(noisy_png(300, 300), None).send().await;
        assert_eq!(none.status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "no content type");

        let avatar: Option<String> = sqlx::query_scalar("select avatar_key from creator_profiles")
            .fetch_one(&pool).await.unwrap();
        assert!(avatar.is_none());
        let owner: Uuid = sqlx::query_scalar("select user_id from creator_profiles")
            .fetch_one(&pool).await.unwrap();
        let stored = app.state.store.list_prefix(&format!("images/avatars/{owner}/")).await.unwrap();
        assert!(stored.is_empty(), "nothing stored for rejected uploads: {stored:?}");

        let student = signup_verified_as(&app, "stud@x.in", "student").await;
        let denied = upload(&app, &student, uri, noisy_png(300, 300), "image/png").await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN);
        let anonymous = app.req(Method::PUT, uri).raw_body(noisy_png(300, 300), Some("image/png")).send().await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
        let cross_site = app.req(Method::PUT, uri).jar(&asha).origin("https://evil.example")
            .raw_body(noisy_png(300, 300), Some("image/png")).send().await;
        assert_eq!(cross_site.status, StatusCode::FORBIDDEN);
    }
}

db_test! {
    async fn course_thumbnails_are_owned_versioned_and_public_on_the_course_page(pool: PgPool) {
        let app = app(pool.clone());
        let owner = signup_creator(&app, "thumb@x.in").await;
        let other = signup_creator(&app, "thumb-other@x.in").await;
        let (id, slug) = published(&app, &pool, &owner, course_body("Thumbnail course")).await;
        let uri = format!("/v1/courses/{id}/thumbnail");
        // The bucket is shared with tests running in parallel, so look only at this course.
        let mine = format!("images/courses/{id}/");
        let version_before: i64 = app.get(&format!("/v1/courses/{id}")).jar(&owner).send().await
            .body["course"]["version"].as_i64().unwrap();

        let foreign = upload(&app, &other, &uri, noisy_png(1280, 720), "image/png").await;
        assert_eq!(foreign.status, StatusCode::NOT_FOUND);
        assert!(app.state.store.list_prefix(&mine).await.unwrap().is_empty(),
                "a non-owner's upload is rejected before anything is stored");
        let stale = app.req(Method::PUT, &uri).jar(&owner).header("if-match", "\"999\"")
            .raw_body(noisy_png(1280, 720), Some("image/png")).send().await;
        assert_eq!((stale.status, stale.code()), (StatusCode::CONFLICT, "version_conflict"));
        let left = app.state.store.list_prefix(&mine).await.unwrap();
        assert!(left.is_empty(), "and a stale upload leaves no orphan: {left:?}");

        let set = upload(&app, &owner, &uri, noisy_png(1400, 800), "image/png").await;
        assert_eq!(set.status, StatusCode::OK, "{:?}", set.body);
        assert_eq!(set.body["course"]["version"].as_i64().unwrap(), version_before + 1);
        let owner_url = set.body["course"]["thumbnail_url"].as_str().unwrap();
        let (status, bytes) = fetch(owner_url).await;
        assert_eq!(status, 200);
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1280, 720));

        let page = app.get(&format!("/v1/public/courses/{slug}")).send().await;
        assert!(page.body["course"]["thumbnail_url"].is_string());
        let listed = search(&app, "").await;
        assert!(listed.body["courses"][0]["thumbnail_url"].is_string());

        let cleared = app.req(Method::DELETE, &uri).jar(&owner).send().await;
        assert_eq!(cleared.status, StatusCode::OK);
        assert!(cleared.body["course"]["thumbnail_url"].is_null());
        assert!(app.state.store.list_prefix(&mine).await.unwrap().is_empty());
    }
}

db_test! {
    async fn search_finds_words_typos_and_hindi_and_filters_precisely(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_creator(&app, "search@x.in").await;
        let make = |title: &'static str, extra: Value| {
            let mut body = course_body(title);
            for (key, value) in extra.as_object().unwrap() {
                body[key] = value.clone();
            }
            body
        };
        published(&app, &pool, &creator, make("Python for Beginners", json!({"price_paise": 0, "language": "en"}))).await;
        published(&app, &pool, &creator, make("Physics for JEE", json!({"price_paise": 99900, "level": "advanced"}))).await;
        published(&app, &pool, &creator, make("गणित की पूरी तैयारी", json!({"language": "hi", "price_paise": 29900}))).await;
        published(&app, &pool, &creator, make("Cooking", json!({
            "description": "Learn python-style indentation in your recipes", "price_paise": 19900}))).await;

        assert_eq!(titles(&search(&app, "q=python").await)[0], "Python for Beginners", "title beats description");
        assert_eq!(titles(&search(&app, "q=python").await).len(), 2, "description matches count too");
        assert_eq!(titles(&search(&app, "q=PHYSICS").await), ["Physics for JEE"]);
        assert_eq!(titles(&search(&app, "q=pythn").await)[0], "Python for Beginners", "one letter off");
        assert_eq!(titles(&search(&app, "q=%E0%A4%97%E0%A4%A3%E0%A4%BF%E0%A4%A4").await), ["गणित की पूरी तैयारी"]);
        assert!(titles(&search(&app, "q=chemistry").await).is_empty());
        assert!(titles(&search(&app, "q=py").await).is_empty(), "too short to guess at");

        assert_eq!(titles(&search(&app, "free=true").await), ["Python for Beginners"]);
        assert_eq!(titles(&search(&app, "language=hi").await), ["गणित की पूरी तैयारी"]);
        assert_eq!(titles(&search(&app, "level=advanced").await), ["Physics for JEE"]);
        assert_eq!(titles(&search(&app, "min_price=25000&max_price=60000&sort=price_asc").await),
                   ["गणित की पूरी तैयारी", ]);
        assert_eq!(titles(&search(&app, "sort=price_desc").await),
                   ["Physics for JEE", "गणित की पूरी तैयारी", "Cooking", "Python for Beginners"]);
        assert_eq!(titles(&search(&app, "sort=price_asc").await)[0], "Python for Beginners");
        assert_eq!(titles(&search(&app, "").await)[0], "Cooking", "newest first by default");

        let first = search(&app, "limit=3&page=1").await;
        let second = search(&app, "limit=3&page=2").await;
        assert_eq!((titles(&first).len(), first.body["has_more"].clone()), (3, json!(true)));
        assert_eq!((titles(&second).len(), second.body["has_more"].clone()), (1, json!(false)));
        let all: Vec<_> = titles(&first).into_iter().chain(titles(&second)).collect();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 4, "pages neither repeat nor skip: {all:?}");
    }
}

db_test! {
    async fn search_survives_hostile_input_and_rejects_bad_parameters(pool: PgPool) {
        let app = app(pool.clone());
        let creator = signup_creator(&app, "hostile-search@x.in").await;
        published(&app, &pool, &creator, course_body("Safe course")).await;

        for nasty in [
            "%27%3B%20drop%20table%20courses%3B%20--", "%27%20OR%201%3D1--", "a%26b", "!!!", "(((", "%7C%7C%7C",
            "%22unterminated", "-", ":*", "%00", "%5C", "%25%25%25", "%F0%9F%8E%93",
        ] {
            let response = app.get(&format!("/v1/public/courses?q={nasty}")).send().await;
            assert!(
                matches!(response.status, StatusCode::OK | StatusCode::UNPROCESSABLE_ENTITY),
                "{nasty}: {:?}", response.status
            );
        }
        assert_eq!(titles(&search(&app, "q=safe").await), ["Safe course"], "and the table is intact");

        for bad in [
            "page=0", "page=51", "page=x", "limit=0", "limit=25", "limit=-1", "free=maybe",
            "min_price=abc", "min_price=-5", "min_price=900&max_price=100", "level=expert",
            "language=%20", "sort=random", "sort=relevance", "q=zz%00zz",
        ] {
            let response = app.get(&format!("/v1/public/courses?{bad}")).send().await;
            assert!(
                matches!(response.status, StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY),
                "{bad}: {:?}", response.status
            );
            assert!(response.body["error"]["code"].is_string(), "{bad}: uniform error body");
        }
        let long = "x".repeat(101);
        assert_eq!(app.get(&format!("/v1/public/courses?q={long}")).send().await.status,
                   StatusCode::UNPROCESSABLE_ENTITY);
        // Unknown parameters (campaign tags on shared links) are ignored, not errors.
        assert_eq!(app.get("/v1/public/courses?utm_source=whatsapp").send().await.status, StatusCode::OK);
    }
}

db_test! {
    async fn creator_pages_list_only_published_courses_and_match_handles_loosely(pool: PgPool) {
        let app = app(pool.clone());
        let asha = signup_creator(&app, "creator-page@x.in").await;
        let ravi = signup_creator(&app, "creator-page2@x.in").await;
        put_profile(&app, &asha, "asha-teaches").await;
        published(&app, &pool, &asha, course_body("Asha course")).await;
        published(&app, &pool, &ravi, course_body("Ravi course")).await;
        draft_course(&app, &asha, course_body("Asha draft")).await;

        let page = app.get("/v1/public/creators/Asha-Teaches").send().await;
        assert_eq!(page.status, StatusCode::OK, "{:?}", page.body);
        assert_eq!(sorted_keys(&page.body["creator"]),
                   ["avatar_url", "bio", "display_name", "handle", "links", "updated_at"]);
        assert_eq!(page.body["creator"]["handle"], "asha-teaches");
        assert_eq!(titles(&page), ["Asha course"], "no drafts, nobody else's courses");
        assert_eq!(page.body["courses"][0]["creator"]["handle"], "asha-teaches");
        assert!(page.headers.contains_key(header::ETAG));

        for missing in ["nobody-here", "admin", "x", "has%20space", "../x"] {
            let response = app.get(&format!("/v1/public/creators/{missing}")).send().await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{missing}");
        }
        let in_listing = search(&app, "").await;
        let ravis = in_listing.body["courses"].as_array().unwrap().iter()
            .find(|c| c["title"] == "Ravi course").unwrap();
        assert!(ravis["creator"]["handle"].is_null(), "no profile, no handle");
    }
}

db_test! {
    async fn browsers_may_send_put_across_origins(pool: PgPool) {
        // Reordering lessons and saving a profile are PUTs; without this the browser blocks them.
        let app = app(pool);
        let response = app.req(Method::OPTIONS, "/v1/me/profile")
            .origin(ORIGIN)
            .header("access-control-request-method", "PUT")
            .send().await;
        let allowed = response.headers["access-control-allow-methods"].to_str().unwrap().to_owned();
        assert!(allowed.split(',').any(|m| m.trim() == "PUT"), "{allowed}");
    }
}
