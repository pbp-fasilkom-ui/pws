//! One-off maintenance task: move `api_token.token` from plaintext to Argon2.
//!
//! Tokens were stored in plaintext and logged on every push, so they must be
//! assumed disclosed. Two modes, because the right choice is an operational
//! one:
//!
//!   --hash    Hash the existing value in place. Every student's configured git
//!             remote keeps working, but a token that already leaked stays
//!             valid until that project regenerates it.
//!
//!   --rotate  Issue a new token and hash that. Closes the leak immediately,
//!             but every existing git remote stops authenticating until its
//!             owner copies the new password from the project settings page.
//!
//! With neither flag, reports what it would do and changes nothing.

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher};
use pemasak_infra::configuration;
use rand::{rngs::StdRng, Rng, SeedableRng};
use sqlx::postgres::PgPoolOptions;
use std::process;
use uuid::Uuid;

const TOKEN_LENGTH: usize = 32;
const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

fn generate_token() -> String {
    let mut rng = StdRng::from_entropy();
    (0..TOKEN_LENGTH)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect()
}

fn hash(value: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .expect("failed to hash token")
        .to_string()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rotate = args.iter().any(|a| a == "--rotate");
    let hash_only = args.iter().any(|a| a == "--hash");

    if rotate && hash_only {
        eprintln!("Pass at most one of --hash and --rotate.");
        process::exit(2);
    }

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

    let rows: Vec<(Uuid, String)> = match sqlx::query_as("SELECT id, token FROM api_token")
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => rows,
        Err(err) => {
            eprintln!("Failed to read api_token: {err:?}");
            process::exit(1);
        }
    };

    let mut plaintext = 0usize;
    let mut already_hashed = 0usize;

    for (id, token) in &rows {
        if PasswordHash::new(token).is_ok() {
            already_hashed += 1;
            continue;
        }

        plaintext += 1;

        if !rotate && !hash_only {
            println!("would convert token for api_token {id}");
            continue;
        }

        let (stored, note) = if rotate {
            let fresh = generate_token();
            (hash(&fresh), "rotated (owner must copy the new password)")
        } else {
            (hash(token), "hashed in place")
        };

        if let Err(err) =
            sqlx::query("UPDATE api_token SET token = $1, updated_at = now() WHERE id = $2")
                .bind(&stored)
                .bind(id)
                .execute(&pool)
                .await
        {
            eprintln!("Failed to update api_token {id}: {err:?}");
            process::exit(1);
        }

        println!("api_token {id}: {note}");
    }

    println!(
        "\n{} rows: {} already hashed, {} plaintext{}.",
        rows.len(),
        already_hashed,
        plaintext,
        if rotate {
            " rotated"
        } else if hash_only {
            " hashed"
        } else {
            " (dry run)"
        }
    );

    if plaintext > 0 && !rotate && !hash_only {
        println!(
            "Re-run with --hash to preserve credentials, or --rotate to invalidate leaked ones."
        );
    }

    if rotate && plaintext > 0 {
        println!("Every affected project must copy its new git password from project settings.");
    }
}
