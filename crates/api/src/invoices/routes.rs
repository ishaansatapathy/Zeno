use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::AuthUser;
use crate::error::ApiError;
use crate::invoices::dto::{InvoiceData, InvoiceEnvelope};
use crate::invoices::{pdf, service};
use crate::ratelimit::{enforce_user, policies};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/orders/:id/invoice", get(show))
        .route("/v1/orders/:id/invoice.pdf", get(download))
}

fn order_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "Order not found.".into(),
    })
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<InvoiceEnvelope>, ApiError> {
    enforce_user(&state, &policies::ORDER_READ, user.id).await?;
    let row = service::for_buyer(&state, &user, order_id(&id)?).await?;
    Ok(Json(service::envelope(&row)?))
}

/// The invoice as a PDF, as a download. Private to the buyer and never cached by anyone else.
async fn download(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    enforce_user(&state, &policies::ORDER_READ, user.id).await?;
    let row = service::for_buyer(&state, &user, order_id(&id)?).await?;
    let data: InvoiceData =
        serde_json::from_value(row.data.clone()).map_err(anyhow::Error::from)?;
    if !pdf::can_render(&data) {
        return Err(ApiError::Http {
            status: StatusCode::CONFLICT,
            code: "invoice_pdf_unsupported_text",
            message:
                "This invoice contains text that the PDF font cannot display. Use the JSON invoice."
                    .into(),
        });
    }
    // ZNO/2026-27/000042 -> ZNO-2026-27-000042.pdf (a slash is not safe in a file name).
    let file_name = format!("{}.pdf", row.number.replace('/', "-"));
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file_name}\""),
            ),
            (header::CACHE_CONTROL, "private, no-store".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        pdf::render(&data),
    )
        .into_response())
}
