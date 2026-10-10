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
                                    "provisioning": "Obtain an accepted token from this server's administrator. Shantytown and CABOODLE clients read ~/.config/quipu/token (mode 0400), overridden by QUIPU_AUTH_TOKEN_FILE or QUIPU_AUTH_TOKEN. Configure a matching token; installing an arbitrary token cannot grant access. Signed authentication is supported only on signed-write routes and requires a registered, unexpired identity with write scope. See the REST API reference Authentication section for provisioning.",
                                })),
                            )
                                .into_response()
}
