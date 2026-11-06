//! Runs against a real S3-compatible server (MinIO from deploy/docker-compose.yml).

use std::time::Duration;

use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeno_core::storage::{S3Store, StorageConfig, StorageError};

const MIB: usize = 1024 * 1024;

async fn store() -> S3Store {
    let _ = dotenvy::dotenv();
    let config = StorageConfig::from_env()
        .expect("S3_* settings are required for storage tests; see .env.example");
    let store = S3Store::new(&config).unwrap();
    store
        .ensure_bucket()
        .await
        .expect("storage must be reachable");
    store
}

fn unique_key(name: &str) -> String {
    format!("tests/{}/{name}", Uuid::new_v4())
}

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

/// What a browser does: PUT the raw bytes to the presigned URL.
async fn browser_put(url: url::Url, body: Vec<u8>) -> reqwest::StatusCode {
    reqwest::Client::new()
        .put(url)
        .body(body)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_multipart_upload_round_trips_through_presigned_part_urls() {
    let store = store().await;
    let key = unique_key("source");
    let upload_id = store.create_multipart(&key, "video/mp4").await.unwrap();

    let part1 = bytes(5 * MIB, 1);
    let part2 = bytes(1234, 2);
    let ttl = Duration::from_secs(60);
    assert!(browser_put(
        store.presign_part(&key, &upload_id, 1, part1.len() as u64, ttl),
        part1.clone()
    )
    .await
    .is_success());
    assert!(browser_put(
        store.presign_part(&key, &upload_id, 2, part2.len() as u64, ttl),
        part2.clone()
    )
    .await
    .is_success());

    let parts = store.list_parts(&key, &upload_id).await.unwrap();
    assert_eq!(
        parts.iter().map(|p| (p.number, p.size)).collect::<Vec<_>>(),
        vec![(1, part1.len() as u64), (2, 1234)]
    );

    let etags: Vec<String> = parts.iter().map(|p| p.etag.clone()).collect();
    store
        .complete_multipart(&key, &upload_id, &etags)
        .await
        .unwrap();

    assert_eq!(
        store.head(&key).await.unwrap(),
        Some((part1.len() + part2.len()) as u64)
    );
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("downloaded");
    let written = store
        .download_to_file(&key, &dest, 100 * MIB as u64)
        .await
        .unwrap();
    assert_eq!(written as usize, part1.len() + part2.len());
    let mut expected = part1;
    expected.extend(part2);
    assert_eq!(
        Sha256::digest(std::fs::read(&dest).unwrap()),
        Sha256::digest(&expected)
    );
}

#[tokio::test]
async fn storage_refuses_a_part_whose_size_differs_from_the_signed_length() {
    let store = store().await;
    let key = unique_key("source");
    let upload_id = store.create_multipart(&key, "video/mp4").await.unwrap();
    let declared = 5 * MIB;
    let url = store.presign_part(
        &key,
        &upload_id,
        1,
        declared as u64,
        Duration::from_secs(60),
    );

    let too_big = browser_put(url.clone(), bytes(declared + 1024, 3)).await;
    assert!(
        too_big.is_client_error(),
        "oversized part must be refused, got {too_big}"
    );
    let too_small = browser_put(url.clone(), bytes(declared - 1024, 3)).await;
    assert!(
        too_small.is_client_error(),
        "undersized part must be refused, got {too_small}"
    );

    assert!(
        browser_put(url, bytes(declared, 3)).await.is_success(),
        "the declared size works"
    );
    store.abort_multipart(&key, &upload_id).await.unwrap();
}

#[tokio::test]
async fn presigned_urls_cannot_be_reused_for_another_key_part_or_after_expiry() {
    let store = store().await;
    let key = unique_key("source");
    let upload_id = store.create_multipart(&key, "video/mp4").await.unwrap();
    let url = store.presign_part(&key, &upload_id, 1, 5 * MIB as u64, Duration::from_secs(60));

    let mut other_part = url.clone();
    other_part.set_query(Some(
        &url.query().unwrap().replace("partNumber=1", "partNumber=2"),
    ));
    assert!(browser_put(other_part, bytes(5 * MIB, 4))
        .await
        .is_client_error());

    let mut other_key = url.clone();
    other_key.set_path(&url.path().replace("source", "elsewhere"));
    assert!(browser_put(other_key, bytes(5 * MIB, 4))
        .await
        .is_client_error());

    let short_lived =
        store.presign_part(&key, &upload_id, 1, 5 * MIB as u64, Duration::from_secs(1));
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(browser_put(short_lived, bytes(5 * MIB, 4))
        .await
        .is_client_error());
    store.abort_multipart(&key, &upload_id).await.unwrap();
}

#[tokio::test]
async fn completing_with_a_missing_part_fails_and_an_aborted_upload_disappears() {
    let store = store().await;
    let key = unique_key("source");
    let upload_id = store.create_multipart(&key, "video/mp4").await.unwrap();
    let part = bytes(5 * MIB, 5);
    browser_put(
        store.presign_part(
            &key,
            &upload_id,
            1,
            part.len() as u64,
            Duration::from_secs(60),
        ),
        part,
    )
    .await;
    let etag = store.list_parts(&key, &upload_id).await.unwrap()[0]
        .etag
        .clone();

    let err = store
        .complete_multipart(
            &key,
            &upload_id,
            &[etag.clone(), "\"00000000000000000000000000000000\"".into()],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::Http { .. }), "{err:?}");
    assert_eq!(
        store.head(&key).await.unwrap(),
        None,
        "nothing is published by a failed completion"
    );

    store.abort_multipart(&key, &upload_id).await.unwrap();
    assert!(matches!(
        store.list_parts(&key, &upload_id).await,
        Err(StorageError::NotFound)
    ));
    store.abort_multipart(&key, &upload_id).await.unwrap();
}

#[tokio::test]
async fn small_objects_can_be_put_read_and_deleted_and_missing_ones_report_not_found() {
    let store = store().await;
    let key = unique_key("master.m3u8");
    assert_eq!(store.head(&key).await.unwrap(), None);
    assert!(matches!(
        store.get_bytes(&key, 1024).await,
        Err(StorageError::NotFound)
    ));

    store
        .put_bytes(&key, b"#EXTM3U\n".to_vec(), "application/vnd.apple.mpegurl")
        .await
        .unwrap();
    assert_eq!(store.get_bytes(&key, 1024).await.unwrap(), b"#EXTM3U\n");
    assert_eq!(store.head(&key).await.unwrap(), Some(8));

    store.delete(&key).await.unwrap();
    assert_eq!(store.head(&key).await.unwrap(), None);
    store.delete(&key).await.unwrap();
}

#[tokio::test]
async fn downloads_stop_at_the_size_limit() {
    let store = store().await;
    let key = unique_key("big");
    store
        .put_bytes(&key, bytes(2 * MIB, 6), "application/octet-stream")
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();

    let err = store
        .download_to_file(&key, &dir.path().join("x"), MIB as u64)
        .await
        .unwrap_err();
    assert!(matches!(err, StorageError::TooLarge { .. }), "{err:?}");
    assert!(matches!(
        store.get_bytes(&key, MIB as u64).await,
        Err(StorageError::TooLarge { .. })
    ));
    assert!(store
        .download_to_file(&key, &dir.path().join("y"), 3 * MIB as u64)
        .await
        .is_ok());
    let missing = store
        .download_to_file("tests/nope/nothing", &dir.path().join("z"), 10)
        .await;
    assert!(matches!(missing, Err(StorageError::NotFound)));
}
