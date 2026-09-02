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

The `CI` GitHub Actions workflow runs backend checks, UI and documentation builds, and a Docker build for pull requests targeting `master`. Clippy, UI lint, and documentation typechecking currently run as advisory checks because the existing `master` branch has baseline findings. The separate `CD` workflow deploys `master` to the production Docker host over SSH.

The workflow rebuilds the `server` image on the deployment host, allowing Docker to reuse its local dependency layers. To enable this, create a `production` environment and add `DEPLOY_ENABLED=true` as an environment variable. Add these additional environment variables:

- `DEPLOY_HOST`: production server hostname or IP
- `DEPLOY_USER`: SSH user
- `DEPLOY_PATH`: directory containing this repository's `docker-compose.yml` and `.env`

Add these environment secrets:

- `DEPLOY_SSH_KEY`: private SSH key for that user

The deployment host must have Git, Docker Compose, `curl`, and the repository checked out on the `master` branch. The workflow refuses to deploy when the checkout has local changes, fast-forwards it from `origin/master`, rebuilds only the `server` service with a commit-specific local image tag, recreates only that service, and verifies `/health`. If the health check fails, it attempts to restore the previous server image.

### Setting up the docusaurus

1. Install nodejs and pnpm.
2. Go to `docs-ui` folder.
3. Run `pnpm install`.
4. Run `pnpm start` to start the docusaurus.
5. Add folder in `docs` folder to add new documentation.
6. Access the docusaurus in `localhost:4000` to access the documentation.
7. To deploy use docker compose by running `docker compose up docs -d` or it also run on default `docker compose up -d` to deploy all service.
8. The docs will be available in `docs.[domain]` domain. The domain is configured in the `configuration.yml` file.
