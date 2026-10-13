# syntax=docker/dockerfile:1
FROM rust:bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY services/sessions/ services/sessions/
COPY services/bee-connection/ services/bee-connection/
COPY services/topics-and-questions/ services/topics-and-questions/
RUN cargo build --locked --release --package bee-connection
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/* && useradd --system --uid 10004 --no-create-home bee
COPY --from=build /app/target/release/bee-connection /usr/local/bin/bee-connection
USER bee
ENTRYPOINT ["/usr/local/bin/bee-connection", "--worker"]
