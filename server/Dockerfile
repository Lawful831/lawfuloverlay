# Build from the repository root:  docker build -f server/Dockerfile .
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY server ./server
COPY client ./client
RUN cargo build --release --locked -p lawful-overlay-server --bin lawful-overlay-server

FROM debian:bookworm-slim

# Set a non-root user for security
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system overlay && adduser --system --ingroup overlay overlay

WORKDIR /app
COPY --from=build /src/target/release/lawful-overlay-server /app/lawful-overlay-server

# Switch to non-root user
USER overlay

EXPOSE 8765

CMD ["/app/lawful-overlay-server"]
