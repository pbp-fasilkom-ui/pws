//! Shared authorization and path-safety helpers.
//!
//! Every handler that acts on a project must go through [`has_project_access`]
//! or [`is_project_owner`] rather than writing its own join, and every handler
//! that builds a filesystem path from request data must go through
//! [`repo_path`]. Hand-rolled copies of these checks are what allowed a family
//! of IDOR and path-traversal bugs to accumulate.

use std::path::{Path, PathBuf};

use sqlx::PgPool;
use uuid::Uuid;

/// Longest permitted `owner` or `project` path segment.
///
/// Matches the `users.username` column width, because an owner namespace is
/// created from a username at registration. A smaller value here would deny
/// git access to any existing account with a longer name.
const MAX_SEGMENT_LEN: usize = 255;

/// True if `user_id` may act on `owner/project`, either through `users_owners`
/// (ownership) or through `project_shares` (an accepted invite).
///
/// Soft-deleted projects are excluded, matching `redeploy_project` and
/// `view_project_tree`. Nothing in the codebase currently writes
/// `projects.deleted_at` -- `delete_project` issues a hard DELETE -- so this is
/// defensive: if soft deletion is ever introduced, authorization is already
/// correct rather than quietly granting access to deleted projects.
pub async fn has_project_access(
    pool: &PgPool,
    owner: &str,
    project: &str,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query(
        r#"SELECT 1 FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           LEFT JOIN users_owners ON project_owners.id = users_owners.owner_id
           LEFT JOIN project_shares ON projects.id = project_shares.project_id
           WHERE projects.name = $1
             AND project_owners.name = $2
             AND projects.deleted_at IS NULL
             AND (users_owners.user_id = $3 OR project_shares.user_id = $3)
        "#,
    )
    .bind(project)
    .bind(owner)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.is_some())
}

/// True if `user_id` owns `owner/project` through `users_owners`.
///
/// Stricter than [`has_project_access`]: a user who only holds a share is not
/// an owner. Use this to gate membership changes, deletion, and anything else
/// a collaborator should not be able to do to someone else's project.
pub async fn is_project_owner(
    pool: &PgPool,
    owner: &str,
    project: &str,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query(
        r#"SELECT 1 FROM projects
           JOIN project_owners ON projects.owner_id = project_owners.id
           JOIN users_owners ON project_owners.id = users_owners.owner_id
           WHERE projects.name = $1
             AND project_owners.name = $2
             AND projects.deleted_at IS NULL
             AND users_owners.user_id = $3
        "#,
    )
    .bind(project)
    .bind(owner)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.is_some())
}

/// True if `user_id` is a member of the owner namespace `owner`, regardless of
/// whether any particular project exists in it. Gates creating projects inside
/// a namespace.
pub async fn is_owner_member(
    pool: &PgPool,
    owner: &str,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query(
        r#"SELECT 1 FROM project_owners
           JOIN users_owners ON project_owners.id = users_owners.owner_id
           WHERE project_owners.name = $1
             AND users_owners.user_id = $2
        "#,
    )
    .bind(owner)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.is_some())
}

/// Rejects anything that must never reach a filesystem path or a container
/// name.
///
/// Usernames — and therefore owner namespaces — may contain dots, so dots
/// cannot simply be banned. Instead `..` is rejected outright and a leading dot
/// is refused, which makes traversal unrepresentable while leaving names like
/// `budi.santoso` valid.
pub fn validate_segment(segment: &str) -> Result<(), &'static str> {
    if segment.is_empty() {
        return Err("must not be empty");
    }
    if segment.len() > MAX_SEGMENT_LEN {
        return Err("is too long");
    }
    if segment.starts_with('.') {
        return Err("must not start with a dot");
    }
    if segment.contains("..") {
        return Err("must not contain '..'");
    }
    if !segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err("may only contain letters, digits, dots, underscores and hyphens");
    }
    Ok(())
}

/// Builds `<base>/<owner>/<project>.git`, refusing anything that would escape
/// `base`.
///
/// Segments are validated before joining, so no `..` or separator can appear;
/// the resulting path is then checked to still be under `base` as a second
/// line of defence in case `base` itself is relative or contains a symlink.
pub fn repo_path(base: &str, owner: &str, project: &str) -> Result<PathBuf, String> {
    validate_segment(owner).map_err(|err| format!("owner {err}"))?;

    let repo = project.strip_suffix(".git").unwrap_or(project);
    validate_segment(repo).map_err(|err| format!("project {err}"))?;

    let base_path = Path::new(base);
    let candidate = base_path.join(owner).join(format!("{repo}.git"));

    // `candidate` normally does not exist yet (creation) or is about to be
    // removed (deletion), so canonicalize the base and compare prefixes rather
    // than canonicalizing the candidate itself.
    let canonical_base = base_path
        .canonicalize()
        .unwrap_or_else(|_| base_path.to_path_buf());
    let canonical_candidate = canonical_base.join(owner).join(format!("{repo}.git"));

    if !canonical_candidate.starts_with(&canonical_base) {
        return Err("resolved path escapes the repository root".to_string());
    }

    Ok(candidate)
}

/// Every container in the platform's own compose stack is named
/// `<something>-pemasak`. A project container is named `<owner>-<project>`, so
/// a name ending in this suffix would collide with infrastructure — reaching
/// the app container, which holds the host Docker socket, or the database.
const RESERVED_CONTAINER_SUFFIX: &str = "-pemasak";

/// Docker container name for a project, derived only from validated segments.
///
/// The reserved-suffix check is applied to the *derived* name rather than to
/// the segments, because dots become dashes: owner `node` with project
/// `exporter.pemasak` would otherwise produce `node-exporter-pemasak`.
pub fn container_name(owner: &str, project: &str) -> Result<String, String> {
    validate_segment(owner).map_err(|err| format!("owner {err}"))?;
    let repo = project.strip_suffix(".git").unwrap_or(project);
    validate_segment(repo).map_err(|err| format!("project {err}"))?;

    let name = format!("{owner}-{repo}").replace('.', "-");

    if name
        .to_ascii_lowercase()
        .ends_with(RESERVED_CONTAINER_SUFFIX)
    {
        return Err("resolves to a reserved infrastructure container name".to_string());
    }

    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_segments() {
        for bad in ["..", "../etc", "a/../b", ".", ".hidden", "", "a/b", "a\\b"] {
            assert!(validate_segment(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn accepts_names_up_to_the_column_width() {
        assert!(validate_segment(&"a".repeat(255)).is_ok());
        assert!(validate_segment(&"a".repeat(256)).is_err());
    }

    #[test]
    fn accepts_realistic_names() {
        for good in ["budi.santoso", "tugas-2", "my_project", "abc123"] {
            assert!(validate_segment(good).is_ok(), "{good} should be accepted");
        }
    }

    #[test]
    fn repo_path_refuses_traversal() {
        assert!(repo_path("./git-repo", "..", "app").is_err());
        assert!(repo_path("./git-repo", "me", "../../etc/passwd").is_err());
        assert!(repo_path("./git-repo", "me/../you", "app").is_err());
    }

    #[test]
    fn repo_path_is_stable_for_git_suffix() {
        let with = repo_path("./git-repo", "me", "app.git").unwrap();
        let without = repo_path("./git-repo", "me", "app").unwrap();
        assert_eq!(with, without);
        assert!(with.ends_with("app.git"));
    }

    #[test]
    fn container_name_matches_previous_scheme() {
        assert_eq!(
            container_name("budi.santoso", "app.git").unwrap(),
            "budi-santoso-app"
        );
        assert!(container_name("..", "app").is_err());
    }

    #[test]
    fn container_name_refuses_infrastructure_collisions() {
        // Direct collision with the app container that holds the Docker socket.
        assert!(container_name("server", "pemasak").is_err());
        assert!(container_name("db", "pemasak").is_err());
        // Dots become dashes, so the suffix can also be reached indirectly.
        assert!(container_name("node", "exporter.pemasak").is_err());
        // A project that merely mentions the word is still fine.
        assert!(container_name("me", "pemasak-clone").is_ok());
    }
}
