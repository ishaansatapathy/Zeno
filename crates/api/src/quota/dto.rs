use serde::Serialize;
use utoipa::ToSchema;

use crate::quota::repo::Usage;

#[derive(Debug, Serialize, ToSchema)]
pub struct StorageUsage {
    /// What the person has kept: the size of every video they have uploaded and still have, as it was sent.
    /// Deleted and failed videos are not counted; one still uploading is.
    pub used_bytes: i64,
    /// What they may keep.
    pub quota_bytes: i64,
    /// `quota_bytes` less `used_bytes`, never below zero.
    pub remaining_bytes: i64,
    /// How many videos make up `used_bytes`.
    pub videos: i64,
}

impl From<Usage> for StorageUsage {
    fn from(usage: Usage) -> Self {
        Self {
            used_bytes: usage.used_bytes,
            quota_bytes: usage.quota_bytes,
            remaining_bytes: usage.remaining(),
            videos: usage.videos,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StorageEnvelope {
    pub storage: StorageUsage,
}
