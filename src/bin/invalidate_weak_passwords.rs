//! One-off maintenance task: invalidate every account whose stored password is
//! its own username.
//!
//! SSO-provisioned accounts used to have their password hash derived from the
//! username, so a single guess logged an attacker in as any of them. SSO users
//! are not flagged in the schema, so they cannot be selected in SQL — but the
//! vulnerable set is exactly "the password verifies against the username",
//! which also catches password-registered users who picked that same value.
//!
//! Replaces each match with a hash of 32 random bytes, so password login can no
//! longer succeed for that account. SSO users continue to log in through CAS;
//! password users must be issued a new password out of band.
//!
//! Run with `--apply` to write; without it, reports what it would change.

use argon2::password_hash::{rand_core::OsRng, rand_core::RngCore, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use pemasak_infra::configuration;
use sqlx::postgres::PgPoolOptions;
use std::process;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let apply = std::env::args().any(|arg| arg == "--apply");

    let config = match configuration::get_configuration() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Failed to read configuration: {err:?}");
            process::exit(1);
        }
    };

    let pool = match PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(config.database.timeout))
        .connect_with(config.connection_options())
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("Failed to connect to Postgres: {err:?}");
            process::exit(1);
        }
    };

    let users: Vec<(Uuid, String, String)> = match sqlx::query_as(
        // Match the soft-delete filter the login path applies, so the
        // dry-run count is a truthful blast-radius estimate rather than
        // including rows login can never reach.
        "SELECT id, username, password FROM users WHERE deleted_at IS NULL",
    )
    .fetch_all(&pool)
    .await
    {
        Ok(users) => users,
        Err(err) => {
            eprintln!("Failed to read users: {err:?}");
            process::exit(1);
        }
    };

    // One transaction for the whole run: a failure part-way through previously
    // left some accounts invalidated and others not, with no record of which.
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            eprintln!("Failed to begin transaction: {err:?}");
            process::exit(1);
        }
    };

    let argon2 = Argon2::default();
    let mut affected = 0usize;
    let mut unparseable = 0usize;

    for (id, username, password) in &users {
        let parsed = match PasswordHash::new(password) {
            Ok(parsed) => parsed,
            Err(_) => {
                // Not a valid PHC string; leave it alone but make it visible,
                // since login.rs would panic on this row.
                eprintln!("warning: user {username} ({id}) has an unparseable password hash");
                unparseable += 1;
                continue;
            }
        };

        if argon2
            .verify_password(username.as_bytes(), &parsed)
            .is_err()
        {
            continue;
        }

        affected += 1;

        if !apply {
            println!("would invalidate: {username} ({id})");
            continue;
        }

        let mut replacement = [0u8; 32];
        OsRng.fill_bytes(&mut replacement);
        let salt = SaltString::generate(&mut OsRng);

        let hash = match argon2.hash_password(&replacement, &salt) {
            Ok(hash) => hash.to_string(),
            Err(err) => {
                eprintln!("Failed to hash replacement for {username}: {err:?}");
                process::exit(1);
            }
        };

        if let Err(err) =
            sqlx::query("UPDATE users SET password = $1, updated_at = now() WHERE id = $2")
                .bind(&hash)
                .bind(id)
                .execute(&mut *tx)
                .await
        {
            eprintln!("Failed to update {username}: {err:?}");
            process::exit(1);
        }

        println!("invalidated: {username} ({id})");
    }

    if apply {
        if let Err(err) = tx.commit().await {
            eprintln!("Failed to commit: {err:?}");
            process::exit(1);
        }
    } else if let Err(err) = tx.rollback().await {
        eprintln!("Failed to roll back dry run: {err:?}");
    }

    println!(
        "\n{} users scanned, {} with password equal to username{}, {} unparseable hashes.",
        users.len(),
        affected,
        if apply { " invalidated" } else { " (dry run)" },
        unparseable
    );

    if !apply && affected > 0 {
        println!("Re-run with --apply to write the changes.");
    }
}
