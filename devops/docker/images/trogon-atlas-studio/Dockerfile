# JS only runs on the build platform: node under QEMU arm64 emulation
# crashes with illegal instruction. Runtime deps are pure JS, so the
# deployed node_modules is valid for every target platform.
#
# Build context is the repository root. A bind mount puts the tsworkspace
# pnpm workspace at /mnt/tsworkspace, so the install resolves against the
# workspace lockfile. `pnpm deploy` then produces a self-contained studio
# tree with production deps only.
FROM --platform=$BUILDPLATFORM node:26-alpine@sha256:6f08a523443ddedf52941874c203475b0ad503e859ad70cb26ab8be9a517dbd5 AS build
ARG BUILDARCH
ARG PNPM_VERSION=10.3.0
ARG PNPM_LINUXSTATIC_X64_SHA256=13319046c28ab9cf18838aea7a06afcba20156c90ee3f164b9e0d11360b0bbd9
ARG PNPM_LINUXSTATIC_ARM64_SHA256=faff1ae7eba0c51c8c05bb27565a897019c210331fcfb763d32db886ae004066
WORKDIR /src
RUN set -eu; \
    case "$BUILDARCH" in \
        amd64) pnpm_arch=x64; pnpm_sha256="$PNPM_LINUXSTATIC_X64_SHA256" ;; \
        arm64) pnpm_arch=arm64; pnpm_sha256="$PNPM_LINUXSTATIC_ARM64_SHA256" ;; \
        *) echo "unsupported BUILDARCH: $BUILDARCH" >&2; exit 1 ;; \
    esac; \
    wget -q -O /usr/local/bin/pnpm "https://github.com/pnpm/pnpm/releases/download/v${PNPM_VERSION}/pnpm-linuxstatic-${pnpm_arch}"; \
    echo "${pnpm_sha256}  /usr/local/bin/pnpm" > /tmp/pnpm.sha256; \
    sha256sum -c /tmp/pnpm.sha256; \
    rm -f /tmp/pnpm.sha256; \
    chmod +x /usr/local/bin/pnpm
RUN --mount=type=bind,source=tsworkspace,target=/mnt/tsworkspace \
    cp /mnt/tsworkspace/pnpm-workspace.yaml /mnt/tsworkspace/package.json /mnt/tsworkspace/pnpm-lock.yaml . \
 && mkdir -p webapps/studio \
 && cp /mnt/tsworkspace/webapps/studio/package.json webapps/studio/
RUN pnpm install --frozen-lockfile
RUN --mount=type=bind,source=tsworkspace,target=/mnt/tsworkspace \
    cp -r /mnt/tsworkspace/. .
RUN pnpm --filter trogon-atlas-studio run build \
 && pnpm --filter trogon-atlas-studio deploy --legacy --prod /app

FROM node:26-alpine@sha256:6f08a523443ddedf52941874c203475b0ad503e859ad70cb26ab8be9a517dbd5
WORKDIR /app
RUN apk upgrade --no-cache \
 && addgroup -g 101 -S studio && adduser -S -u 100 -G studio studio \
 && rm -rf /usr/local/lib/node_modules/npm /usr/local/bin/npm /usr/local/bin/npx
COPY --from=build /app/package.json ./
COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/dist ./dist
COPY --from=build /app/server ./server
COPY --from=build /app/shared ./shared
# Canonical protos come from a bind mount of the repository's proto/ tree so
# the image never embeds a stale copy. event_model.proto transitively imports
# the sibling files, so the whole schema set ships together.
RUN --mount=type=bind,source=proto,target=/mnt/proto \
    mkdir -p /app/proto && cp -r /mnt/proto/. /app/proto/
USER 100:101
ENV PROTO_PATH=/app/proto/trogonatlas/api/eventmodel/v1alpha1/service.proto \
    HOST=0.0.0.0 \
    PORT=80
EXPOSE 80
CMD ["node", "server/index.mjs"]
