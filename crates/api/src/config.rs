use url::Url;
use zeno_core::config::{parse_or, required, Config, ConfigError};
use zeno_core::playback::PlaybackSigner;
use zeno_core::storage::StorageConfig;

pub struct ApiConfig {
    pub core: Config,
    /// Origin (scheme + host + port) of the web app, without a trailing slash.
    pub client_origin: String,
    pub jwt_access_secret: String,
    pub cookie_secure: bool,
    pub trusted_proxy_hops: usize,
    pub storage: StorageConfig,
    /// Largest video a creator may upload, in bytes.
    pub max_upload_bytes: u64,
    /// Days the original upload is kept once a video is ready (or has failed), in case it needs
    /// re-encoding. After that only the encrypted renditions remain.
    pub source_retention_days: u32,
    /// Signs playback tokens; the key server holds the same secret to check them.
    pub playback: PlaybackSigner,
    /// Public base URL of the key server, without a trailing slash. Key URIs in served
    /// playlists point here.
    pub key_server_url: String,
    /// Playback sessions one account may have open at once.
    pub max_concurrent_streams: u32,
    /// Where course and avatar images are served from, without a trailing slash: a CDN in front
    /// of the bucket. Unset, images are served through short-lived presigned links instead.
    pub image_base_url: Option<String>,
}

impl ApiConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let core = Config::from_env("0.0.0.0:8080")?;
        let client_origin = parse_client_origin(&required("CLIENT_URL")?)?;
        let jwt_access_secret = required("JWT_ACCESS_SECRET")?;
        let cookie_secure = parse_or("COOKIE_SECURE", false)?;
        let trusted_proxy_hops = parse_or("TRUSTED_PROXY_HOPS", 0usize)?;
        let storage = StorageConfig::from_env()?;
        let max_upload_bytes = parse_or("MAX_UPLOAD_BYTES", 10u64 * 1024 * 1024 * 1024)?;
        let source_retention_days = parse_or("SOURCE_RETENTION_DAYS", 7u32)?;
        let playback = PlaybackSigner::from_env()?;
        let key_server_url = parse_public_url("KEY_SERVER_URL", &required("KEY_SERVER_URL")?)?;
        let max_concurrent_streams = parse_or("MAX_CONCURRENT_STREAMS", 2u32)?;
        let image_base_url = match std::env::var("IMAGE_BASE_URL") {
            Ok(raw) if !raw.trim().is_empty() => Some(parse_public_url("IMAGE_BASE_URL", &raw)?),
            _ => None,
        };
        if max_concurrent_streams == 0 {
            return Err(ConfigError::Invalid {
                name: "MAX_CONCURRENT_STREAMS",
                reason: "must be at least 1".into(),
            });
        }

        if jwt_access_secret.len() < 32 {
            return Err(ConfigError::Invalid {
                name: "JWT_ACCESS_SECRET",
                reason: "must be at least 32 bytes".into(),
            });
        }
        if core.env.is_production() {
            if !cookie_secure {
                return Err(ConfigError::Invalid {
                    name: "COOKIE_SECURE",
                    reason: "must be true in production".into(),
                });
            }
            if !key_server_url.starts_with("https://") {
                return Err(ConfigError::Invalid {
                    name: "KEY_SERVER_URL",
                    reason: "must use https in production".into(),
                });
            }
            if image_base_url
                .as_deref()
                .is_some_and(|url| !url.starts_with("https://"))
            {
                return Err(ConfigError::Invalid {
                    name: "IMAGE_BASE_URL",
                    reason: "must use https in production".into(),
                });
            }
            if !client_origin.starts_with("https://") {
                return Err(ConfigError::Invalid {
                    name: "CLIENT_URL",
                    reason: "must use https in production".into(),
                });
            }
        }

        Ok(Self {
            core,
            client_origin,
            jwt_access_secret,
            cookie_secure,
            trusted_proxy_hops,
            storage,
            max_upload_bytes,
            source_retention_days,
            playback,
            key_server_url,
            max_concurrent_streams,
            image_base_url,
        })
    }
}

/// An http(s) URL with no query or fragment, returned without a trailing slash.
pub fn parse_public_url(name: &'static str, raw: &str) -> Result<String, ConfigError> {
    let invalid = |reason: &str| ConfigError::Invalid {
        name,
        reason: reason.to_string(),
    };
    let url = Url::parse(raw.trim()).map_err(|e| invalid(&e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("scheme must be http or https"));
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(invalid("must not carry a query, fragment or credentials"));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

pub fn parse_client_origin(raw: &str) -> Result<String, ConfigError> {
    let invalid = |reason: &str| ConfigError::Invalid {
        name: "CLIENT_URL",
        reason: reason.to_string(),
    };
    let url = Url::parse(raw.trim()).map_err(|e| invalid(&e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("scheme must be http or https"));
    }
    let origin = url.origin();
    if !origin.is_tuple() {
        return Err(invalid("must include a host"));
    }
    Ok(origin.ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_drops_path_and_trailing_slash() {
        assert_eq!(
            parse_client_origin("http://localhost:3000/").unwrap(),
            "http://localhost:3000"
        );
        assert_eq!(
            parse_client_origin("https://zeno.in/app/login?x=1").unwrap(),
            "https://zeno.in"
        );
    }

    #[test]
    fn origin_rejects_non_http_and_garbage() {
        assert!(parse_client_origin("ftp://zeno.in").is_err());
        assert!(parse_client_origin("not a url").is_err());
        assert!(parse_client_origin("javascript:alert(1)").is_err());
    }
}
