//! The API described as an OpenAPI 3.1 document.
//!
//! The description is written next to the handlers (`#[utoipa::path]`) and the types
//! (`#[derive(ToSchema)]`), so it is compiled together with them and cannot name a type that no
//! longer exists. What the compiler cannot check, the tests do: `tests/openapi.rs` fails when the
//! committed `docs/openapi.json` is out of date, when a route is missing from the document, and
//! when a real response does not match the schema the document promises for it.
//!
//! The web app generates its types from the committed file (`npm run api:types`), so a change to
//! the API that the web app has not caught up with is a type error, not a surprise in production.
//!
//! Covered so far: authentication, the recordings library, and share links (the owner's side and
//! the visitor's). Other areas are added as the web app starts to use them; `tests/openapi.rs` lists the routes
//! that exist but are not described yet, so none is forgotten.

use serde_json::{json, Value};
use utoipa::openapi::path::{Operation, PathItem};
use utoipa::openapi::response::ResponseBuilder;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::openapi::{ContentBuilder, Ref, RefOr, Response, Schema};
use utoipa::{Modify, OpenApi};

use crate::auth::cookies::ACCESS_COOKIE;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Zeno API",
        version = "1.0.0",
        description = "Everything the web app, the extension and the studio ask the API.\n\n\
            Errors always look like `{\"error\": {\"code\", \"message\", \"fields\"?}}`; branch on \
            `code`. Browsers send `Origin`, and state-changing requests from any other origin are \
            refused with `403 forbidden_origin`.",
    ),
    paths(
        crate::auth::routes::register,
        crate::auth::routes::verify_email,
        crate::auth::routes::resend_verification,
        crate::auth::routes::login,
        crate::auth::routes::refresh,
        crate::auth::routes::logout,
        crate::auth::routes::forgot_password,
        crate::auth::routes::reset_password,
        crate::auth::routes::me,
        crate::auth::routes::activity,
        crate::recordings::routes::create,
        crate::recordings::routes::list,
        crate::recordings::routes::show,
        crate::recordings::routes::update,
        crate::recordings::routes::remove,
        crate::shares::routes::create,
        crate::shares::routes::list,
        crate::shares::routes::revoke,
        crate::shares::routes::resolve,
        crate::shares::routes::play,
    ),
    components(schemas(
        crate::auth::dto::RegisterRequest,
        crate::auth::dto::LoginRequest,
        crate::auth::dto::EmailRequest,
        crate::auth::dto::TokenRequest,
        crate::auth::dto::ResetPasswordRequest,
        crate::auth::dto::SignupRole,
        crate::auth::dto::MessageResponse,
        crate::auth::dto::UserEnvelope,
        crate::auth::dto::UserResponse,
        crate::auth::dto::ActivityResponse,
        crate::auth::dto::ActivityItem,
        crate::error::ErrorBody,
        crate::error::ErrorDetail,
        crate::error::FieldError,
        zeno_core::model::Role,
        crate::recordings::dto::CreateRecordingRequest,
        crate::recordings::dto::UpdateRecordingRequest,
        crate::recordings::dto::RecordingResponse,
        crate::recordings::dto::RecordingEnvelope,
        crate::recordings::dto::RecordingListEnvelope,
        crate::shares::dto::CreateLinkRequest,
        crate::shares::dto::LinkResponse,
        crate::shares::dto::LinkEnvelope,
        crate::shares::dto::LinkListEnvelope,
        crate::shares::dto::ShareView,
        crate::shares::dto::ShareEnvelope,
        crate::shares::dto::PlayRequest,
        crate::shares::dto::SharedWatermark,
        crate::shares::dto::SharedPlayback,
        crate::shares::dto::SharedPlaybackEnvelope,
    )),
    modifiers(&SessionCookie, &CommonErrors, &Enums),
    tags(
        (name = "auth", description = "Accounts and sessions."),
        (name = "recordings", description = "A person's library, and the links that share its recordings."),
        (name = "share", description = "What anyone holding a share link can do. No account, no cookie."),
    )
)]
pub struct ApiDoc;

/// Sessions are cookies, not bearer tokens: the browser attaches them and the page cannot read them.
struct SessionCookie;

impl Modify for SessionCookie {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                ACCESS_COOKIE,
                "The access cookie set by sign-in. It lasts minutes; `POST /v1/auth/refresh` renews it.",
            ))),
        );
    }
}

/// What can go wrong on any route, said once here instead of on every handler: a body that is not
/// JSON or not the right JSON, one that is too large, a request from another site, and a bug.
/// A route's own annotation wins when it already lists the status.
struct CommonErrors;

fn error_response(description: &str) -> RefOr<Response> {
    RefOr::T(
        ResponseBuilder::new()
            .description(description)
            .content(
                "application/json",
                ContentBuilder::new()
                    .schema(Some(Ref::from_schema_name("ErrorResponse")))
                    .build(),
            )
            .build(),
    )
}

fn add_common(operation: &mut Operation, state_changing: bool) {
    let mut add = |status: &str, description: &str| {
        operation
            .responses
            .responses
            .entry(status.to_owned())
            .or_insert_with(|| error_response(description));
    };
    add("500", "`internal_error`: our fault; safe to retry.");
    if state_changing {
        add(
            "403",
            "`forbidden_origin`: the request came from a site that is not the web app.",
        );
    }
    if operation.request_body.is_some() {
        add(
            "400",
            "`invalid_request`: the body is not valid JSON for this route.",
        );
        add(
            "422",
            "`invalid_request`: valid JSON, but not the shape this route takes (or an unknown field).",
        );
        add("413", "`payload_too_large`");
        add(
            "415",
            "`unsupported_media_type`: send `Content-Type: application/json`.",
        );
    }
}

impl Modify for CommonErrors {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        for item in openapi.paths.paths.values_mut() {
            let PathItem {
                get,
                put,
                post,
                delete,
                patch,
                ..
            } = item;
            if let Some(operation) = get {
                add_common(operation, false);
            }
            for operation in [put, post, delete, patch].into_iter().flatten() {
                add_common(operation, true);
            }
        }
    }
}

/// Fields that hold one of a fixed set of words say which words. The lists come from the constants
/// the validation and the response builders use, so the document cannot name a value the API does
/// not, or leave one out.
struct Enums;

fn restrict(
    openapi: &mut utoipa::openapi::OpenApi,
    schema: &str,
    field: &str,
    values: &[&str],
    nullable: bool,
) {
    let components = openapi.components.as_mut().expect("components exist");
    let Some(RefOr::T(Schema::Object(object))) = components.schemas.get_mut(schema) else {
        panic!("{schema} is not an object schema");
    };
    let Some(RefOr::T(Schema::Object(property))) = object.properties.get_mut(field) else {
        panic!("{schema}.{field} is not a plain property");
    };
    let mut words: Vec<Value> = values.iter().map(|word| json!(word)).collect();
    if nullable {
        words.push(Value::Null);
    }
    property.enum_values = Some(words);
}

impl Modify for Enums {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use crate::recordings::dto::{SOURCES, STATUSES, VISIBILITIES};
        use crate::shares::dto::LINK_STATES;
        restrict(openapi, "CreateRecordingRequest", "source", SOURCES, false);
        restrict(
            openapi,
            "UpdateRecordingRequest",
            "visibility",
            VISIBILITIES,
            true,
        );
        restrict(openapi, "RecordingResponse", "source", SOURCES, false);
        restrict(
            openapi,
            "RecordingResponse",
            "visibility",
            VISIBILITIES,
            false,
        );
        restrict(openapi, "RecordingResponse", "status", STATUSES, false);
        restrict(openapi, "LinkResponse", "state", LINK_STATES, false);
        restrict(openapi, "ShareView", "status", STATUSES, false);
    }
}

/// The document, as the committed `docs/openapi.json` should read.
pub fn document() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// The same, as pretty JSON with a trailing newline.
pub fn document_json() -> String {
    let mut json = document()
        .to_pretty_json()
        .expect("the OpenAPI document serialises");
    json.push('\n');
    json
}
