# syntax=docker/dockerfile:1
# The repository toolchain file selects the same Rust version as host builds.
FROM rust:bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY services/sessions/ services/sessions/
COPY services/bee-connection/ services/bee-connection/
COPY services/topics-and-questions/ services/topics-and-questions/
RUN cargo build --locked --release --package sessions-service --bin sessions-service

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home sessions
COPY --from=build /app/target/release/sessions-service /usr/local/bin/sessions
USER sessions
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/sessions"]
