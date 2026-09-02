use super::client::CasClient;
use crate::{
    auth::{Auth, ErrorResponse, RegisterUserErrorType, SsoCallbackRequest, User},
    startup::AppState,
};
use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHasher};
use axum::extract::{Json, State};
use garde::Unvalidated;
use hyper::{Body, Response, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashSet;
use ulid::Ulid;
use uuid::Uuid;

#[tracing::instrument(skip(auth, pool))]
pub async fn handle_callback(
    auth: Auth,
    State(AppState { pool, .. }): State<AppState>,
    Json(req): Json<Unvalidated<SsoCallbackRequest>>,
) -> Response<Body> {
    // Validate oncoming request
    let SsoCallbackRequest {
        ticket,
        service_url,
    } = match req.validate(&()) {
        Ok(validated) => validated.into_inner(),
        Err(err) => {
            let body = serde_json::to_string(&json!({ "error": err.to_string() })).unwrap();
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap();
        }
    };

    let cas_server_url = "https://sso.ui.ac.id/cas2/";
    let client = CasClient::new(service_url.clone(), cas_server_url, None);

    // Verify CAS ticket
    let profile = match client.verify_ticket(&ticket).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("CAS verification failed: {:?}", err);
            let body = serde_json::to_string(&json!({ "error": "Invalid ticket" })).unwrap();
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap();
        }
    };

    tracing::warn!(?profile);

    // Lookup or create local user
    let username = &profile.username;
    let fullname = profile
        .attributes
        .as_ref()
        .and_then(|attrs| attrs.nama.clone())
        .unwrap_or_else(|| username.to_string());

    let existing_user = User::get_from_username(&username, &pool).await.ok();

    let user = if let Some(user) = existing_user {
        user
    } else {
        // Check if kd_org ends with "12.01" (FASILKOM UI)
        let is_fasilkom = profile
            .attributes
            .as_ref()
            .and_then(|attrs| attrs.kd_org.as_ref())
            .map(|kd_org| kd_org.ends_with("12.01"))
            .unwrap_or(false);

        if !is_fasilkom {
            let json = serde_json::to_string(&ErrorResponse {
                message: "User is not from UI Faculty of Computer Science".to_string(),
                error_type: RegisterUserErrorType::SSOError,
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }

        let user_id = Uuid::from(Ulid::new());
        let hasher = Argon2::default();
        let salt = SaltString::generate(&mut OsRng);

        // Hash the username
        let password_hash = match hasher.hash_password(username.as_bytes(), &salt) {
            Ok(hash) => hash,
            Err(err) => {
                tracing::error!(?err, "Can't register User: Failed to hash password");
                let json = serde_json::to_string(&ErrorResponse {
                    message: format!("failed to hash password: {}", err.to_string()),
                    error_type: RegisterUserErrorType::InternalServerError,
                })
                .unwrap();
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("Content-Type", "application/json")
                    .body(Body::from(json))
                    .unwrap();
            }
        };

        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(err) => {
                tracing::error!(?err, "Can't insert user: Failed to begin transaction");
                let json = serde_json::to_string(&ErrorResponse {
                    message: "failed to request sso: Failed to begin transaction".to_string(),
                    error_type: RegisterUserErrorType::InternalServerError,
                })
                .unwrap();
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("Content-Type", "application/json")
                    .body(Body::from(json))
                    .unwrap();
            }
        };

        if let Err(err) = sqlx::query!(
            r#"INSERT INTO users (id, username, password, name) VALUES ($1, $2, $3, $4)"#,
            user_id,
            username,
            password_hash.to_string(),
            fullname
        )
        .execute(&mut *tx)
        .await
        {
            tracing::error!(?err, "Can't insert user: Failed to insert into database");
            if let Err(err) = tx.rollback().await {
                tracing::error!(?err, "Can't insert user: Failed to rollback transaction");
            }
            let json = serde_json::to_string(&ErrorResponse {
                message: format!("failed to insert into database: {}", err.to_string()),
                error_type: RegisterUserErrorType::InternalServerError,
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        };

        let owner_id = Uuid::from(Ulid::new());
        if let Err(err) = sqlx::query!(
            r#"INSERT INTO project_owners (id, name) VALUES ($1, $2)"#,
            owner_id,
            username
        )
        .execute(&mut *tx)
        .await
        {
            tracing::error!(
                ?err,
                "Can't insert project_owners: Failed to insert into database"
            );
            if let Err(err) = tx.rollback().await {
                tracing::error!(
                    ?err,
                    "Can't insert project_owners: Failed to rollback transaction"
                );
            }
            let json = serde_json::to_string(&ErrorResponse {
                message: format!("failed to insert into database: {}", err.to_string()),
                error_type: RegisterUserErrorType::InternalServerError,
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        };

        if let Err(err) = sqlx::query!(
            r#"INSERT INTO users_owners (user_id, owner_id) VALUES ($1, $2)"#,
            user_id,
            owner_id,
        )
        .execute(&mut *tx)
        .await
        {
            tracing::error!(
                ?err,
                "Can't insert users_owners: Failed to insert into database"
            );
            if let Err(err) = tx.rollback().await {
                tracing::error!(
                    ?err,
                    "Can't insert users_owners: Failed to rollback transaction"
                );
            }
            let json = serde_json::to_string(&ErrorResponse {
                message: format!("failed to insert into database: {}", err.to_string()),
                error_type: RegisterUserErrorType::InternalServerError,
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        };

        if let Err(err) = tx.commit().await {
            tracing::error!(?err, "Can't register user: Failed to commit transaction");
            let json = serde_json::to_string(&ErrorResponse {
                message: format!("failed to commit transaction: {}", err.to_string()),
                error_type: RegisterUserErrorType::InternalServerError,
            })
            .unwrap();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }

        // Return the new user - need to include password and permissions fields
        User {
            id: user_id,
            username: username.clone(),
            password: password_hash.to_string(),
            name: fullname,
            permissions: HashSet::new(),
        }
    };

    // Login the user (both existing and new users)
    auth.login_user(user.id);

    let json = serde_json::to_string(&json!({
        "message": "Login successful"
    }))
    .unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("HX-Location", "/api/dashboard")
        .body(Body::from(json))
        .unwrap()
}
