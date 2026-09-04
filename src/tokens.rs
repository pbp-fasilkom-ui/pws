//! Hashing for machine-generated git push tokens.
//!
//! Tokens are 32 characters drawn from a 62-character alphabet by a CSPRNG --
//! roughly 190 bits of entropy. A memory-hard KDF exists to protect
//! *low-entropy* secrets from offline brute force, which does not apply here:
//! nobody enumerates a 190-bit space. Using Argon2 for these bought nothing and
//! cost 19 MiB of allocation plus tens of milliseconds of blocking CPU on the
//! git authentication path, which runs before the caller is authenticated and
//! has no rate limit -- an unauthenticated memory and CPU amplifier.
//!
//! SHA-256 with a constant-time comparison is the right primitive: a database
//! or log disclosure still yields no usable credential, verification costs
//! microseconds, and there is nothing to amplify. No salt is needed because the
//! input is already high-entropy and unique, so precomputation is not possible.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use uuid::Uuid;

/// Prefix marking a stored value as a SHA-256 token digest.
const PREFIX: &str = "sha256:";

/// Hashes a token for storage.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    format!("{PREFIX}{}", hex(&digest))
}

/// Verifies a presented token against a stored digest.
///
/// Anything not in the expected form is refused rather than compared literally:
/// falling back to a plaintext comparison for an unconverted row would
/// reintroduce exactly the weakness this replaced. Rows still holding a
/// plaintext or Argon2 value must be converted with the `migrate_git_tokens`
/// binary.
pub fn verify_token(presented: &str, stored: &str) -> bool {
    let Some(expected) = stored.strip_prefix(PREFIX) else {
        tracing::error!(
            "Stored git token is not a SHA-256 digest; run migrate_git_tokens. Refusing to authenticate."
        );
        return false;
    };

    let actual = hex(&Sha256::digest(presented.as_bytes()));
    actual.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// True if a stored value has already been converted.
pub fn is_hashed(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

/// True if a stored value is an Argon2 PHC string, which cannot be converted
/// (the plaintext is unrecoverable) and requires the owner to regenerate.
pub fn is_argon2(stored: &str) -> bool {
    stored.starts_with("$argon2")
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Credentials that could authenticate `basic_username` for `owner/repo`.
///
/// Two shapes are accepted. A row with `user_id` NULL is the pre-existing
/// project-wide token, presented under the owner namespace, which keeps git
/// remotes configured before per-user tokens working. A row with a `user_id` is
/// that user's own credential, presented under their username.
///
/// Scoped to exactly the repository named in the URL: that is what stops a
/// token for one project authorizing another.
pub async fn candidate_credentials(
    pool: &PgPool,
    owner: &str,
    repo: &str,
    basic_username: &str,
) -> Result<Vec<(String, Option<Uuid>)>, sqlx::Error> {
    sqlx::query_as::<_, (String, Option<Uuid>)>(
        r#"SELECT api_token.token, api_token.user_id
           FROM api_token
           JOIN projects ON api_token.project_id = projects.id
           JOIN project_owners ON projects.owner_id = project_owners.id
           LEFT JOIN users ON api_token.user_id = users.id
           WHERE project_owners.name = $1
             AND projects.name = $2
             AND projects.deleted_at IS NULL
             AND (
                   (api_token.user_id IS NULL AND $3 = project_owners.name)
                OR users.username = $3
                 )
        "#,
    )
    .bind(owner)
    .bind(repo)
    .bind(basic_username)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let stored = hash_token("aBcD1234");
        assert!(verify_token("aBcD1234", &stored));
        assert!(!verify_token("aBcD1235", &stored));
    }

    #[test]
    fn is_deterministic_and_prefixed() {
        assert_eq!(hash_token("x"), hash_token("x"));
        assert!(hash_token("x").starts_with("sha256:"));
        // Known-answer check against SHA-256("x").
        assert_eq!(
            hash_token("x"),
            "sha256:2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"
        );
    }

    #[test]
    fn refuses_unconverted_values() {
        // Plaintext must never authenticate, even against itself.
        assert!(!verify_token("plaintok", "plaintok"));
        // An Argon2 row is likewise refused rather than mishandled.
        assert!(!verify_token(
            "tok",
            "$argon2id$v=19$m=19456,t=2,p=1$abc$def"
        ));
        assert!(!verify_token("tok", ""));
    }

    #[test]
    fn classifies_stored_values() {
        assert!(is_hashed(&hash_token("t")));
        assert!(!is_hashed("plaintok"));
        assert!(is_argon2("$argon2id$v=19$m=19456,t=2,p=1$abc$def"));
        assert!(!is_argon2(&hash_token("t")));
    }
}
