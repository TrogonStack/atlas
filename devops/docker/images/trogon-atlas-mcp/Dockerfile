# The workspace root package uses edition 2024 (Rust >= 1.85) and
# trogon-atlas-store uses Duration::from_mins, stable since Rust 1.91.
FROM rust:1.94-bookworm AS builder
ARG TARGETARCH
RUN apt-get update \
 && apt-get install -y --no-install-recommends musl-tools pkg-config \
 && rm -rf /var/lib/apt/lists/*
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl > /target.txt ;; \
      arm64) echo aarch64-unknown-linux-musl > /target.txt ;; \
      *) echo "unsupported TARGETARCH=$TARGETARCH" >&2; exit 1 ;; \
    esac
RUN rustup target add "$(cat /target.txt)"

# rsworkspace arrives as a bind mount from the repository root context
# instead of a COPY, so the image never embeds a stale snapshot of the
# tree. The mount is read-only, so the build and its target
# directory live under cache mounts instead of inside /src.
RUN --mount=type=bind,source=rsworkspace,target=/src \
    --mount=type=cache,target=/target,id=trogon-atlas-mcp-target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cd /src \
 && cargo build --release --locked -p trogon-atlas-mcp --bin trogon-atlas-mcp --target "$(cat /target.txt)" --target-dir /target \
 && mkdir -p /out \
 && cp "/target/$(cat /target.txt)/release/trogon-atlas-mcp" /out/trogon-atlas-mcp

FROM alpine:3.20 AS runtime
RUN apk add --no-cache ca-certificates tini \
 && addgroup -S trogon-atlas \
 && adduser -S -G trogon-atlas trogon-atlas
COPY --from=builder /out/trogon-atlas-mcp /usr/local/bin/trogon-atlas-mcp
USER trogon-atlas
ENV TROGON_ATLAS_GRPC=http://trogon-atlas-server:50069
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/trogon-atlas-mcp"]
