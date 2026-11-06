use zeno_core::storage::StorageError;

/// How a failed job should be treated.
#[derive(Debug)]
pub enum JobError {
    /// The input is bad and retrying cannot help. `code` is what the creator is told.
    Permanent { code: &'static str, detail: String },
    /// Something around the job failed (storage, database, a missing binary). Try again later.
    Transient(String),
    /// The encoder failed on an input that probed fine. Retried a couple of times in case it
    /// was resource pressure, then abandoned so a poison file cannot burn CPU forever.
    Encode(String),
    /// This worker lost its lease or is shutting down; someone else will carry on.
    Cancelled,
}

impl JobError {
    pub fn permanent(code: &'static str, detail: impl Into<String>) -> Self {
        JobError::Permanent {
            code,
            detail: detail.into(),
        }
    }
}

impl From<sqlx::Error> for JobError {
    fn from(err: sqlx::Error) -> Self {
        JobError::Transient(format!("database: {err}"))
    }
}

impl From<std::io::Error> for JobError {
    fn from(err: std::io::Error) -> Self {
        JobError::Transient(format!("local io: {err}"))
    }
}

impl From<StorageError> for JobError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::NotFound => {
                JobError::permanent("processing_failed", "source object is missing from storage")
            }
            StorageError::TooLarge { limit } => JobError::permanent(
                "unsupported_media",
                format!("source is larger than the {limit} byte limit"),
            ),
            other => JobError::Transient(format!("storage: {other}")),
        }
    }
}
