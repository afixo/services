# syntax=docker/dockerfile:1.7
#
# One Dockerfile for every service: `docker build --build-arg SERVICE=gateway .`
# cargo-chef caches the dependency build across services and across commits.

ARG RUST_VERSION=1.96

# ---------------------------------------------------------------- toolchain --
# libprotobuf-dev ships the well-known types (google/protobuf/*.proto) that
# protobuf-compiler alone does not; crates/proto imports timestamp.proto.
FROM rust:${RUST_VERSION}-bookworm AS chef
RUN apt-get update \
 && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev \
 && rm -rf /var/lib/apt/lists/* \
 && cargo install cargo-chef --locked
WORKDIR /app

# ------------------------------------------------------------------ recipe --
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ------------------------------------------------------------------- build --
FROM chef AS builder
ARG SERVICE
COPY --from=planner /app/recipe.json recipe.json
# Dependencies only — cached until Cargo.lock changes.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p afixo-${SERVICE} \
 && cp target/release/afixo-${SERVICE} /usr/local/bin/service

# ----------------------------------------------------------------- runtime --
# distroless cc: glibc + libgcc, no shell, no package manager, runs as nonroot.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
ARG SERVICE
LABEL org.opencontainers.image.source="https://github.com/afixo/services" \
      org.opencontainers.image.title="afixo-${SERVICE}"
COPY --from=builder /usr/local/bin/service /usr/local/bin/service
USER nonroot:nonroot
ENV RUST_LOG=info LOG_FORMAT=json
ENTRYPOINT ["/usr/local/bin/service"]
