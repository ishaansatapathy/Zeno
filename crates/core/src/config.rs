use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("invalid value for {name}: {reason}")]
    Invalid { name: &'static str, reason: String },
}

/// Reads a required variable; empty or whitespace-only counts as missing.
pub fn required(name: &'static str) -> Result<String, ConfigError> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(ConfigError::Missing(name)),
    }
}

pub fn parse_or<T>(name: &'static str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => {
            value
                .trim()
                .parse()
                .map_err(|e: T::Err| ConfigError::Invalid {
                    name,
                    reason: e.to_string(),
                })
        }
        _ => Ok(default),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
}

impl Environment {
    pub fn is_production(self) -> bool {
        self == Environment::Production
    }
}

impl FromStr for Environment {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "development" | "dev" => Ok(Environment::Development),
            "production" | "prod" => Ok(Environment::Production),
            other => Err(format!("expected development or production, got {other:?}")),
        }
    }
}

/// Settings every service needs. There are deliberately no defaults for connection
/// strings: a service that silently falls back to a dev database in production is worse
/// than one that refuses to start.
#[derive(Clone)]
pub struct Config {
    pub env: Environment,
    pub database_url: String,
    pub redis_url: String,
    pub bind_addr: SocketAddr,
}

impl Config {
    pub fn from_env(default_bind: &str) -> Result<Self, ConfigError> {
        let env = parse_or("APP_ENV", Environment::Development)?;
        let database_url = required("DATABASE_URL")?;
        let redis_url = required("REDIS_URL")?;
        let bind_addr = match std::env::var("BIND_ADDR") {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => default_bind.to_string(),
        }
        .parse()
        .map_err(|e: std::net::AddrParseError| ConfigError::Invalid {
            name: "BIND_ADDR",
            reason: e.to_string(),
        })?;

        Ok(Self {
            env,
            database_url,
            redis_url,
            bind_addr,
        })
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("env", &self.env)
            .field("bind_addr", &self.bind_addr)
            .field("database_url", &"<redacted>")
            .field("redis_url", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_parses_known_values_only() {
        assert_eq!(
            "production".parse::<Environment>(),
            Ok(Environment::Production)
        );
        assert_eq!("DEV".parse::<Environment>(), Ok(Environment::Development));
        assert!("staging".parse::<Environment>().is_err());
    }

    #[test]
    fn debug_output_never_contains_connection_strings() {
        let config = Config {
            env: Environment::Development,
            database_url: "postgres://user:hunter2@db/zeno".into(),
            redis_url: "redis://:hunter2@cache".into(),
            bind_addr: "127.0.0.1:8080".parse().unwrap(),
        };
        assert!(!format!("{config:?}").contains("hunter2"));
    }
}
