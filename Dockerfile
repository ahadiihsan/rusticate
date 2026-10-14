# syntax=docker/dockerfile:1.7
# =============================================================================
# One Dockerfile, one image per binary (OPS-006):
#   docker build --target api    -t rusticate-api .
#   docker build --target worker -t rusticate-worker .
#
# * cargo-chef caches the dependency build: code-only changes rebuild in
#   seconds, not minutes.
# * SQLX_OFFLINE: the build never needs a database (.sqlx is committed).
# * Runtime is distroless (no shell, no package manager), non-root, and the
#   binary is the only file added.
# =============================================================================

ARG RUST_IMAGE=rust:1.99-slim-bookworm
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian12:nonroot

FROM ${RUST_IMAGE} AS chef
RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ENV SQLX_OFFLINE=true \
    CARGO_INCREMENTAL=0
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json -p api -p worker
COPY . .
RUN cargo build --release --locked -p api -p worker

FROM ${RUNTIME_IMAGE} AS api
COPY --from=builder /src/target/release/api /usr/local/bin/api
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/api"]
CMD ["serve"]

FROM ${RUNTIME_IMAGE} AS worker
COPY --from=builder /src/target/release/worker /usr/local/bin/worker
USER nonroot:nonroot
EXPOSE 8081
ENTRYPOINT ["/usr/local/bin/worker"]
