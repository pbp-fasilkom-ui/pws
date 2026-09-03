//! Behaviour tests for the shared authorization helpers.
//!
//! These exercise the real queries in `pemasak_infra::authz` against a real
//! Postgres, because they are runtime `sqlx::query` calls rather than the
//! compile-checked macro -- a mistake in the SQL is invisible until executed.
//!
//! Requires a throwaway database:
//!
//!   docker run -d --name pws-test -e POSTGRES_PASSWORD=test \
//!     -e POSTGRES_DB=pwstest -p 55432:5432 postgres:15
//!   export TEST_DATABASE_URL=postgres://postgres:test@localhost:55432/pwstest
//!   cargo test --test authz
//!
//! With TEST_DATABASE_URL unset the tests report that they were skipped and
//! pass, so `cargo test` stays green where no database is available.

use pemasak_infra::authz::{has_project_access, is_owner_member, is_project_owner};
use sqlx::{Executor, PgPool};
use uuid::Uuid;

const SCHEMA: &str = include_str!("../schema.sql");
const MIGRATION: &str = include_str!("../migration.sql");

fn uid(suffix: &str) -> Uuid {
    Uuid::parse_str(&format!("00000000-0000-0000-0000-{suffix:0>12}")).unwrap()
}

fn alice() -> Uuid {
    uid("a1")
}
fn bob() -> Uuid {
    uid("b1")
}
fn carol() -> Uuid {
    uid("c1")
}
fn dave() -> Uuid {
    uid("d1")
}
fn eve() -> Uuid {
    uid("e1")
}

/// Two owners with a same-named project, a share, an unrelated user, a user who
/// is both member and share, a project with two shares, and a soft-deleted
/// project.
const FIXTURE: &str = r#"
INSERT INTO users (id, username, password, name) VALUES
  ('00000000-0000-0000-0000-0000000000a1','alice','x','Alice'),
  ('00000000-0000-0000-0000-0000000000b1','bob','x','Bob'),
  ('00000000-0000-0000-0000-0000000000c1','carol','x','Carol'),
  ('00000000-0000-0000-0000-0000000000d1','dave','x','Dave'),
  ('00000000-0000-0000-0000-0000000000e1','eve','x','Eve');

INSERT INTO project_owners (id, name) VALUES
  ('10000000-0000-0000-0000-0000000000a1','alice'),
  ('10000000-0000-0000-0000-0000000000b1','bob'),
  ('10000000-0000-0000-0000-0000000000d1','dave'),
  ('10000000-0000-0000-0000-0000000000ca','Alice');

INSERT INTO users_owners (user_id, owner_id) VALUES
  ('00000000-0000-0000-0000-0000000000a1','10000000-0000-0000-0000-0000000000a1'),
  ('00000000-0000-0000-0000-0000000000b1','10000000-0000-0000-0000-0000000000b1'),
  ('00000000-0000-0000-0000-0000000000d1','10000000-0000-0000-0000-0000000000d1'),
  ('00000000-0000-0000-0000-0000000000e1','10000000-0000-0000-0000-0000000000a1');

INSERT INTO projects (id, owner_id, name) VALUES
  ('20000000-0000-0000-0000-0000000000a1','10000000-0000-0000-0000-0000000000a1','app'),
  ('20000000-0000-0000-0000-0000000000b1','10000000-0000-0000-0000-0000000000b1','app'),
  ('20000000-0000-0000-0000-0000000000ff','10000000-0000-0000-0000-0000000000a1','multi');

INSERT INTO projects (id, owner_id, name, deleted_at) VALUES
  ('20000000-0000-0000-0000-0000000000de','10000000-0000-0000-0000-0000000000a1','gone', now());

INSERT INTO project_shares (project_id, user_id) VALUES
  ('20000000-0000-0000-0000-0000000000a1','00000000-0000-0000-0000-0000000000c1'),
  ('20000000-0000-0000-0000-0000000000ff','00000000-0000-0000-0000-0000000000c1'),
  ('20000000-0000-0000-0000-0000000000ff','00000000-0000-0000-0000-0000000000d1'),
  ('20000000-0000-0000-0000-0000000000a1','00000000-0000-0000-0000-0000000000e1');
"#;

/// Builds a pool against a database of this test's own, so tests can run in
/// parallel without racing on the shared schema. `schema.sql` creates enum
/// types, which cannot simply be re-created in a shared database.
async fn setup(test_name: &str) -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;

    // Swap the database component to reach the maintenance database, which is
    // where CREATE DATABASE has to be issued from.
    let (server, _) = url.rsplit_once('/').expect("TEST_DATABASE_URL must include a database");
    let admin_url = format!("{server}/postgres");
    let db_name = format!("pws_test_{test_name}");
    let test_url = format!("{server}/{db_name}");

    let admin = PgPool::connect(&admin_url)
        .await
        .expect("connect to maintenance database");
    admin
        .execute(format!("DROP DATABASE IF EXISTS {db_name}").as_str())
        .await
        .expect("drop previous test database");
    admin
        .execute(format!("CREATE DATABASE {db_name}").as_str())
        .await
        .expect("create test database");
    admin.close().await;

    let pool = PgPool::connect(&test_url).await.expect("connect to test database");
    pool.execute(SCHEMA).await.expect("apply schema.sql");
    pool.execute(MIGRATION).await.expect("apply migration.sql");
    pool.execute(FIXTURE).await.expect("load fixture");

    Some(pool)
}

macro_rules! skip_without_db {
    ($pool:expr) => {
        match $pool {
            Some(pool) => pool,
            None => {
                eprintln!("SKIPPED: set TEST_DATABASE_URL to run authz behaviour tests");
                return;
            }
        }
    };
}

#[tokio::test]
async fn has_project_access_matrix() {
    let pool = skip_without_db!(setup("has_project_access_matrix").await);

    // Owner of the namespace.
    assert!(has_project_access(&pool, "alice", "app", alice()).await.unwrap());
    // Holder of a share on the project.
    assert!(has_project_access(&pool, "alice", "app", carol()).await.unwrap());

    // A user who is BOTH a namespace member and a share holder matches on more
    // than one join row. Guards against fetch_optional rejecting multiple rows,
    // which would deny a legitimate user, since every call site treats Err as
    // deny.
    assert!(has_project_access(&pool, "alice", "app", eve()).await.unwrap());

    // A project with several shares likewise multiplies rows.
    assert!(has_project_access(&pool, "alice", "multi", carol()).await.unwrap());
    assert!(has_project_access(&pool, "alice", "multi", dave()).await.unwrap());

    // Member of a different namespace.
    assert!(!has_project_access(&pool, "alice", "app", bob()).await.unwrap());
    // No relationship at all.
    assert!(!has_project_access(&pool, "alice", "app", dave()).await.unwrap());
    assert!(!has_project_access(&pool, "alice", "multi", bob()).await.unwrap());
}

/// The case the old git-auth bug turned on: two owners with a same-named
/// project. Credentials for your own project must not reach the other one.
#[tokio::test]
async fn same_project_name_under_two_owners_is_not_confused() {
    let pool = skip_without_db!(setup("same_project_name_under_two_owners_is_not_confused").await);

    assert!(has_project_access(&pool, "alice", "app", alice()).await.unwrap());
    assert!(has_project_access(&pool, "bob", "app", bob()).await.unwrap());

    assert!(!has_project_access(&pool, "bob", "app", alice()).await.unwrap());
    assert!(!has_project_access(&pool, "alice", "app", bob()).await.unwrap());
}

/// Owner names are compared exactly; a name differing only by case is a
/// different namespace and must not grant access.
#[tokio::test]
async fn owner_name_comparison_is_case_sensitive() {
    let pool = skip_without_db!(setup("owner_name_comparison_is_case_sensitive").await);

    assert!(!has_project_access(&pool, "Alice", "app", alice()).await.unwrap());
    assert!(!is_owner_member(&pool, "Alice", alice()).await.unwrap());
    assert!(is_owner_member(&pool, "alice", alice()).await.unwrap());
}

/// A share must not confer ownership: it is what gates membership changes and
/// deletion.
#[tokio::test]
async fn is_project_owner_rejects_share_holders() {
    let pool = skip_without_db!(setup("is_project_owner_rejects_share_holders").await);

    assert!(is_project_owner(&pool, "alice", "app", alice()).await.unwrap());
    assert!(!is_project_owner(&pool, "alice", "app", carol()).await.unwrap());
    assert!(!is_project_owner(&pool, "bob", "app", alice()).await.unwrap());
}

#[tokio::test]
async fn is_owner_member_matrix() {
    let pool = skip_without_db!(setup("is_owner_member_matrix").await);

    assert!(is_owner_member(&pool, "alice", alice()).await.unwrap());
    assert!(is_owner_member(&pool, "alice", eve()).await.unwrap());
    assert!(!is_owner_member(&pool, "alice", carol()).await.unwrap());
    assert!(!is_owner_member(&pool, "bob", alice()).await.unwrap());
    // A namespace that does not exist.
    assert!(!is_owner_member(&pool, "nobody", alice()).await.unwrap());
}

/// Documents current behaviour: the helpers do NOT filter soft-deleted
/// projects, while redeploy_project.rs and view_project_tree.rs both check
/// `deleted_at IS NULL` in their own queries. If the helpers gain that filter,
/// invert these assertions.
#[tokio::test]
async fn soft_deleted_projects_are_still_reachable_via_the_helpers() {
    let pool = skip_without_db!(setup("soft_deleted_projects_are_still_reachable_via_the_helpers").await);

    assert!(
        has_project_access(&pool, "alice", "gone", alice()).await.unwrap(),
        "soft-deleted project unexpectedly filtered -- update this test if that was intended"
    );
    assert!(is_project_owner(&pool, "alice", "gone", alice()).await.unwrap());
}
