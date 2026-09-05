# stage 0: chef
# Pinned to bookworm (glibc 2.36) rather than the floating `rust:1.98.0` tag,
# which now resolves to trixie (glibc 2.41). The binary is linked against the
# builder's glibc and must load on the runtime stage below: an unpinned tag
# silently moves that requirement, which is exactly how the server came to
# reference GLIBC_2.39 and fail to start on an ubuntu:22.04 runtime (2.35).
# Keep this distro no newer than the runtime stage's.
FROM rust:1.98.0-bookworm AS chef
WORKDIR /app
RUN cargo install cargo-chef --version 0.1.60 --locked
RUN apt update

#stage 1: planner
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# stage 2: caching
FROM chef AS cacher
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# stage 3: build
FROM chef AS builder
COPY --from=cacher /app/target target
COPY --from=cacher $CARGO_HOME $CARGO_HOME
RUN curl -sL https://deb.nodesource.com/setup_20.x -o /tmp/nodesource_setup.sh && bash /tmp/nodesource_setup.sh && apt update && apt install -y nodejs
RUN npm install -g pnpm@8.15.9
COPY ui/package.json ui/pnpm-lock.yaml ./ui/
RUN cd ui && pnpm install --frozen-lockfile
COPY . .
RUN cargo build --release

# stage 4: run
# FROM gcr.io/distroless/cc-debian11
# 24.04 provides glibc 2.39, comfortably above the bookworm builder's 2.36.
# 22.04 (2.35) is too old: it cannot load a binary built on any current Rust
# image. If you change either base, check that the runtime's glibc is >= the
# builder's -- nothing in the build fails when it is not, the container just
# exits at startup and the deploy rolls back.
FROM ubuntu:24.04
WORKDIR /app
COPY --from=builder /app/target/release/pemasak-infra /app
# Maintenance tasks. They need the database, which is reachable only from the
# control-plane network, and the configuration mounted at /app/configuration.yml
# -- so they have to run inside this image rather than on the VM host:
#   docker compose run --rm --entrypoint /app/migrate_git_tokens server --hash
COPY --from=builder /app/target/release/migrate_git_tokens /app
COPY --from=builder /app/target/release/invalidate_weak_passwords /app
COPY --from=builder /app/ui/dist /app/ui/dist
RUN apt update && apt install -y libssl-dev ca-certificates git apt-transport-https curl software-properties-common gnupg
RUN curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg
RUN echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null
RUN apt update
RUN apt-cache policy docker-ce
RUN apt install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
# Fail the build here rather than at deploy time. A binary built against a
# newer glibc than this stage provides still builds, still copies in, and only
# dies when the container starts -- which surfaces as a failed health check and
# a production rollback. `ldd` reports the mismatch on stdout and exits 0, so
# the exit code cannot be used; grep for the message instead. This also catches
# an ordinary missing shared library.
RUN set -e; for b in pemasak-infra migrate_git_tokens invalidate_weak_passwords; do \
      if ldd "/app/$b" 2>&1 | grep -q "not found"; then \
        echo "ERROR: /app/$b has unresolved libraries on this runtime:" >&2; \
        ldd "/app/$b" >&2; \
        exit 1; \
      fi; \
    done

CMD ["./pemasak-infra"]
