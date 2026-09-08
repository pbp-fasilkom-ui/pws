#!/usr/bin/env bash

set -Eeuo pipefail

destination="10.119.106.139"
gateway="10.119.79.254"
interface="ens19"
source_address="10.119.79.101"
target_config="/etc/netplan/60-pws-database-route.yaml"

usage() {
  echo "Usage: $0 [--check]" >&2
}

if [[ $# -gt 1 ]]; then
  usage
  exit 2
fi

mode="${1:-apply}"
if [[ "$mode" != "apply" && "$mode" != "--check" ]]; then
  usage
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source_config="$script_dir/../config/netplan/60-pws-database-route.yaml"

route_matches() {
  local route
  route="$(ip -4 route get "$destination" 2>/dev/null)" || return 1

  [[ "$route" == *"via $gateway"* ]] \
    && [[ "$route" == *"dev $interface"* ]] \
    && [[ "$route" == *"src $source_address"* ]]
}

if [[ "$mode" == "--check" ]]; then
  if route_matches; then
    echo "Database route is active: $(ip -4 route get "$destination")"
    exit 0
  fi

  echo "Database route is not active." >&2
  ip -4 route get "$destination" >&2 || true
  exit 1
fi

for command in ip netplan sudo; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "Required command is unavailable: $command" >&2
    exit 1
  }
done

[[ -f "$source_config" ]] || {
  echo "Netplan source file is missing: $source_config" >&2
  exit 1
}

ip -4 address show dev "$interface" \
  | grep -Fq "inet $source_address/" || {
    echo "$interface does not have the expected address $source_address." >&2
    exit 1
  }

if sudo -n test -f "$target_config" \
  && sudo -n cmp -s "$source_config" "$target_config" \
  && route_matches; then
  echo "Database route is already configured."
  exit 0
fi

backup_file="$(mktemp)"
had_previous_config=false
if sudo -n test -f "$target_config"; then
  sudo -n cat "$target_config" >"$backup_file"
  had_previous_config=true
fi

cleanup() {
  rm -f "$backup_file"
}
trap cleanup EXIT

restore_config() {
  echo "Restoring the previous Netplan configuration." >&2
  if [[ "$had_previous_config" == true ]]; then
    sudo -n install -o root -g root -m 600 "$backup_file" "$target_config"
  else
    sudo -n rm -f "$target_config"
  fi
  sudo -n netplan generate
  sudo -n netplan apply
}

sudo -n install -o root -g root -m 600 "$source_config" "$target_config"

if ! sudo -n netplan generate; then
  restore_config
  echo "Netplan validation failed; the route was not installed." >&2
  exit 1
fi

if ! sudo -n netplan apply; then
  restore_config
  echo "Netplan apply failed; the previous configuration was restored." >&2
  exit 1
fi

if ! route_matches; then
  restore_config
  echo "The applied route does not use the expected gateway; the previous configuration was restored." >&2
  exit 1
fi

echo "Database route installed: $(ip -4 route get "$destination")"
