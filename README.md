# Pemasak-infra

PaaS (Platform as a Service) to help sustain application deployment in Fasilkom UI.

## Developer setup guide

Make sure your user has docker access by running `groups` and check if docker is in it. If not run `sudo usermod -aG docker $USER newgrp docker` or run the app with sudo.

### Using nix (recomended)

1. Run `./script/install-nix.sh` make sure not using root but the user have root privileges
2. Close terminal and open it again to get new session
3. Run `direnv allow`
4. Copy `configuration.example.yml` to `configuration.yml` and change the config
5. Run `./scripts/env.sh > .env`
6. Run `docker compose up -d`
7. Run `./scripts/apply.sh`
8. Run `nix run .#dev` this will talke a while

### Not Using nix

1. Install rust via rustup `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. Install tool by running `./scripts/install-tools.sh`
3. Install `jq` and `yj`
4. Copy `configuration.example.yml` to `configuration.yml` and change the config
5. Run `./scripts/env.sh > .env`
6. Run `docker compose up -d`
7. Run `./scripts/apply.sh`
8. Run `RUST_LOG=info cargo run` this will talke a while

### Sqlx for database

After writing code. Before commit, run `cargo sqlx prepare`. To do that automatically you can enable the git hook by running `ln -sf ../../scripts/pre-commit ./.git/hooks`

## Server Maintainer Guide

0. Prerequisite knowledge. need to know docker, linux admin, caddy well.
   docker knowledge including debugging docker runtime and navigating with the cli.
   linux administration used for debugging if the storage ran out, increasing the file open limits.
   caddy to debug the reverse proxy.
1. Make sure docker is installed. The server uses docker build to build the image and to run the image.
2. Change the docker daemon file in `/etc/docker/daemon.json` to

```json
{
  "metrics-addr": "127.0.0.1:9323",
  "bip": "172.32.0.1/12",
  "default-address-pools": [
    {
      "base": "172.17.0.0/12",
      "size": 24
    },
    {
      "base": "192.168.0.0/16",
      "size": 24
    }
  ]
}
```

to make sure the project won't ran out of ip. This is important for deploying a lot of project since the default settings only give you 31 networks.

3. Make sure the user have docker group access by running `groups` and check if docker is in it. If not run `sudo usermod -aG docker $USER && newgrp docker`  .
   The application uses docker API to access the docker daemon. Make sure the user have access to the docker daemon.
4. Increase the file open limit size in `/etc/security/limits.conf` to large number like `65536` and add `fs.file-max = 65536` to `/etc/sysctl.conf` file.  
   This is important to make sure the server can handle a lot of file open at the same time when deploying a lot of project.
5. Copy `configuration.example.yml` to `configuration.yml` and change the `configuration.yml` `application.bodylimit` to large value like 500mb or 1gb to allow large file upload.
   The bodylimit is important to mitigate git error `unexpected disconnect while reading sideband packet`.
6. Copy `.env.example` in `ui` folder to `.env` and change the `VITE_API_URL` to the server ip.
7. Run `./scripts/env.sh > .env` to generate the environment variable.
8. Run `docker compose up -d` to start the server. This will take a while.

### Common Issue for deployment

1. If the deployment can't run, add procfile to the root of the project. For django its

```procfile
release: python manage.py collectstatic --noinput && python manage.py migrate --noinput
web: gunicorn [project_name].wsgi
```

and make sure have `gunicorn` in the `requirements.txt` file.

2. Push the branch you want to deploy. PWS deploys the branch included in the push and supports both `main` and `master`.

### CI/CD Guide

The `CI` GitHub Actions workflow runs backend checks, UI and documentation builds, and a Docker build for pull requests targeting `master`. Clippy blocks the build; its pre-existing findings are frozen via a crate-level allow list in `src/lib.rs`, which should be burned down over time. UI lint and documentation typechecking remain advisory because the UI has a baseline of style errors that cannot be frozen without disabling most of the useful rules. A `cargo-audit` job reports Rust dependency advisories, and third-party actions are pinned to commit SHAs with Dependabot keeping them current. On pushes to `master`, the container job publishes an immutable image to GHCR tagged with the commit SHA:

`ghcr.io/pbp-fasilkom-ui/pws:<commit-sha>`

The `CD` workflow can be triggered manually or automatically after the `CI` workflow succeeds for a push to `master`. It uses a GitHub-hosted runner, connects to the production network through OpenVPN, and deploys over SSH. It does not use a self-hosted runner or depend on GHCR for deployment.

For manual deployment, make sure the production VM has Docker Compose, `curl`, and this repository checked out on the `master` branch with no local changes. The local-build deployment script updates the checkout before deploying:

```bash
cd /home/admin/pws
./scripts/deploy-local.sh [expected-commit-sha]
```

The script verifies the checkout, fast-forward pulls `origin/master`, builds an image locally using Docker's build cache, recreates only the `server` service, verifies `/health`, and attempts to restore the previous image if the health check fails. The optional commit SHA prevents deploying a different `master` revision than the one selected by CD. The VM does not need GHCR credentials for this deployment flow.

### Security-related deployment requirements

Recent hardening added a few prerequisites. A deployment that skips them will
fail to start or will lock users out.

**Required configuration.** `configuration.yml` must now set:

- `database.password` — there is no longer a default, so the application
  refuses to start without one. Previously it silently fell back to a weak
  value.
- `auth.key` — at least 64 bytes, used to encrypt the session cookie. If it is
  absent a random key is generated at startup, which is safe but logs users out
  on every restart. Generate one with `openssl rand -base64 64`.
- `auth.secure: true` for any deployment reachable over HTTPS.

**Migrations.** Apply `migration.sql`, which adds a unique index on
`project_owners.name`. It removes unreferenced duplicate rows first; if it
fails, resolve the remaining duplicates by hand before retrying.

**One-off maintenance tasks.** Two binaries, both dry-run by default. They are
shipped inside the application image and must run there: the database is
reachable only from the control-plane network, and they read the
`configuration.yml` mounted at `/app/configuration.yml`. Running them from the
VM host will not reach the database.

```bash
# Git push tokens were stored in plaintext and logged on every push.
#   --hash        keeps every credential working, but a token that already
#                 leaked stays valid until that project regenerates it.
#   --invalidate  revokes them immediately, which breaks every configured git
#                 remote until each owner regenerates from project settings.
#                 No replacement password is printed -- the regenerate endpoint
#                 is the only thing that can show one.
docker compose run --rm --entrypoint /app/migrate_git_tokens server --hash

# SSO accounts used to have their password derived from their username.
# Reports affected accounts; --apply invalidates those hashes.
docker compose run --rm --entrypoint /app/invalidate_weak_passwords server --apply
```

`scripts/deploy-local.sh` runs `migrate_git_tokens --hash` automatically after
building the image and before starting the container, so a normal deploy needs
no manual step. It is idempotent — already-converted rows are skipped — and a
failure there aborts the deploy without touching the running container.

Run it by hand only to revoke leaked credentials (`--invalidate`), or to inspect
before deploying. The server counts unconverted rows at startup and refuses to
boot while any remain, rather than starting healthy and silently rejecting every
push.

Note `invalidate_weak_passwords` also catches password-registered users who
chose their username as their password, and there is no self-service password
reset in this codebase. Read the dry-run output before applying: SSO users
recover by signing in through CAS, but a password-only account caught by it has
to be reset with direct SQL.

**Traefik dashboard and API.** The API is bound to the Traefik container's own
loopback (`--entrypoints.traefik.address=127.0.0.1:8080`). Publishing it on the
host's loopback was not sufficient: a published port constrains only host
access, and Traefik sits on both docker networks, so every student container
could reach `traefik-pemasak:8080` and read the full routing table.

Reach it from the VM with the IPv4 literal -- `localhost` resolves to `::1`,
which is not bound:

```bash
docker exec traefik-pemasak wget -qO- http://127.0.0.1:8080/api/rawdata
docker exec traefik-pemasak wget -qO- http://127.0.0.1:8080/api/overview
```

Verified: refused from any other container, and ports 80/443 routing is
unaffected. Viewing the dashboard in a browser now requires temporarily
republishing the port (a compose override), because a port cannot be published
into a namespace bound to loopback.

**Student images and container hardening.** Project containers run with
`cap_drop: ALL`, a single `cap_add: NET_BIND_SERVICE` so a server can bind port
80, and `no-new-privileges`. Images that rely on `gosu`/`su-exec` to step down
from root at startup, or that need a capability beyond binding a low port, will
fail to start and must be adjusted -- typically by running as a non-root `USER`
in the Dockerfile rather than dropping privileges at runtime. The generated
Django image is unaffected. This is a deliberate trade: it is the containment
that keeps a compromised project off the host.

These options are applied when a container is *created*, so they take effect
per project on its next push, not at deploy time -- `deploy-local.sh` recreates
only the `server` service. Already-running project containers keep running
unhardened until their owner pushes again.

That gradual rollout is deliberate. Recreating every container at once was
considered and rejected: rebuilding each project risks a build that succeeded
weeks ago failing today on dependency resolution, which would leave that
student's app down. Verified against the running fleet that the generated image
starts correctly under these options (gunicorn still binds port 80 with only
NET_BIND_SERVICE added back), and that no project ships its own Dockerfile, so
nothing relies on a setuid entrypoint.

The residual is a project whose owner stops pushing: its container stays as it
was. If a specific project needs the hardening sooner, redeploy it from the
project page rather than recreating the whole fleet.

**Check for names the new validation rejects.** Owner and project names are now
refused if they start with a dot or contain `..`, since both become filesystem
paths. Such names were previously accepted. Run this before deploying; any rows
it returns belong to accounts that would lose git access and need renaming
first:

```sql
SELECT 'user' AS kind, username AS name FROM users
  WHERE username LIKE '.%' OR username LIKE '%..%'
UNION ALL
SELECT 'owner', name FROM project_owners
  WHERE name LIKE '.%' OR name LIKE '%..%'
UNION ALL
SELECT 'project', name FROM projects
  WHERE name LIKE '.%' OR name LIKE '%..%';
```

**Access to admin interfaces changed.** Postgres, Grafana, Prometheus, the app
port and the Traefik dashboard are now published on loopback only, and Portainer
has no public route at all. Reach them over an SSH tunnel:

```bash
ssh -L 9000:127.0.0.1:9000 -L 7070:127.0.0.1:7070 -L 3000:127.0.0.1:3000 admin@<host>
```

Grafana also remains available at `grafana.<domain>` through Traefik.

**Repository settings to confirm.** These are not visible in the repository and
should be verified directly: the `production` environment should require
reviewers and restrict deployments to `master`, and `master` should be
branch-protected. Without them, a pull request can reference the `production`
environment and read `DEPLOY_SSH_KEY` and `OPENVPN_CONFIG`.

### Setting up the docusaurus

1. Install nodejs and pnpm.
2. Go to `docs-ui` folder.
3. Run `pnpm install`.
4. Run `pnpm start` to start the docusaurus.
5. Add folder in `docs` folder to add new documentation.
6. Access the docusaurus in `localhost:4000` to access the documentation.
7. To deploy use docker compose by running `docker compose up docs -d` or it also run on default `docker compose up -d` to deploy all service.
8. The docs will be available in `docs.[domain]` domain. The domain is configured in the `configuration.yml` file.
