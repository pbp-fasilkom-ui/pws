use hyper::{client::HttpConnector, Body};
use pemasak_infra::{
    build_logs, configuration,
    queue::{build_queue_handler, BuildQueue},
    startup, telemetry,
};
use sqlx::postgres::PgPoolOptions;
use std::{net::TcpListener, path::Path, process};
use tokio::fs::OpenOptions;

type Client = hyper::client::Client<HttpConnector, Body>;

#[tokio::main]
async fn main() {
    telemetry::init_tracing();
    let config = match configuration::get_configuration() {
        Ok(config) => config,
        Err(err) => {
            tracing::error!(?err, "Failed to read configuration");
            process::exit(1);
        }
    };

    let pool = match PgPoolOptions::new()
        .max_connections(300)
        .min_connections(40)
        .acquire_timeout(std::time::Duration::from_secs(config.database.timeout))
        .idle_timeout(std::time::Duration::from_secs(600)) // 10 minutes
        .max_lifetime(std::time::Duration::from_secs(1800)) // 30 minutes
        .connect_with(config.connection_options())
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(?err, "Failed to connect to Postgres");
            process::exit(1);
        }
    };

    // check if the database is up
    if let Err(err) = sqlx::query("SELECT 1").fetch_one(&pool).await.map(|_| ()) {
        tracing::error!(?err, "Failed to query Postgres");
        process::exit(1);
    }

    // Refuse to start quietly on an unmigrated database. verify_token rejects
    // any api_token row that is not a SHA-256 digest, so deploying without
    // running migrate_git_tokens silently revokes git push for every existing
    // project while /health keeps returning 200 -- the deploy script's rollback
    // would never fire and the breakage would surface only as student reports.
    match sqlx::query_scalar::<_, i64>(
        r#"SELECT count(*) FROM api_token
           JOIN projects ON api_token.project_id = projects.id
           WHERE projects.deleted_at IS NULL
             AND api_token.token NOT LIKE 'sha256:%'
             AND api_token.token NOT LIKE '$argon2%'"#,
    )
    .fetch_one(&pool)
    .await
    {
        Ok(0) => {}
        Ok(unconverted) => {
            tracing::error!(
                unconverted,
                "Refusing to start: {unconverted} git token(s) are still plaintext, so every \
                 push to those projects would fail. Run: docker compose run --rm --entrypoint \
                 /app/migrate_git_tokens server --hash"
            );
            process::exit(1);
        }
        Err(err) => {
            tracing::error!(?err, "Failed to check git token migration state");
            process::exit(1);
        }
    }

    // Per-user tokens require api_token.user_id, added by migration.sql. Without
    // it the credential lookup errors on every git request and basic_auth turns
    // that into a 401 -- so every push fails while the server still boots, still
    // answers /health with 200, and the deploy still reports success. Refuse to
    // start instead, so the health check fails and the deploy rolls back.
    match sqlx::query_scalar::<_, i64>(
        r#"SELECT count(*) FROM information_schema.columns
           WHERE table_name = 'api_token' AND column_name = 'user_id'"#,
    )
    .fetch_one(&pool)
    .await
    {
        Ok(1..) => {}
        Ok(_) => {
            tracing::error!(
                "Refusing to start: api_token.user_id is missing, so every git push would \
                 fail with a 401. Apply migration.sql before deploying this version."
            );
            process::exit(1);
        }
        Err(err) => {
            tracing::error!(?err, "Failed to check the api_token schema");
            process::exit(1);
        }
    }

    // Argon2 rows are deliberately excluded from the gate above. They came from
    // an earlier revision of this branch and cannot be converted -- the
    // plaintext is unrecoverable -- so the owner has to regenerate through the
    // running application. Refusing to boot on them would be a deadlock: the
    // only remedy requires the server that is refusing to start.
    match sqlx::query_scalar::<_, i64>(
        r#"SELECT count(*) FROM api_token
           JOIN projects ON api_token.project_id = projects.id
           WHERE projects.deleted_at IS NULL
             AND api_token.token LIKE '$argon2%'"#,
    )
    .fetch_one(&pool)
    .await
    {
        Ok(0) => {}
        Ok(stuck) => tracing::warn!(
            stuck,
            "{stuck} project(s) hold an unconvertible Argon2 git token; their pushes will fail \
             until each owner regenerates from project settings"
        ),
        Err(err) => tracing::warn!(?err, "Failed to count Argon2 git tokens"),
    }

    // Atlas migration check removed - using schema.sql initialization instead

    // check docker permissions
    if let Err(err) = tokio::fs::metadata("/var/run/docker.sock").await {
        tracing::error!(?err, "Failed to access docker socket");
        process::exit(1);
    }

    // check if git folder exists
    match tokio::fs::metadata(&config.git.base).await {
        Err(err) => {
            tracing::error!(?err, "Failed to access git folder");
            process::exit(1);
        }
        Ok(metadata) => {
            if !metadata.is_dir() {
                tracing::error!("Git folder is not a directory");
                process::exit(1);
            }
            if metadata.permissions().readonly() {
                tracing::error!("Git folder is read-only");
                process::exit(1);
            }

            let git_path = Path::new(&config.git.base);
            let temp_path = git_path.join("temp");
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .await
            {
                Ok(_) => {
                    // Clean up: remove the temporary file
                    if let Err(err) = tokio::fs::remove_file(&temp_path).await {
                        tracing::error!(?err, "Failed to remove temporary file");
                    }
                }
                Err(err) => {
                    tracing::error!(?err, "Cannot write to the git folder");
                    process::exit(1);
                }
            }
        }
    }

    let build_logs = build_logs::new_registry();
    let (build_queue, build_channel) = BuildQueue::new(
        config.build.max,
        pool.clone(),
        config.clone(),
        build_logs.clone(),
    );

    tokio::spawn(async move {
        build_queue_handler(build_queue).await;
    });

    let state = startup::AppState {
        base: config.git.base.clone(),
        git_auth: config.git.auth,
        sso: config.auth.sso,
        client: Client::new(),
        domain: config.domain(),
        build_channel,
        build_logs,
        pool,
        secure: config.application.secure,
    };

    let addr_string = config.address_string();

    let addr = match config.address() {
        Ok(addr) => addr,
        Err(err) => {
            tracing::error!(?err, "Failed to parse address {}", addr_string);
            process::exit(1);
        }
    };

    let listener = match TcpListener::bind(addr) {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!(?err, "Failed to bind address {}", addr_string);
            process::exit(1);
        }
    };

    if let Err(err) = startup::run(listener, state, config).await {
        tracing::error!(?err, "Failed to start server on address {}", addr_string);
        process::exit(1);
    };
}
