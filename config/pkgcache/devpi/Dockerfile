# Caching PyPI mirror for project builds. See pkgcache-pypi in
# docker-compose.yml.
FROM python:3.13-slim

RUN pip install --no-cache-dir devpi-server==6.21.0

COPY entrypoint.sh /entrypoint.sh

# Unprivileged: the mirror only needs its data directory.
RUN useradd --system --home-dir /data devpi \
    && mkdir /data && chown devpi /data \
    && chmod 0755 /entrypoint.sh
USER devpi

EXPOSE 3141
ENTRYPOINT ["/entrypoint.sh"]
