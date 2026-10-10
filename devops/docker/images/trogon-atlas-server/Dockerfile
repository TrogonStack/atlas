# The workspace root package uses edition 2024 (Rust >= 1.85) and
# trogon-atlas-store uses Duration::from_mins, stable since Rust 1.91.
# Compile on the build platform and cross-link for the target: rustc and
# cargo under QEMU arm64 emulation hang the image job for hours.
FROM --platform=$BUILDPLATFORM rust:1.94-bookworm AS builder
ARG TARGETARCH
ARG BUILDARCH
RUN case "$TARGETARCH" in \
      amd64) triple=x86_64-unknown-linux-musl; gnu=x86_64-linux-gnu; pkg="gcc-x86-64-linux-gnu libc6-dev-amd64-cross" ;; \
      arm64) triple=aarch64-unknown-linux-musl; gnu=aarch64-linux-gnu; pkg="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;; \
      *) echo "unsupported TARGETARCH=$TARGETARCH" >&2; exit 1 ;; \
    esac \
 && if [ "$TARGETARCH" = "$BUILDARCH" ]; then pkg=; fi \
 && apt-get update \
 && apt-get install -y --no-install-recommends pkg-config $pkg \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add "$triple" \
 && echo "$triple" > /target.txt \
 && env_triple=$(echo "$triple" | tr 'a-z-' 'A-Z_') \
 && printf 'export CC_%s=%s-gcc\nexport CARGO_TARGET_%s_LINKER=%s-gcc\n' \
      "$(echo "$triple" | tr - _)" "$gnu" "$env_triple" "$gnu" > /cross.env

# rsworkspace arrives as a bind mount from the repository root context
# instead of a COPY, so the image never embeds a stale snapshot of the
# tree. The mount is read-only, so the build and its target
# directory live under cache mounts instead of inside /src.
RUN --mount=type=bind,source=rsworkspace,target=/src \
    --mount=type=cache,target=/target,id=trogon-atlas-server-target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    . /cross.env \
 && cd /src \
 && cargo build --release --locked -p trogon-atlas-server --bin trogon-atlas-server --target "$(cat /target.txt)" --target-dir /target \
 && mkdir -p /out \
 && cp "/target/$(cat /target.txt)/release/trogon-atlas-server" /out/trogon-atlas-server

FROM alpine:3.20 AS runtime
RUN apk add --no-cache git ca-certificates tini wget \
 && addgroup -S trogon-atlas \
 && adduser -S -G trogon-atlas trogon-atlas \
 && mkdir -p /var/lib/trogon-atlas/mirror \
 && chown -R trogon-atlas:trogon-atlas /var/lib/trogon-atlas
COPY --from=builder /out/trogon-atlas-server /usr/local/bin/trogon-atlas-server
USER trogon-atlas
ENV TROGON_ATLAS_LISTEN=0.0.0.0:50069 \
    TROGON_ATLAS_STORE=nats \
    TROGON_ATLAS_METRICS_LISTEN=0.0.0.0:9100
EXPOSE 50069 9100
HEALTHCHECK --interval=5s --timeout=3s --retries=10 --start-period=10s \
  CMD wget --spider -q http://127.0.0.1:9100/metrics
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/trogon-atlas-server"]
