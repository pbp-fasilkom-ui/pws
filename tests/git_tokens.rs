//! Behaviour tests for per-user git push tokens.
//!
//! Needs a throwaway database; see tests/authz.rs for the invocation. Skips
//! itself when TEST_DATABASE_URL is unset.

use pemasak_infra::tokens::{candidate_credentials, hash_token, verify_token};
use sqlx::{Executor, PgPool};
use uuid::Uuid;

const SCHEMA: &str = include_str!("../schema.sql");
const MIGRATION: &str = include_str!("../migration.sql");

fn alice() -> Uuid {
    Uuid::parse_str("00000000-0000-0000-0000-0000000000a1").unwrap()
}
fn carol() -> Uuid {
    Uuid::parse_str("00000000-0000-0000-0000-0000000000c1").unwrap()
}

/// alice owns alice/app; carol holds a share on it. bob owns an unrelated
/// project of the same name.
const FIXTURE: &str = r#"
INSERT INTO users (id, username, password, name) VALUES
  ('00000000-0000-0000-0000-0000000000a1','alice','x','Alice'),
  ('00000000-0000-0000-0000-0000000000b1','bob','x','Bob'),
  ('00000000-0000-0000-0000-0000000000c1','carol','x','Carol');

INSERT INTO project_owners (id, name) VALUES
  ('10000000-0000-0000-0000-0000000000a1','alice'),
  ('10000000-0000-0000-0000-0000000000b1','bob');

INSERT INTO users_owners (user_id, owner_id) VALUES
  ('00000000-0000-0000-0000-0000000000a1','10000000-0000-0000-0000-0000000000a1'),
  ('00000000-0000-0000-0000-0000000000b1','10000000-0000-0000-0000-0000000000b1');

INSERT INTO projects (id, owner_id, name) VALUES
  ('20000000-0000-0000-0000-0000000000a1','10000000-0000-0000-0000-0000000000a1','app'),
  ('20000000-0000-0000-0000-0000000000b1','10000000-0000-0000-0000-0000000000b1','app');

INSERT INTO project_shares (project_id, user_id) VALUES
  ('20000000-0000-0000-0000-0000000000a1','00000000-0000-0000-0000-0000000000c1');
"#;

async fn setup(test_name: &str) -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let (server, _) = url
        .rsplit_once('/')
        .expect("TEST_DATABASE_URL must include a database");
    let db_name = format!("pws_tok_{test_name}");

    let admin = PgPool::connect(&format!("{server}/postgres"))
        .await
        .expect("connect admin");
    admin
        .execute(format!("DROP DATABASE IF EXISTS {db_name}").as_str())
        .await
        .expect("drop");
    admin
        .execute(format!("CREATE DATABASE {db_name}").as_str())
        .await
        .expect("create");
    admin.close().await;

    let pool = PgPool::connect(&format!("{server}/{db_name}"))
        .await
        .expect("connect test db");
    pool.execute(SCHEMA).await.expect("schema");
    pool.execute(MIGRATION).await.expect("migration");
    pool.execute(FIXTURE).await.expect("fixture");
    Some(pool)
}

macro_rules! skip_without_db {
    ($pool:expr) => {
        match $pool {
            Some(pool) => pool,
            None => {
                eprintln!("SKIPPED: set TEST_DATABASE_URL to run git token tests");
                return;
            }
        }
    };
}

async fn issue(pool: &PgPool, project: Uuid, user: Option<Uuid>, token: &str) {
    sqlx::query("INSERT INTO api_token (id, project_id, user_id, token) VALUES ($1,$2,$3,$4)")
        .bind(Uuid::new_v4())
        .bind(project)
        .bind(user)
        .bind(hash_token(token))
        .execute(pool)
        .await
        .expect("issue token");
}

fn app() -> Uuid {
    Uuid::parse_str("20000000-0000-0000-0000-0000000000a1").unwrap()
}

/// A remote configured before per-user tokens still authenticates: the
/// project-wide row is presented under the owner namespace.
#[tokio::test]
async fn legacy_project_token_still_authenticates() {
    let pool = skip_without_db!(setup("legacy").await);
    issue(&pool, app(), None, "legacytok").await;

    let found = candidate_credentials(&pool, "alice", "app", "alice")
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert!(verify_token("legacytok", &found[0].0));
    assert!(found[0].1.is_none(), "legacy row must have no user_id");

    // Not offered under anyone else's name.
    let as_carol = candidate_credentials(&pool, "alice", "app", "carol")
        .await
        .unwrap();
    assert!(as_carol.is_empty());
}

/// Each user's credential is returned only under their own username.
#[tokio::test]
async fn per_user_tokens_are_scoped_to_their_owner() {
    let pool = skip_without_db!(setup("per_user").await);
    issue(&pool, app(), Some(alice()), "alicetok").await;
    issue(&pool, app(), Some(carol()), "caroltok").await;

    let a = candidate_credentials(&pool, "alice", "app", "alice")
        .await
        .unwrap();
    assert_eq!(a.len(), 1);
    assert!(verify_token("alicetok", &a[0].0));
    assert!(!verify_token("caroltok", &a[0].0));

    let c = candidate_credentials(&pool, "alice", "app", "carol")
        .await
        .unwrap();
    assert_eq!(c.len(), 1);
    assert!(verify_token("caroltok", &c[0].0));
}

/// The lookup is scoped to the repository in the URL, so a credential for one
/// project cannot authorize another of the same name under a different owner.
#[tokio::test]
async fn a_token_does_not_cross_to_a_same_named_project() {
    let pool = skip_without_db!(setup("cross").await);
    issue(&pool, app(), Some(alice()), "alicetok").await;

    let other = candidate_credentials(&pool, "bob", "app", "alice")
        .await
        .unwrap();
    assert!(
        other.is_empty(),
        "alice's credential must not be offered for bob/app"
    );
}

/// Rotating a personal token must replace it, not accumulate rows, and must
/// leave every other collaborator's credential untouched.
#[tokio::test]
async fn rotation_replaces_only_the_callers_token() {
    let pool = skip_without_db!(setup("rotate").await);
    issue(&pool, app(), Some(alice()), "alicetok").await;
    issue(&pool, app(), Some(carol()), "caroltok").await;

    sqlx::query(
        r#"INSERT INTO api_token (id, project_id, user_id, token)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (project_id, user_id) WHERE user_id IS NOT NULL
           DO UPDATE SET token = EXCLUDED.token, updated_at = now()"#,
    )
    .bind(Uuid::new_v4())
    .bind(app())
    .bind(carol())
    .bind(hash_token("carolnew"))
    .execute(&pool)
    .await
    .expect("rotate");

    let c = candidate_credentials(&pool, "alice", "app", "carol")
        .await
        .unwrap();
    assert_eq!(c.len(), 1, "rotation must replace, not add a second row");
    assert!(verify_token("carolnew", &c[0].0));
    assert!(
        !verify_token("caroltok", &c[0].0),
        "the old token must stop working"
    );

    // The owner is unaffected -- this is the whole point of per-user tokens.
    let a = candidate_credentials(&pool, "alice", "app", "alice")
        .await
        .unwrap();
    assert!(verify_token("alicetok", &a[0].0));
}

/// The partial unique indexes hold.
#[tokio::test]
async fn a_project_cannot_hold_duplicate_tokens() {
    let pool = skip_without_db!(setup("unique").await);
    issue(&pool, app(), None, "legacy1").await;
    issue(&pool, app(), Some(alice()), "alice1").await;

    let second_legacy = sqlx::query(
        "INSERT INTO api_token (id, project_id, user_id, token) VALUES ($1,$2,NULL,$3)",
    )
    .bind(Uuid::new_v4())
    .bind(app())
    .bind(hash_token("legacy2"))
    .execute(&pool)
    .await;
    assert!(
        second_legacy.is_err(),
        "only one project-wide token may exist"
    );

    let second_for_alice =
        sqlx::query("INSERT INTO api_token (id, project_id, user_id, token) VALUES ($1,$2,$3,$4)")
            .bind(Uuid::new_v4())
            .bind(app())
            .bind(alice())
            .bind(hash_token("alice2"))
            .execute(&pool)
            .await;
    assert!(
        second_for_alice.is_err(),
        "only one token per (project, user)"
    );
}
