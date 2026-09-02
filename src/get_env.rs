use std::env;

/// Get environment variable with default value
pub fn get_env_or_default(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Get database user
pub fn db_user() -> String {
    get_env_or_default("DB_USER", "postgres")
}

/// Get database password
pub fn db_password() -> String {
    get_env_or_default("DB_PASSWORD", "123")
}

/// Get database port
pub fn db_port() -> u16 {
    get_env_or_default("DB_PORT", "5432")
        .parse()
        .unwrap_or(5432)
}

/// Get database name
pub fn db_name() -> String {
    get_env_or_default("DB_NAME", "postgres")
}

/// Get application port
pub fn app_port() -> u16 {
    get_env_or_default("APPLICATION_PORT", "8080")
        .parse()
        .unwrap_or(8080)
}

/// Get domain for Traefik routing
pub fn domain() -> String {
    get_env_or_default("DOMAIN", "localhost")
}

/// Get database URL
pub fn database_url() -> String {
    get_env_or_default(
        "DATABASE_URL",
        &format!(
            "postgresql://{}:{}@localhost:{}/{}",
            db_user(),
            db_password(),
            db_port(),
            db_name()
        ),
    )
}

/// Get Grafana admin user
pub fn grafana_user() -> String {
    get_env_or_default("GF_SECURITY_ADMIN_USER", "user")
}

/// Get Grafana admin password
pub fn grafana_password() -> String {
    get_env_or_default("GF_SECURITY_ADMIN_PASSWORD", "password")
}
