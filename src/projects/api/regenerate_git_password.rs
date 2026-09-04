use axum::extract::{Path, State};
use axum::response::Response;
use hyper::{Body, StatusCode};
use serde::Serialize;

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use rand::{Rng, SeedableRng};

use crate::{auth::Auth, startup::AppState};
use sqlx::Row;
use ulid::Ulid;
use uuid::Uuid;

const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const TOKEN_LENGTH: usize = 32;

#[derive(Serialize, Debug)]
struct RegeneratePasswordResponse {
    git_username: String,
    git_password: String,
    git_url: String,
    message: String,
}

#[derive(Serialize, Debug)]
struct ErrorResponse {
    message: String,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn post(
    auth: Auth,
    Path((owner, project)): Path<(String, String)>,
    State(AppState {
        pool,
        domain,
        secure,
        ..
    }): State<AppState>,
) -> Response<Body> {
    let user = match auth.current_user {
        Some(user) => user,
        None => {
            let json = serde_json::to_string(&ErrorResponse {
                message: "Authentication required. Please log in to access this resource."
                    .to_string(),
            })
            .unwrap();

            return Response::builder().body(Body::from(json)).unwrap();
        }
    };

    let project_id: Uuid = match sqlx::query(
        r#"SELECT DISTINCT projects.id
           FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           LEFT JOIN users_owners ON project_owners.id = users_owners.owner_id
           LEFT JOIN project_shares ON projects.id = project_shares.project_id
           WHERE (users_owners.user_id = $3 OR project_shares.user_id = $3)
             AND projects.name = $1
             AND project_owners.name = $2
        "#,
    )
    .bind(&project)
    .bind(&owner)
    .bind(user.id)
    .fetch_optional(&pool)
    .await
    {
        Ok(Some(row)) => row.get::<Uuid, _>("id"),
        Ok(None) => {
            let json = serde_json::to_string(&ErrorResponse {
                message: "Project does not exist or you don't have access".to_string(),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }
        Err(err) => {
            tracing::error!(?err, "Can't get project: Failed to query database");

            let json = serde_json::to_string(&ErrorResponse {
                message: "An internal error occurred".to_string(),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }
    };

    // Generate new password
    let mut rng = rand::rngs::StdRng::from_entropy();
    let new_password = (0..TOKEN_LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect::<String>();

    // Stored hashed. The plaintext is returned once in this response and is
    // not recoverable afterwards -- get_git_credentials never returned it.
    let password_hash = crate::tokens::hash_token(&new_password);

    // Upsert the CALLER's own credential. Previously this rotated the single
    // project-wide token, so any collaborator could invalidate the owner's
    // credential -- and a collaborator had no way to obtain one without doing
    // exactly that. Rotating your own token now affects nobody else, which is
    // what makes member-or-share access to this endpoint correct.
    match sqlx::query(
        r#"INSERT INTO api_token (id, project_id, user_id, token)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (project_id, user_id) WHERE user_id IS NOT NULL
           DO UPDATE SET token = EXCLUDED.token, updated_at = now()"#,
    )
    .bind(Uuid::from(Ulid::new()))
    .bind(project_id)
    .bind(user.id)
    .bind(&password_hash)
    .execute(&pool)
    .await
    {
        Ok(_) => {}
        Err(err) => {
            tracing::error!(?err, "Failed to update password in database");

            let json = serde_json::to_string(&ErrorResponse {
                message: "Failed to update password".to_string(),
            })
            .unwrap();

            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "application/json")
                .body(Body::from(json))
                .unwrap();
        }
    }

    let protocol = match secure {
        true => "https",
        false => "http",
    };

    let git_url = format!("{protocol}://{domain}/{owner}/{project}");

    let json = serde_json::to_string(&RegeneratePasswordResponse {
        git_username: user.username.clone(),
        git_password: new_password,
        git_url,
        message: "Password regenerated successfully. Please save this password as it won't be shown again.".to_string(),
    }).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(json))
        .unwrap()
}
