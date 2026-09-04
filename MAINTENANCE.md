# Maintenance plan: framework upgrade

The dependency audit reports 8 advisories. Seven of them are one piece of
maintenance debt, not seven separate bugs: the web framework is roughly three
years behind, and the fixes are chained behind it.

None are urgent. Each needs an improbable precondition (a multi-gigabyte
payload, an active network interception, or access already restricted to the
control plane). This is scheduled maintenance, best done in a gap between
course intakes rather than against a live cohort.

## Why it is one move, not five

```
sqlx 0.8          needs  axum_session >= 0.15
axum_session      needs  axum 0.8
axum 0.8          needs  hyper 1.x
h2 0.4.16         needs  hyper 1.x
rustls 0.23       needs  reqwest 0.12  (and sqlx off rustls 0.21)
```

Attempting any of them alone fails outright: both `sqlx-sqlite` versions declare
`links = "sqlite3"`, so cargo rejects a graph containing sqlx 0.7 and 0.8.
`reqwest` 0.12 on its own was tried and reverted — it adds rustls 0.22 beside
sqlx's 0.21 and produces four more advisories than it removes.

## Phase 0 — remove leptos (do this first, independently valuable)

`leptos` is a direct dependency used in exactly **3 files, 9 call sites**, and
only to render one-line `<h1>` error pages in `create_project_owner.rs` and
`update_project_owner.rs`. The third hit is a log-filter string in
`telemetry.rs:79` and is not a real use.

Replacing those with the plain JSON error responses used everywhere else in the
codebase removes `leptos`, `leptos_dom` **and `server_fn`** — and `server_fn` is
one of the two consumers of the old `reqwest 0.11` chain.

Doing this first avoids a leptos 0.5 -> 0.8 migration, which would otherwise be
the single largest piece of work here. `update_project_owner` is additionally a
no-op stub that returns 204 without doing anything, so it may be deletable
outright.

Low risk, no framework interaction, can be merged any time.

## Phase 1 — hyper 1.x + axum 0.8 (the bulk of the work)

- `hyper` 0.14 -> 1.x
- `axum` 0.6 -> 0.8, `axum-extra` to match
- `tower-http` 0.4 -> 0.6
- `axum_session` 0.6.1 -> 0.21, `axum_session_auth` to match

Expect real work, not just version bumps:

- `hyper::Server::bind().serve()` is gone; the entry point in `startup.rs` moves
  to `axum::serve`
- middleware signatures changed — `git.rs::basic_auth`, `auth::auth` and the
  rate limiter in `rate_limit.rs` all use `Next<B>` and will need rewriting
- the generic body parameter `<B>` is removed from axum's middleware types
- the WebSocket handler in `web_terminal.rs` will need updating
- `axum_session` is 15 minor versions behind; its config API differs, so the
  session key wiring added recently (`with_key` + `with_database_key` +
  `SecurityMode::PerSession`) must be re-established and re-verified. This is
  the piece that silently 500s everything if it is wrong, so it needs an
  explicit test.
- both `axum_session` 0.6.1 and `axum_session_auth` 0.6.0 are **yanked**, which
  is a second reason to move

Closes: RUSTSEC-2026-0258 (h2).

## Phase 2 — sqlx 0.8

Unblocked once `axum_session` no longer pins sqlx 0.7. Requires regenerating the
`.sqlx` offline query cache against a live database (`cargo sqlx prepare`).

Closes: RUSTSEC-2024-0363.

## Phase 3 — reqwest 0.12 / rustls 0.23

Safe only once sqlx has stopped pulling rustls 0.21, otherwise two rustls
versions coexist and the advisory count rises.

Closes: RUSTSEC-2026-0098, -0099, -0104 (rustls-webpki).

## Phase 4 — url / idna

Closes: RUSTSEC-2024-0421.

## End state

8 advisories -> 1. The remaining one is `rsa` RUSTSEC-2023-0071, which has no
patch upstream and is not in the build graph at all (sqlx's MySQL support is
disabled), so it stays permanently ignored with that justification recorded.

## Verification, given there is no staging environment

The mitigations that make this survivable:

- `deploy-local.sh` health-checks and rolls back to the previous image
- the startup gates refuse to boot on an unmigrated database rather than
  starting healthy and failing silently
- `tests/authz.rs` and `tests/git_tokens.rs` exercise the authorization and
  credential queries against a real Postgres in CI

What is still needed before the phase 1 merge:

- a test that a session-bearing request succeeds, since a misconfigured session
  layer returns 200 on `/health` while 500-ing every real request
- a manual pass over login, project view, env edit, build log, web terminal and
  a git push
- do it in a gap between intakes, not mid-course

## Status

Not started. Scheduled for the next gap between course intakes.

Re-review date recorded in `.cargo/audit.toml` is **2026-12-01**, so the
accepted advisories cannot quietly become permanent. If this has not started by
then, the baseline should be revisited rather than extended.

## Related

- `.cargo/audit.toml` — the reviewed baseline, one justification per advisory
- `src/lib.rs` — the equivalent Clippy baseline, same burn-down intent
- The `Audit` workflow runs weekly on a schedule, so a newly published advisory
  surfaces without a push
