//! Startup validation and diagnostics for write authentication.

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

    auth_policy
}
