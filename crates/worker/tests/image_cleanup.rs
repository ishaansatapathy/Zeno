mod common;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeno_worker::runner;

#[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
async fn cleanup_keeps_live_images_and_deletes_unreferenced_ones(pool: PgPool) {
    let ctx = common::context(pool.clone()).await;
    let user = Uuid::new_v4();
    let course = Uuid::new_v4();
    let key = format!("images/courses/{course}/test-{}.jpg", Uuid::new_v4());
    sqlx::query(
        "insert into users(id, name, password_hash, role) values($1,'Teacher','x','creator')",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into courses(id, creator_id, title, slug, thumbnail_key) \
         values($1,$2,'Course',$3,$4)",
    )
    .bind(course)
    .bind(user)
    .bind(format!("course-{course}"))
    .bind(&key)
    .execute(&pool)
    .await
    .unwrap();
    ctx.store
        .put_bytes(&key, vec![1, 2, 3], "image/jpeg")
        .await
        .unwrap();

    let enqueue_due = |key: String| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "insert into jobs(kind, payload, run_at) \
                 values('image_cleanup', jsonb_build_object('key', $1::text), now())",
            )
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    enqueue_due(key.clone()).await;
    assert!(runner::run_once(&ctx, &CancellationToken::new())
        .await
        .unwrap());
    assert!(
        ctx.store.head(&key).await.unwrap().is_some(),
        "live image survives"
    );

    sqlx::query("update courses set thumbnail_key = null where id = $1")
        .bind(course)
        .execute(&pool)
        .await
        .unwrap();
    enqueue_due(key.clone()).await;
    assert!(runner::run_once(&ctx, &CancellationToken::new())
        .await
        .unwrap());
    assert!(
        ctx.store.head(&key).await.unwrap().is_none(),
        "unreferenced image is removed"
    );
}
