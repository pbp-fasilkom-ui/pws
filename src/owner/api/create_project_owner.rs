use axum::body::Body;
use axum::http::StatusCode;
use axum::{extract::State, response::Response, Form};
use garde::{Unvalidated, Validate};
use serde::{Deserialize, Serialize};
use ulid::Ulid;
use uuid::Uuid;

use crate::{auth::Auth, startup::AppState};

#[derive(Serialize, Debug)]
struct ErrorResponse {
    message: String,
}

/// Matches the JSON error shape every other handler returns. These paths
/// previously rendered one-line HTML through leptos, which was the only real
/// use of that dependency in the codebase.
fn json_error(status: StatusCode, message: impl Into<String>) -> Response<Body> {
    let body = serde_json::to_string(&ErrorResponse {
        message: message.into(),
    })
    .unwrap();

    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

// TODO: separate schema for create and update when needed later on
#[derive(Deserialize, Validate, Debug)]
pub struct CreateProjectOwnerRequest {
    // Owner names become a path segment under the git repository root and part
    // of a container name, so they need the same charset restriction that
    // project names already had. Dots stay permitted because usernames contain
    // them, but a leading dot and `..` are not representable.
    // Deliberately the same charset as usernames (src/auth/mod.rs). Allowing
    // '-' or '_' here would let an owner named `a-b` collide with the user
    // `a.b`, because container names replace dots with dashes -- two tenants
    // deriving one container name.
    #[garde(length(min = 1, max = 128), pattern(r"^[a-zA-Z0-9][a-zA-Z0-9.]*$"))]
    pub name: String,
}

#[tracing::instrument(skip(auth, pool))]
pub async fn post(
    auth: Auth,
    State(AppState { pool, .. }): State<AppState>,
    Form(req): Form<Unvalidated<CreateProjectOwnerRequest>>,
) -> Response<Body> {
    let Some(user) = auth.current_user else {
        return json_error(StatusCode::UNAUTHORIZED, "Unauthorized");
    };

    let data = match req.validate(&()) {
        Ok(valid) => valid.into_inner(),
        Err(err) => return json_error(StatusCode::BAD_REQUEST, err.to_string()),
    };

    // Check for existing project
    match sqlx::query!(
        r#"SELECT id FROM project_owners
        WHERE name = $1
        "#,
        data.name
    )
    .fetch_optional(&pool)
    .await
    {
        Ok(None) => (),
        Ok(Some(_)) => {
            tracing::error!(
                "Project owner already exists with the following name: {}",
                data.name
            );

            return json_error(
                StatusCode::BAD_REQUEST,
                format!("An owner named {} already exists", data.name),
            );
        }
        Err(err) => {
            tracing::error!(
                ?err,
                "Can't get existing project owner: Failed to query database"
            );

            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to query database",
            );
        }
    };

    let owner_id = Uuid::from(Ulid::new());

    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(
                ?err,
                "Can't insert project owner: Failed to begin transaction"
            );
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create owner");
        }
    };

    if let Err(err) = sqlx::query!(
        r#"INSERT INTO project_owners (id, name)
        VALUES ($1, $2)
        "#,
        owner_id,
        data.name
    )
    .execute(&mut *tx)
    .await
    {
        tracing::error!(
            ?err,
            "Can't insert project owner: Failed to insert into database"
        );
        if let Err(err) = tx.rollback().await {
            tracing::error!(
                ?err,
                "Can't insert project owner: Failed to rollback transaction"
            );
        }

        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create owner");
    }

    // Without this the namespace has no member, so `authz::is_owner_member`
    // rejects every attempt to create a project in it -- leaving the namespace
    // permanently unusable by anyone, including whoever created it.
    if let Err(err) = sqlx::query!(
        r#"INSERT INTO users_owners (user_id, owner_id) VALUES ($1, $2)"#,
        user.id,
        owner_id,
    )
    .execute(&mut *tx)
    .await
    {
        tracing::error!(?err, "Can't link project owner to its creator");
        if let Err(err) = tx.rollback().await {
            tracing::error!(?err, "Failed to rollback transaction");
        }

        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create owner");
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::empty())
        .unwrap()
}
