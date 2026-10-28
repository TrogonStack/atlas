# The workspace root package uses edition 2024 (Rust >= 1.85) and
# trogon-atlas-store uses Duration::from_mins, stable since Rust 1.91.
# Compile on the build platform and cross-link for the target: rustc and
# cargo under QEMU arm64 emulation hang the image job for hours.
FROM --platform=$BUILDPLATFORM bufbuild/buf:1.73.0@sha256:75d8f756f0919dcded71e67968a91adfe55fe4dd62c4900c57bbfcd7fe625608 AS buf
FROM --platform=$BUILDPLATFORM rust:1.99-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8 AS builder
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

FROM alpine:3.24@sha256:be2656061b536c1883c4034790e21d0a09c5aaaf51968b89368a3c0371ea2c9a AS runtime
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
