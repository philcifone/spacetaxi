# Build stage
FROM rust:1.83-slim-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY cli/Cargo.toml ./cli/
COPY server/Cargo.toml ./server/
COPY shared/Cargo.toml ./shared/

# Create dummy source files for caching dependencies
RUN mkdir -p cli/src server/src shared/src && \
    echo "fn main() {}" > cli/src/main.rs && \
    echo "fn main() {}" > server/src/main.rs && \
    echo "" > shared/src/lib.rs

# Build dependencies
RUN cargo build --release -p spacetaxi-server && \
    rm -rf cli/src server/src shared/src

# Copy actual source code
COPY cli ./cli
COPY server ./server
COPY shared ./shared
COPY web/dist ./web/dist

# Build the server
RUN touch cli/src/main.rs server/src/main.rs shared/src/lib.rs && \
    cargo build --release -p spacetaxi-server

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/spacetaxi-server /usr/local/bin/

# Create data directory
RUN mkdir -p /data/files /data/chunks

ENV SPACETAXI_DATA_DIR=/data
ENV SPACETAXI_HOST=0.0.0.0
ENV SPACETAXI_PORT=3000
ENV RUST_LOG=spacetaxi_server=info,tower_http=info

EXPOSE 3000

VOLUME ["/data"]

CMD ["spacetaxi-server"]
