# The workspace root package uses edition 2024 (Rust >= 1.85) and
# trogon-atlas-store uses Duration::from_mins, stable since Rust 1.91.
# Compile on the build platform and cross-link for the target: rustc and
# cargo under QEMU arm64 emulation hang the image job for hours.
FROM --platform=$BUILDPLATFORM bufbuild/buf:1.75.0@sha256:adf82c7170be6629ec81dc907921ac1699407cd11c4f7fd7521b13c74152f849 AS buf
FROM --platform=$BUILDPLATFORM rust:1.99-bookworm@sha256:e12566d5abc17a0da741628782cdb504b68b8961f5bfcf677936bf3e8b32de29 AS builder
COPY --from=buf /usr/local/bin/buf /usr/local/bin/buf
ARG TARGETARCH
ARG BUILDARCH
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
# hadolint ignore=SC2086
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
# directory live under cache mounts instead of inside /src. Both platforms
# build at once on the build host, and cargo's download lock sits outside the
# registry mount, so each platform gets its own registry cache.
WORKDIR /src/rsworkspace
RUN --mount=type=bind,source=rsworkspace,target=/src/rsworkspace \
    --mount=type=bind,source=proto,target=/src/proto \
    --mount=type=cache,target=/target,id=trogon-atlas-server-target \
    --mount=type=cache,target=/usr/local/cargo/registry,id=trogon-atlas-server-cargo-registry-${TARGETARCH} \
    . /cross.env \
 && cargo build --release --locked -p trogon-atlas-server --bin trogon-atlas-server --target "$(cat /target.txt)" --target-dir /target \
 && mkdir -p /out \
 && cp "/target/$(cat /target.txt)/release/trogon-atlas-server" /out/trogon-atlas-server

FROM alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS runtime
RUN apk upgrade --no-cache \
 && apk add --no-cache git ca-certificates tini wget \
 && addgroup -g 101 -S trogon-atlas \
 && adduser -S -u 100 -G trogon-atlas trogon-atlas \
 && mkdir -p /var/lib/trogon-atlas/mirror \
 && chown -R trogon-atlas:trogon-atlas /var/lib/trogon-atlas
COPY --from=builder /out/trogon-atlas-server /usr/local/bin/trogon-atlas-server
USER 100:101
ENV TROGON_ATLAS_LISTEN=0.0.0.0:50069 \
    TROGON_ATLAS_STORE=nats
EXPOSE 50069
HEALTHCHECK --interval=5s --timeout=3s --retries=10 --start-period=10s \
  CMD ["/bin/sh", "-c", "if [ -n \"$TROGON_ATLAS_TLS_CERT\" ]; then exec wget --spider -q --no-check-certificate https://127.0.0.1:50069/-/liveness; else exec wget --spider -q http://127.0.0.1:50069/-/liveness; fi"]
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/trogon-atlas-server"]
