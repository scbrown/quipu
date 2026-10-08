//! Credential provisioning metadata for definite bearer refusals.
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub(super) fn unauthorized(path: &str, why: &str) -> Response {
    (
                                StatusCode::UNAUTHORIZED,
                                axum::Json(serde_json::json!({
                                    "error": format!(
                                        "unauthorized: {path} is an authentication-gated endpoint and requires a bearer \
                                         token. Send `Authorization: Bearer <token>`. Read endpoints \
                                         (/query, /search, entity reads, /health) are open and need no \
                                         credential.{why}"
                                    ),
                                    "endpoint": path,
                                    "reason": "missing_or_invalid_bearer_token",
                                    "credential_type": "bearer",
                                    "provisioning": "Configure a matching QUIPU_AUTH_TOKEN or QUIPU_AUTH_TOKEN_FILE in the client. Signed authentication is supported only on signed-write routes and requires a registered, unexpired identity with write scope.",
                                })),
                            )
                                .into_response()
}
