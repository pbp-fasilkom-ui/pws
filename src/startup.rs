use axum::extract::State;
use axum::middleware::Next;
use axum::response::Redirect;
use axum::routing::get;
use axum::{middleware, routing, Router};

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, Uri};
use axum_session::SessionLayer;
use axum_session_auth::AuthSessionLayer;
use axum_session_sqlx::SessionPgPool;
use bollard::Docker;
use bytes::Bytes;
use http_body::combinators::UnsyncBoxBody;

use sqlx::PgPool;
use tokio::sync::mpsc::Sender;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use std::net::{SocketAddr, TcpListener};

use crate::auth::User;
use crate::build_logs::BuildLogRegistry;
use crate::configuration::Settings;
use crate::queue::BuildQueueItem;
use crate::{auth, dashboard, git, owner, projects, telemetry};

#[derive(Clone)]
pub struct AppState {
    pub base: String,
    pub git_auth: bool,
    pub sso: bool,
    pub domain: String,
    pub pool: PgPool,
    pub build_channel: Sender<BuildQueueItem>,
    pub build_logs: BuildLogRegistry,
    pub secure: bool,
}

pub async fn run(listener: TcpListener, state: AppState, config: Settings) -> Result<(), String> {
    let http_trace = telemetry::http_trace_layer();
    let pool = state.pool.clone();

    let (auth_config, session_store) = auth::auth_layer(&pool, &config).await;

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(["Content-Type".parse().unwrap()])
        .allow_origin([
            "http://localhost:8080".parse().unwrap(),
            "http://localhost:5173".parse().unwrap(),
            format!("https://{}", config.domain()).parse().unwrap(),
            format!("http://{}", config.domain()).parse().unwrap(),
        ])
        .allow_credentials(true);

    let git_router = git::router(state.clone(), &config);
    let auth_router = auth::api::router(state.clone(), &config).await;
    let dashboard_router: Router<AppState> = dashboard::api::router(state.clone(), &config).await;
    let project_router = projects::api::router(state.clone(), &config).await;
    let owners_router = owner::api::router(state.clone(), &config).await;

    let app = Router::new()
        .route("/", routing::any(|| async { Redirect::permanent("/web") }))
        .merge(git_router)
        .merge(auth_router)
        .merge(dashboard_router)
        .merge(project_router)
        .merge(owners_router)
        .layer(http_trace)
        // TODO: rethink if we need this here. since it makes all routes under this query the
        // session even if they don't need it
        .layer(
            AuthSessionLayer::<User, Uuid, SessionPgPool, PgPool>::new(Some(pool.clone()))
                .with_config(auth_config),
        )
        .layer(SessionLayer::new(session_store))
        .route("/health", get(health_check)) // Health check without auth layers
        .route(
            "/web",
            routing::get(|| async { Redirect::permanent("/web/") }),
        )
        .nest_service("/assets", ServeDir::new("assets"))
        // TODO: find a way to have this on the "/" path instead of "/web"
        .nest_service(
            "/web/",
            ServeDir::new("ui/dist").fallback(ServeFile::new("ui/dist/index.html")),
        )
        // .fallback(fallback)  // Disabled: Traefik handles routing directly
        .with_state(state.clone())
        // .route_layer(middleware::from_fn_with_state(state, fallback_middleware))  // Disabled with fallback
        .layer(cors)
        // Several handlers still unwrap on database results. A panic in one
        // request should return 500, not drop the connection.
        .layer(CatchPanicLayer::new());

    let addr = listener
        .local_addr()
        .map_err(|err| format!("Failed to get local address: {}", err))?;

    tracing::info!("listening on {}", addr);

    // axum 0.8 replaced hyper's Server with axum::serve over a tokio listener.
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("Failed to set non-blocking: {}", err))?;
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|err| format!("Failed to adopt listener: {}", err))?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|err| format!("failed to start server: {}", err))
}

pub async fn health_check() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain")
        .body(Body::from("OK"))
        .unwrap()
}
