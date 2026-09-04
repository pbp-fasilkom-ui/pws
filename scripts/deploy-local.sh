#!/usr/bin/env bash

set -Eeuo pipefail

service_name="${DEPLOY_SERVICE:-server}"
container_name="${DEPLOY_CONTAINER:-server-pemasak}"
health_url="${DEPLOY_HEALTH_URL:-http://127.0.0.1:8080/health}"

usage() {
  echo "Usage: $0 [expected-commit-sha]" >&2
  echo "The optional SHA prevents deploying a different master revision." >&2
}

if [[ $# -gt 1 ]]; then
  usage
  exit 2
fi

expected_sha="${1:-}"
if [[ -n "$expected_sha" && ! "$expected_sha" =~ ^[0-9a-fA-F]{40}$ ]]; then
  usage
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_dir="$(cd -- "$script_dir/.." && pwd)"
cd "$repository_dir"

current_branch="$(git branch --show-current)"
if [[ "$current_branch" != "master" ]]; then
  echo "Deployment stopped: expected the deployment checkout to be on master (found $current_branch)." >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Deployment stopped: the deployment checkout has local changes." >&2
  echo "Modified paths:" >&2
  git status --porcelain >&2
  exit 1
fi

echo "Updating deployment checkout"
git pull --ff-only origin master

deployed_sha="$(git rev-parse HEAD)"
if [[ -n "$expected_sha" && "$deployed_sha" != "$expected_sha" ]]; then
  echo "Deployment stopped: expected $expected_sha but master is $deployed_sha." >&2
  exit 1
fi

test -f docker-compose.yml
test -f .env

image_ref="pws-server:$deployed_sha"
previous_image="$(docker inspect --format '{{.Config.Image}}' "$container_name" 2>/dev/null || true)"

healthcheck() {
  curl --fail --silent --show-error \
    --connect-timeout 5 --max-time 10 \
    --retry 20 --retry-delay 3 --retry-max-time 90 \
    --retry-connrefused --retry-all-errors \
    "$health_url"
}

rollback() {
  if [[ -z "$previous_image" ]] || ! docker image inspect "$previous_image" >/dev/null 2>&1; then
    echo "No usable previous image is available for rollback." >&2
    return 1
  fi

  echo "Restoring $previous_image"
  PWS_IMAGE="$previous_image" docker compose up -d --no-build --no-deps "$service_name"
  healthcheck
}

echo "Building $image_ref locally using Docker's build cache"
PWS_IMAGE="$image_ref" docker compose build "$service_name"

# Convert any git token still stored in plaintext, using the image that was
# just built. The server refuses to start while unconverted rows remain -- it
# would otherwise come up healthy and silently reject every push -- so without
# this step the first deploy of that change fails its health check and rolls
# back, and would keep doing so until someone ran the migration by hand.
#
# Idempotent: already-converted rows are skipped, so this is a no-op on every
# subsequent deploy. --hash preserves existing credentials; revoking leaked
# ones is a deliberate, separate --invalidate run.
echo "Converting any plaintext git tokens"
if ! PWS_IMAGE="$image_ref" docker compose run --rm --no-deps \
  --entrypoint /app/migrate_git_tokens "$service_name" --hash; then
  echo "Token migration failed; leaving the running container untouched." >&2
  exit 1
fi

echo "Starting $service_name with $image_ref"
if ! PWS_IMAGE="$image_ref" docker compose up -d --no-build --no-deps "$service_name"; then
  echo "Container restart failed; attempting rollback." >&2
  rollback || true
  exit 1
fi

if ! healthcheck; then
  echo "Health check failed; attempting rollback." >&2
  rollback || true
  exit 1
fi

echo
echo "Deployment completed successfully: $image_ref"
