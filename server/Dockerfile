# Build from the repository root:  docker build -f server/Dockerfile .
FROM rust:1-slim-bookworm AS build
# audiopus_sys (via songbird) needs pkg-config + libopus headers to link;
# without them the crate falls back to a cmake source build.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libopus-dev cmake \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY server ./server
COPY client ./client
RUN cargo build --release --locked -p lawful-overlay-server --bin lawful-overlay-server

FROM debian:bookworm-slim

# Set a non-root user for security
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libopus0 \
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system overlay && adduser --system --ingroup overlay overlay

WORKDIR /app
COPY --from=build /src/target/release/lawful-overlay-server /app/lawful-overlay-server

# Switch to non-root user
USER overlay

EXPOSE 8765

CMD ["/app/lawful-overlay-server"]
