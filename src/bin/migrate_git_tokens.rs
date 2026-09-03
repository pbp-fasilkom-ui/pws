//! One-off maintenance task: convert `api_token.token` to a stored SHA-256
//! digest.
//!
//! Tokens were stored in plaintext and logged on every push, so they must be
//! assumed disclosed. Two modes, because the right choice is operational:
//!
//!   --hash        Hash the existing value in place. Every configured git remote
//!                 keeps working, but a token that already leaked stays valid
//!                 until that project regenerates it.
//!
//!   --invalidate  Replace each token with an unusable random digest. This
//!                 closes the leak immediately at the cost of breaking every
//!                 existing git remote: each project owner must then open
//!                 project settings and regenerate, which is the only path that
//!                 can show them a new password.
//!
//! `--invalidate` deliberately does NOT print a replacement credential. An
//! earlier version generated one, hashed it, and dropped the plaintext, so it
//! claimed to issue a password that had never existed anywhere. Printing tokens
//! to an operator's terminal would put live credentials into shell history and
//! deploy logs, so the regenerate endpoint stays the only way to obtain one.
//!
//! With neither flag, reports what it would do and changes nothing.
//!
//! Run it inside the application container, which is on the database's network
//! and has the mounted configuration:
//!
//!   docker compose run --rm --entrypoint /app/migrate_git_tokens server --hash

use pemasak_infra::configuration;
use pemasak_infra::tokens;
use rand::{rngs::StdRng, Rng, SeedableRng};
use sqlx::postgres::PgPoolOptions;
use std::process;
use uuid::Uuid;

const TOKEN_LENGTH: usize = 32;
const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

fn unusable_token() -> String {
    let mut rng = StdRng::from_entropy();
    (0..TOKEN_LENGTH)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let invalidate = args.iter().any(|a| a == "--invalidate");
    let hash_only = args.iter().any(|a| a == "--hash");

    if invalidate && hash_only {
        eprintln!("Pass at most one of --hash and --invalidate.");
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

    let rows: Vec<(Uuid, String)> = match sqlx::query_as(
        r#"SELECT api_token.id, api_token.token
           FROM api_token
           JOIN projects ON api_token.project_id = projects.id
           WHERE projects.deleted_at IS NULL"#,
    )
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
    let mut done = 0usize;
    let mut stuck = 0usize;

    for (id, token) in &rows {
        if tokens::is_hashed(token) {
            done += 1;
            continue;
        }

        // An Argon2 row came from an earlier revision of this branch. The
        // plaintext is unrecoverable, so it cannot be converted -- the owner has
        // to regenerate.
        if tokens::is_argon2(token) {
            stuck += 1;
            println!("api_token {id}: Argon2 value, owner must regenerate (cannot convert)");
            continue;
        }

        plaintext += 1;

        if !invalidate && !hash_only {
            println!("would convert token for api_token {id}");
            continue;
        }

        let stored = if invalidate {
            tokens::hash_token(&unusable_token())
        } else {
            tokens::hash_token(token)
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

        println!(
            "api_token {id}: {}",
            if invalidate {
                "invalidated (owner must regenerate)"
            } else {
                "hashed in place"
            }
        );
    }

    println!(
        "\n{} live rows: {} already converted, {} plaintext{}, {} stuck on Argon2.",
        rows.len(),
        done,
        plaintext,
        if invalidate {
            " invalidated"
        } else if hash_only {
            " hashed"
        } else {
            " (dry run)"
        },
        stuck
    );

    if plaintext > 0 && !invalidate && !hash_only {
        println!(
            "Re-run with --hash to preserve credentials, or --invalidate to revoke leaked ones."
        );
    }

    if invalidate && plaintext > 0 {
        println!(
            "No replacement password is printed by design. Every affected owner must open\n\
             project settings and regenerate -- that endpoint is the only one that shows a\n\
             new password."
        );
    }

    if stuck > 0 {
        println!("{stuck} row(s) hold an Argon2 value that cannot be converted; those projects must regenerate.");
    }
}
