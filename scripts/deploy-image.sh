#!/usr/bin/env bash

set -Eeuo pipefail

image_repository="${IMAGE_REPOSITORY:-ghcr.io/pbp-fasilkom-ui/pws}"
service_name="${DEPLOY_SERVICE:-server}"
container_name="${DEPLOY_CONTAINER:-server-pemasak}"
health_url="${DEPLOY_HEALTH_URL:-http://127.0.0.1:8080/health}"

usage() {
  echo "Usage: $0 <commit-sha>" >&2
  echo "The commit SHA must be the full 40-character SHA published by CI." >&2
}

if [[ $# -ne 1 || ! "$1" =~ ^[0-9a-fA-F]{40}$ ]]; then
  usage
  exit 2
fi

image_sha="$1"
image_ref="$image_repository:$image_sha"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_dir="$(cd -- "$script_dir/.." && pwd)"
cd "$repository_dir"

test -f docker-compose.yml
test -f .env

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Deployment stopped: the deployment checkout has local changes." >&2
  exit 1
fi

echo "Pulling $image_ref"
docker pull "$image_ref"

previous_image="$(docker inspect --format '{{.Config.Image}}' "$container_name" 2>/dev/null || true)"

healthcheck() {
  curl --fail --silent --show-error \
    --retry 10 --retry-delay 3 --retry-connrefused \
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
