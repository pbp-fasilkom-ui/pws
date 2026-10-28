#!/bin/sh
set -eu

serverdir=/data/server

if [ ! -f "$serverdir/.serverversion" ]; then
  # Every project container can reach this port, and devpi's root user can
  # repoint root/pypi at any upstream -- serving every later build whatever
  # packages it likes. devpi-init would otherwise leave root's password empty.
  # A random one nobody keeps leaves root unusable over HTTP; reset it with
  # devpi-passwd inside the container if it is ever needed.
  devpi-init --serverdir "$serverdir" \
    --root-passwd "$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
fi

# --restrict-modify root: anonymous clients cannot create users or indexes,
# so the only thing this server does for them is mirror PyPI.
exec devpi-server --serverdir "$serverdir" \
  --host 0.0.0.0 --port 3141 \
  --restrict-modify root
