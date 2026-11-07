//! S3-compatible object storage (AWS S3, Cloudflare R2, MinIO). Requests are authorised by
//! SigV4 presigned URLs built locally with `rusty-s3`; `reqwest` performs the calls.
//!
//! Browsers upload straight to storage through presigned part URLs, so video bytes never pass
//! through the API. The API keeps the control plane (start, list, complete, abort) and the
//! worker downloads sources and uploads encrypted output.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Method, StatusCode};
use rusty_s3::actions::{
    AbortMultipartUpload, CompleteMultipartUpload, CreateBucket, CreateMultipartUpload,
    DeleteObject, GetObject, HeadObject, ListObjectsV2, ListParts, PutObject, S3Action, UploadPart,
};
use rusty_s3::{Bucket, Credentials, UrlStyle};
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::config::{parse_or, required, ConfigError};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const SIGNED_URL_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_ERROR_SNIPPET: usize = 400;
/// S3 returns at most 1000 parts per ListParts call.
const LIST_PARTS_PAGE: u16 = 1000;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("object not found")]
    NotFound,
    #[error("storage returned {status}: {snippet}")]
    Http { status: u16, snippet: String },
    #[error("storage request failed: {0}")]
    Transport(String),
    #[error("storage response was not understood: {0}")]
    Protocol(String),
    #[error("object is larger than the {limit} byte limit")]
    TooLarge { limit: u64 },
    #[error("local file error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<reqwest::Error> for StorageError {
    fn from(err: reqwest::Error) -> Self {
        StorageError::Transport(err.without_url().to_string())
    }
}

#[derive(Clone)]
pub struct StorageConfig {
    pub endpoint: Url,
    /// Endpoint embedded in presigned URLs handed to browsers. Often differs from `endpoint`
    /// in production (a CDN or public hostname versus an internal address).
    pub public_endpoint: Url,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub path_style: bool,
}

impl StorageConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let endpoint = parse_url("S3_ENDPOINT", &required("S3_ENDPOINT")?)?;
        let public_endpoint = match std::env::var("S3_PUBLIC_ENDPOINT") {
            Ok(value) if !value.trim().is_empty() => parse_url("S3_PUBLIC_ENDPOINT", &value)?,
            _ => endpoint.clone(),
        };
        Ok(Self {
            endpoint,
            public_endpoint,
            region: parse_or("S3_REGION", "us-east-1".to_owned())?,
            bucket: required("S3_BUCKET")?,
            access_key: required("S3_ACCESS_KEY")?,
            secret_key: required("S3_SECRET_KEY")?,
            path_style: parse_or("S3_PATH_STYLE", true)?,
        })
    }
}

fn parse_url(name: &'static str, raw: &str) -> Result<Url, ConfigError> {
    Url::parse(raw.trim()).map_err(|e| ConfigError::Invalid {
        name,
        reason: e.to_string(),
    })
}

impl fmt::Debug for StorageConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageConfig")
            .field("endpoint", &self.endpoint.as_str())
            .field("bucket", &self.bucket)
            .field("access_key", &"<redacted>")
            .field("secret_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartInfo {
    pub number: u16,
    pub size: u64,
    pub etag: String,
}

#[derive(Clone)]
pub struct S3Store {
    internal: Bucket,
    public: Bucket,
    credentials: Credentials,
    http: reqwest::Client,
}

impl fmt::Debug for S3Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Store")
            .field("bucket", &self.internal.name())
            .finish_non_exhaustive()
    }
}

impl S3Store {
    pub fn new(config: &StorageConfig) -> Result<Self, StorageError> {
        // reqwest is built without a bundled TLS provider so the C toolchain isn't needed;
        // installing ring is idempotent, and an error just means one is already installed.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let style = if config.path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let bucket = |endpoint: &Url| {
            Bucket::new(
                endpoint.clone(),
                style,
                config.bucket.clone(),
                config.region.clone(),
            )
            .map_err(|e| StorageError::Protocol(format!("invalid S3 endpoint: {e}")))
        };
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            internal: bucket(&config.endpoint)?,
            public: bucket(&config.public_endpoint)?,
            credentials: Credentials::new(config.access_key.clone(), config.secret_key.clone()),
            http,
        })
    }

    /// Creates the bucket if it does not exist yet. For development and tests; production
    /// buckets are provisioned by infrastructure code.
    pub async fn ensure_bucket(&self) -> Result<(), StorageError> {
        let url = CreateBucket::new(&self.internal, &self.credentials).sign(SIGNED_URL_TTL);
        let response = self.http.put(url).timeout(CONTROL_TIMEOUT).send().await?;
        match response.status() {
            status if status.is_success() => Ok(()),
            // Already there (S3 says 409, some implementations 200).
            StatusCode::CONFLICT => Ok(()),
            _ => Err(error_from(response).await),
        }
    }

    // ---- multipart upload ------------------------------------------------------------

    pub async fn create_multipart(
        &self,
        key: &str,
        content_type: &str,
    ) -> Result<String, StorageError> {
        let mut action = CreateMultipartUpload::new(&self.internal, Some(&self.credentials), key);
        action
            .headers_mut()
            .insert("content-type", content_type.to_owned());
        let response = self
            .http
            .post(action.sign(SIGNED_URL_TTL))
            .header("content-type", content_type)
            .timeout(CONTROL_TIMEOUT)
            .send()
            .await?;
        let body = ok_body(response).await?;
        let parsed = CreateMultipartUpload::parse_response(&body)
            .map_err(|e| StorageError::Protocol(e.to_string()))?;
        Ok(parsed.upload_id().to_owned())
    }

    /// A URL a browser can PUT one part to. The part's exact byte length is part of the
    /// signature, so storage refuses a body of any other size: a client cannot upload more
    /// than the size it declared.
    /// A URL a player can GET `key` from without credentials, valid for `ttl` (S3 allows at most
    /// seven days). Built on the public endpoint, like upload URLs.
    pub fn presign_get(&self, key: &str, ttl: Duration) -> Url {
        GetObject::new(&self.public, Some(&self.credentials), key).sign(ttl)
    }

    pub fn presign_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: u16,
        content_length: u64,
        ttl: Duration,
    ) -> Url {
        let mut action = UploadPart::new(
            &self.public,
            Some(&self.credentials),
            key,
            part_number,
            upload_id,
        );
        action
            .headers_mut()
            .insert("content-length", content_length.to_string());
        action.sign(ttl)
    }

    /// Parts storage actually holds, in part-number order. The server trusts this, never
    /// what the client says it uploaded.
    pub async fn list_parts(
        &self,
        key: &str,
        upload_id: &str,
    ) -> Result<Vec<PartInfo>, StorageError> {
        let mut parts = Vec::new();
        let mut marker: Option<u16> = None;
        loop {
            let mut action =
                ListParts::new(&self.internal, Some(&self.credentials), key, upload_id);
            action.set_max_parts(LIST_PARTS_PAGE);
            if let Some(marker) = marker {
                action.set_part_number_marker(marker);
            }
            let response = self
                .http
                .get(action.sign(SIGNED_URL_TTL))
                .timeout(CONTROL_TIMEOUT)
                .send()
                .await?;
            if response.status() == StatusCode::NOT_FOUND {
                return Err(StorageError::NotFound);
            }
            let body = ok_body(response).await?;
            let page = ListParts::parse_response(&body)
                .map_err(|e| StorageError::Protocol(e.to_string()))?;
            parts.extend(page.parts.into_iter().map(|p| PartInfo {
                number: p.number,
                size: p.size,
                etag: p.etag,
            }));
            match page.next_part_number_marker {
                Some(next) => marker = Some(next),
                None => break,
            }
        }
        parts.sort_by_key(|p| p.number);
        Ok(parts)
    }

    pub async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        etags: &[String],
    ) -> Result<(), StorageError> {
        let action = CompleteMultipartUpload::new(
            &self.internal,
            Some(&self.credentials),
            key,
            upload_id,
            etags.iter().map(String::as_str),
        );
        let url = action.clone().sign(SIGNED_URL_TTL);
        let response = self
            .http
            .post(url)
            .body(action.body())
            .timeout(Duration::from_secs(120))
            .send()
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(StorageError::NotFound);
        }
        let body = ok_body(response).await?;
        // S3 can answer 200 and still report failure in the body.
        if body.contains("<Error>") {
            return Err(StorageError::Http {
                status: 200,
                snippet: snippet(&body),
            });
        }
        Ok(())
    }

    pub async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), StorageError> {
        let action =
            AbortMultipartUpload::new(&self.internal, Some(&self.credentials), key, upload_id);
        let response = self
            .http
            .delete(action.sign(SIGNED_URL_TTL))
            .timeout(CONTROL_TIMEOUT)
            .send()
            .await?;
        match response.status() {
            status if status.is_success() => Ok(()),
            StatusCode::NOT_FOUND => Ok(()),
            _ => Err(error_from(response).await),
        }
    }

    // ---- objects ---------------------------------------------------------------------

    /// Size of an object, or `None` if it does not exist.
    pub async fn head(&self, key: &str) -> Result<Option<u64>, StorageError> {
        let action = HeadObject::new(&self.internal, Some(&self.credentials), key);
        let response = self
            .http
            .request(Method::HEAD, action.sign(SIGNED_URL_TTL))
            .timeout(CONTROL_TIMEOUT)
            .send()
            .await?;
        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if status.is_success() => {
                // reqwest reports a HEAD response's (empty) body, not the Content-Length header.
                let length = response
                    .headers()
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok());
                length.map(Some).ok_or_else(|| {
                    StorageError::Protocol("HEAD response had no Content-Length".into())
                })
            }
            _ => Err(error_from(response).await),
        }
    }

    /// Streams an object to a local file, aborting if it exceeds `max_bytes`.
    pub async fn download_to_file(
        &self,
        key: &str,
        dest: &Path,
        max_bytes: u64,
    ) -> Result<u64, StorageError> {
        let action = GetObject::new(&self.internal, Some(&self.credentials), key);
        let response = self.http.get(action.sign(SIGNED_URL_TTL)).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(StorageError::NotFound);
        }
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }
        if response.content_length().is_some_and(|len| len > max_bytes) {
            return Err(StorageError::TooLarge { limit: max_bytes });
        }
        let mut file = tokio::fs::File::create(dest).await?;
        let mut stream = response.bytes_stream();
        let mut written: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            written += chunk.len() as u64;
            if written > max_bytes {
                return Err(StorageError::TooLarge { limit: max_bytes });
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(written)
    }

    pub async fn get_bytes(&self, key: &str, max_bytes: u64) -> Result<Vec<u8>, StorageError> {
        let action = GetObject::new(&self.internal, Some(&self.credentials), key);
        let response = self.http.get(action.sign(SIGNED_URL_TTL)).send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(StorageError::NotFound);
        }
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }
        let mut out = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk?);
            if out.len() as u64 > max_bytes {
                return Err(StorageError::TooLarge { limit: max_bytes });
            }
        }
        Ok(out)
    }

    /// Single-request upload, for objects that fit comfortably in memory (playlists, HLS
    /// segments). Large source files go through multipart instead.
    pub async fn put_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        let mut action = PutObject::new(&self.internal, Some(&self.credentials), key);
        action
            .headers_mut()
            .insert("content-type", content_type.to_owned());
        let response = self
            .http
            .put(action.sign(SIGNED_URL_TTL))
            .header("content-type", content_type)
            .body(bytes)
            .timeout(Duration::from_secs(120))
            .send()
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(error_from(response).await)
        }
    }

    /// Every key under `prefix`, following pagination.
    pub async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut action = ListObjectsV2::new(&self.internal, Some(&self.credentials));
            action.with_prefix(prefix);
            if let Some(token) = &token {
                action.with_continuation_token(token);
            }
            let response = self
                .http
                .get(action.sign(SIGNED_URL_TTL))
                .timeout(CONTROL_TIMEOUT)
                .send()
                .await?;
            let body = ok_body(response).await?;
            let page = ListObjectsV2::parse_response(&body)
                .map_err(|e| StorageError::Protocol(e.to_string()))?;
            keys.extend(page.contents.into_iter().map(|object| object.key));
            match page.next_continuation_token {
                Some(next) => token = Some(next),
                None => break,
            }
        }
        Ok(keys)
    }

    /// Deletes everything under `prefix` and returns how many objects were removed. `prefix`
    /// must end in `/`, so `videos/1` can never take `videos/10` with it.
    pub async fn delete_prefix(&self, prefix: &str) -> Result<usize, StorageError> {
        if !prefix.ends_with('/') || prefix == "/" {
            return Err(StorageError::Protocol(format!(
                "refusing to delete by prefix {prefix:?}"
            )));
        }
        let keys = self.list_prefix(prefix).await?;
        let count = keys.len();
        // Owned clones, not borrows: borrowed futures inside a stream make the caller's future
        // fail rustc's "Send is not general enough" check when it is spawned.
        let mut removals = futures_util::stream::iter(keys)
            .map(|key| {
                let store = self.clone();
                async move { store.delete(&key).await }
            })
            .buffer_unordered(8);
        while let Some(result) = removals.next().await {
            result?;
        }
        Ok(count)
    }

    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let action = DeleteObject::new(&self.internal, Some(&self.credentials), key);
        let response = self
            .http
            .delete(action.sign(SIGNED_URL_TTL))
            .timeout(CONTROL_TIMEOUT)
            .send()
            .await?;
        match response.status() {
            status if status.is_success() => Ok(()),
            StatusCode::NOT_FOUND => Ok(()),
            _ => Err(error_from(response).await),
        }
    }
}

async fn ok_body(response: reqwest::Response) -> Result<String, StorageError> {
    if !response.status().is_success() {
        return Err(error_from(response).await);
    }
    Ok(response.text().await?)
}

async fn error_from(response: reqwest::Response) -> StorageError {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    StorageError::Http {
        status,
        snippet: snippet(&body),
    }
}

fn snippet(body: &str) -> String {
    body.chars().take(MAX_ERROR_SNIPPET).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_debug_never_shows_credentials() {
        let config = StorageConfig {
            endpoint: "http://localhost:9000".parse().unwrap(),
            public_endpoint: "http://localhost:9000".parse().unwrap(),
            region: "us-east-1".into(),
            bucket: "b".into(),
            access_key: "AKIAEXAMPLE".into(),
            secret_key: "super-secret-value".into(),
            path_style: true,
        };
        let shown = format!("{config:?}");
        assert!(!shown.contains("AKIAEXAMPLE") && !shown.contains("super-secret-value"));
    }

    #[test]
    fn presigned_part_url_signs_the_exact_length_and_uses_the_public_host() {
        let config = StorageConfig {
            endpoint: "http://minio.internal:9000".parse().unwrap(),
            public_endpoint: "https://uploads.zeno.in".parse().unwrap(),
            region: "us-east-1".into(),
            bucket: "zeno".into(),
            access_key: "key".into(),
            secret_key: "secret".into(),
            path_style: true,
        };
        let store = S3Store::new(&config).unwrap();
        let url = store.presign_part(
            "uploads/x/source",
            "up-1",
            3,
            16_777_216,
            Duration::from_secs(60),
        );
        assert_eq!(url.host_str(), Some("uploads.zeno.in"));
        let query = url.query().unwrap();
        assert!(query.contains("partNumber=3") && query.contains("uploadId=up-1"));
        assert!(
            query.contains("X-Amz-SignedHeaders=content-length%3Bhost")
                || query.contains("X-Amz-SignedHeaders=content-length;host"),
            "content-length must be a signed header: {query}"
        );
    }
}
