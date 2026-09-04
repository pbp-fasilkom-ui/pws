use std::env;

/// Get environment variable with default value
pub fn get_env_or_default(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Get domain for Traefik routing
pub fn domain() -> String {
    get_env_or_default("DOMAIN", "localhost")
}
