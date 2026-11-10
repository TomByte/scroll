# Build from the repository root, which holds the Cargo workspace:
#   docker build -f crates/scroll-server/Dockerfile -t scroll-server .

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p scroll-server \
    && cp target/release/scroll-server /usr/local/bin/scroll-server

FROM debian:bookworm-slim
# TLS and trusted roots for talking to Google when sign-in with Google is on.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin scroll
COPY --from=build /usr/local/bin/scroll-server /usr/local/bin/scroll-server
USER scroll
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/scroll-server"]
