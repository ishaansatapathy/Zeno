//! A video as the worker leaves it, for tests of anything that plays video: uploaded through the
//! API, then `ready`, with a master playlist, one media playlist per rendition (carrying the key
//! placeholder) and its segments in storage, and a wrapped key in the database.
#![allow(dead_code)]

use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use super::*;

pub const MIB: usize = 1024 * 1024;
pub const DURATION: i32 = 600;
pub const RENDITIONS: [&str; 2] = ["144p", "audio"];

pub fn segment_bytes(rendition: &str, n: usize) -> Vec<u8> {
    format!("encrypted segment {n} of {rendition}").into_bytes()
}

pub async fn ready_video(app: &App, jar: &Jar, pool: &PgPool) -> Uuid {
    let created = app
        .post("/v1/videos")
        .jar(jar)
        .json(json!({"filename": "Lecture.mp4", "size_bytes": MIB, "content_type": "video/mp4"}))
        .send()
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let id = Uuid::parse_str(created.body["video"]["id"].as_str().unwrap()).unwrap();
    let url = created.body["upload"]["parts"][0]["url"].as_str().unwrap();
    assert!(browser_put(url, vec![1; MIB]).await.is_success());
    let done = app
        .post(&format!("/v1/videos/{id}/complete"))
        .jar(jar)
        .send()
        .await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);

    let store = &app.state.store;
    let master = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n\
        #EXT-X-STREAM-INF:BANDWIDTH=250000,RESOLUTION=256x144,CODECS=\"avc1.4d401f,mp4a.40.2\"\n144p/index.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=70000,CODECS=\"mp4a.40.2\"\naudio/index.m3u8\n";
    store
        .put_bytes(
            &format!("videos/{id}/master.m3u8"),
            master.into(),
            "application/vnd.apple.mpegurl",
        )
        .await
        .unwrap();
    for rendition in RENDITIONS {
        let media = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n\
            #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-KEY:METHOD=AES-128,URI=\"zeno-key://video\",IV=0x00000000000000000000000000000000\n\
            #EXTINF:6.000000,\nseg_00000.ts\n#EXTINF:6.000000,\nseg_00001.ts\n#EXT-X-ENDLIST\n";
        store
            .put_bytes(
                &format!("videos/{id}/{rendition}/index.m3u8"),
                media.into(),
                "application/vnd.apple.mpegurl",
            )
            .await
            .unwrap();
        for n in 0..2 {
            store
                .put_bytes(
                    &format!("videos/{id}/{rendition}/seg_0000{n}.ts"),
                    segment_bytes(rendition, n),
                    "video/mp2t",
                )
                .await
                .unwrap();
        }
    }
    let renditions: Value = RENDITIONS
        .iter()
        .map(|name| json!({"name": name, "playlist": format!("{name}/index.m3u8")}))
        .collect();
    sqlx::query("update videos set status = 'ready', ready_at = now(), duration_seconds = $2, renditions = $3 where id = $1")
        .bind(id).bind(DURATION).bind(renditions).execute(pool).await.unwrap();
    sqlx::query("insert into video_keys (video_id, wrapped_key) values ($1, $2)")
        .bind(id)
        .bind(vec![7u8; 45])
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("update jobs set completed_at = now()")
        .execute(pool)
        .await
        .unwrap();
    id
}
