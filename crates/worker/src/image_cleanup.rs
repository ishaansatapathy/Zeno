//! Remove replaced images after public caches have expired. Uploads use unique object keys, so
//! an old cleanup job can never delete a later upload of the same pixels.

use zeno_core::jobs::{self, Job};

use crate::pipeline::Context;

pub const JOB_KIND: &str = "image_cleanup";

pub async fn handle(ctx: &Context, job: &Job) {
    if job.out_of_attempts() {
        let _ = jobs::fail(&ctx.db, job, "image cleanup exceeded attempts").await;
        return;
    }
    match clean(ctx, job).await {
        Ok(()) => {
            if let Err(err) = jobs::complete(&ctx.db, job).await {
                tracing::error!(job_id = %job.id, error = %err, "could not complete image cleanup");
            }
        }
        Err(err) => {
            let detail = err.to_string();
            tracing::warn!(job_id = %job.id, error = %detail, "image cleanup will retry");
            if let Err(update_err) =
                jobs::retry_later(&ctx.db, job, &detail, jobs::backoff(job.attempts)).await
            {
                tracing::error!(job_id = %job.id, error = %update_err, "could not reschedule image cleanup");
            }
        }
    }
}

async fn clean(ctx: &Context, job: &Job) -> anyhow::Result<()> {
    let key = job
        .payload
        .get("key")
        .and_then(serde_json::Value::as_str)
        .filter(|key| {
            (key.starts_with("images/courses/") || key.starts_with("images/avatars/"))
                && key.ends_with(".jpg")
                && !key.contains("..")
        })
        .ok_or_else(|| anyhow::anyhow!("invalid image cleanup key"))?;
    let referenced: bool = sqlx::query_scalar(
        "select exists(select 1 from courses where thumbnail_key = $1 and deleted_at is null) \
         or exists(select 1 from creator_profiles where avatar_key = $1)",
    )
    .bind(key)
    .fetch_one(&ctx.db)
    .await?;
    if !referenced {
        ctx.store.delete(key).await?;
    }
    Ok(())
}
