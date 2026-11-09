use std::fmt;

use serde::Deserialize;

/// A string that must never reach a log: passwords and one-time tokens. `Debug` always
/// prints a placeholder, so a stray `{:?}` on a request struct cannot leak them. Reading the
/// value requires an explicit `expose()`.
#[derive(Clone, Deserialize, Default)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct Body {
        password: Secret,
    }

    #[test]
    fn debug_never_shows_the_value() {
        let body: Body = serde_json::from_str(r#"{"password":"hunter2-hunter2"}"#).unwrap();
        assert!(!format!("{body:?}").contains("hunter2"));
        assert!(!format!("{:#?}", body.password).contains("hunter2"));
        assert_eq!(body.password.expose(), "hunter2-hunter2");
    }
}
