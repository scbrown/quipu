//! Startup validation and diagnostics for write authentication.

tokio::task_local! {
    static REQUEST_IDENTITY: Option<quipu::transaction_auth::Identity>;
}

pub(super) fn request_identity() -> Option<quipu::transaction_auth::Identity> {
    REQUEST_IDENTITY.try_with(Clone::clone).ok().flatten()
}

pub(super) fn bearer_policy(
    config: &quipu::config::ServerConfig,
) -> quipu::http_auth::BearerPolicy {
    let auth_policy = match quipu::http_auth::BearerPolicy::new(
        config.auth_token.clone(),
        config.previous_auth_token.clone(),
        config.previous_auth_token_expires_at_epoch_secs,
        quipu::time::epoch_secs(),
    ) {
        Ok(policy) => policy,
        Err(reason) => {
            eprintln!("error: invalid [quipu.server] bearer rotation configuration: {reason}");
            std::process::exit(2);
        }
    };
    if auth_policy.requires_auth() {
        eprintln!("write endpoints require a bearer token");
    }
    if let Some(expiry) = auth_policy.previous_expiry() {
        eprintln!("temporary previous bearer enabled until UTC epoch second {expiry}");
    } else if config.previous_auth_token.is_some() {
        eprintln!(
            "warning: expired previous bearer ignored; remove previous_auth_token and its expiry from configuration"
        );
    }

    match config.crew_credentials_file.as_deref() {
        None => auth_policy,
        Some(path) => match load_registry(path) {
            Ok(registry) => {
                eprintln!(
                    "additive crew credential registry: {} entries",
                    registry.len()
                );
                auth_policy.with_named(registry)
            }
            Err(reason) => {
                // Optional additions cannot disable an existing shared bearer.
                // Deployment must validate the candidate before provisioning clients.
                eprintln!(
                    "warning: crew credential registry not activated: {reason}; shared bearer policy unchanged"
                );
                auth_policy
            }
        },
    }
}

fn load_registry(path: &str) -> Result<quipu::crew_credentials::CredentialRegistry, &'static str> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "cannot open registry file")?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read registry file")?;
    quipu::crew_credentials::CredentialRegistry::parse(&bytes, "quipu")
}

pub(super) async fn run_authorized(
    req: axum::extract::Request,
    next: axum::middleware::Next,
    policy: &quipu::http_auth::BearerPolicy,
    authorization: quipu::http_auth::Authorization,
    header: Option<&str>,
) -> axum::response::Response {
    if authorization.generation == Some(quipu::http_auth::AuthGeneration::Named) {
        let principal = policy
            .named_principal(header)
            .expect("named authorization has a principal");
        run_named(req, next, principal).await
    } else {
        let identity = req
            .extensions()
            .get::<quipu::http_auth::AuthenticatedPrincipal>()
            .map(|p| quipu::transaction_auth::Identity {
                principal: p.as_str().to_owned(),
                credential_id: None,
                auth_class: "legacy_shared_bearer".to_owned(),
            });
        REQUEST_IDENTITY.scope(identity, next.run(req)).await
    }
}

pub(super) async fn run_named(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
    principal: quipu::crew_credentials::CrewPrincipal,
) -> axum::response::Response {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let id = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let method = req.method().to_string();
    let endpoint = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or("unmatched", |p| p.as_str())
        .to_owned();
    let event = |phase: &str, status: Option<u16>| {
        serde_json::json!({
            "event": phase, "timestamp": quipu::time::now_iso(),
            "credential_request_id": id, "principal": principal.iri,
            "credential_id": principal.credential_id, "auth_class": "named_bearer",
            "method": method, "endpoint": endpoint, "status": status,
        })
    };
    eprintln!("{}", event("authenticated_request_start", None));
    req.extensions_mut()
        .insert(quipu::http_auth::AuthenticatedPrincipal::from_crew(
            &principal,
        ));
    req.extensions_mut().insert(principal.clone());
    let identity = quipu::transaction_auth::Identity {
        principal: principal.iri.clone(),
        credential_id: Some(principal.credential_id.clone()),
        auth_class: "named_bearer".to_owned(),
    };
    let response = REQUEST_IDENTITY.scope(Some(identity), next.run(req)).await;
    eprintln!(
        "{}",
        event(
            "authenticated_request_complete",
            Some(response.status().as_u16())
        )
    );
    response
}

#[cfg(test)]
mod tests {
    use axum::{Extension, Router, middleware, routing::post};
    use quipu::{crew_credentials::CrewPrincipal, http_auth::AuthenticatedPrincipal};

    #[tokio::test]
    async fn concurrent_named_requests_keep_owned_identity_across_blocking_dispatch() {
        async fn handler(Extension(p): Extension<AuthenticatedPrincipal>) -> String {
            let owned = p.as_str().to_owned();
            tokio::task::yield_now().await;
            tokio::task::spawn_blocking(move || owned).await.unwrap()
        }
        let router = |name: &'static str| {
            Router::new()
                .route(&format!("/{name}"), post(handler))
                .layer(middleware::from_fn(move |req, next| {
                    super::run_named(
                        req,
                        next,
                        CrewPrincipal {
                            iri: format!("urn:crew:{name}"),
                            credential_id: format!("{name}-1"),
                        },
                    )
                }))
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router("alice").merge(router("bob")))
                .await
                .unwrap();
        });
        let request = |name: &'static str| {
            tokio::task::spawn_blocking(move || {
                ureq::post(&format!("http://{address}/{name}"))
                    .set("x-actor", "urn:crew:impostor")
                    .call()
                    .unwrap()
                    .into_string()
                    .unwrap()
            })
        };
        let (a, b) = tokio::join!(request("alice"), request("bob"));
        server.abort();
        assert_eq!(a.unwrap(), "urn:crew:alice");
        assert_eq!(b.unwrap(), "urn:crew:bob");
    }
}

#[cfg(test)]
mod registry_tests {
    use quipu::http_auth::{AccessDecision, AuthGeneration, authorize_bearers};

    #[test]
    fn invalid_optional_registry_preserves_shared_policy() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"not JSON").unwrap();
        let config = quipu::config::ServerConfig {
            auth_token: Some("shared-test-token".into()),
            crew_credentials_file: Some(temp.path().to_string_lossy().into_owned()),
            ..Default::default()
        };
        let policy = super::bearer_policy(&config);
        let result = authorize_bearers(true, false, &policy, Some("Bearer shared-test-token"), 1);
        assert_eq!(result.decision, AccessDecision::Allow);
        assert_eq!(result.generation, Some(AuthGeneration::Current));
        assert_eq!(
            authorize_bearers(true, false, &policy, Some("Bearer invalid"), 1).decision,
            AccessDecision::Unauthorized
        );
    }
}

#[cfg(test)]
#[path = "transaction_auth_tests.rs"]
mod transaction_auth_tests;
