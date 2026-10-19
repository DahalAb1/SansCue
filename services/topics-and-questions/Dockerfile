# syntax=docker/dockerfile:1
FROM rust:bookworm AS build
WORKDIR /app
ENV CARGO_BUILD_JOBS=1
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY services/sessions/ services/sessions/
COPY services/bee-connection/ services/bee-connection/
COPY services/topics-and-questions/ services/topics-and-questions/
RUN cargo build --locked --release --workspace
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl libssl3 && rm -rf /var/lib/apt/lists/* && useradd --system --uid 10003 --no-create-home topics
COPY --from=build /app/target/release/topics-and-questions /usr/local/bin/topics
USER topics
EXPOSE 3002
ENTRYPOINT ["/usr/local/bin/topics"]
