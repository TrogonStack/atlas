# JS only runs on the build platform: node under QEMU arm64 emulation
# crashes with illegal instruction. Runtime deps are pure JS, so the
# deployed node_modules is valid for every target platform.
#
# Build context is the repository root. A bind mount puts the tsworkspace
# pnpm workspace at /mnt/tsworkspace, so the install resolves against the
# workspace lockfile. `pnpm deploy` then produces a self-contained studio
# tree with production deps only.
FROM --platform=$BUILDPLATFORM node:26-alpine@sha256:143494b1da2945f061539253adc65e4f1569ddf07da2d384c022c791a9d90a4a AS build
WORKDIR /src
RUN npm install -g pnpm@10.3.0
RUN --mount=type=bind,source=tsworkspace,target=/mnt/tsworkspace \
    cp /mnt/tsworkspace/pnpm-workspace.yaml /mnt/tsworkspace/package.json /mnt/tsworkspace/pnpm-lock.yaml . \
 && mkdir -p webapps/studio \
 && cp /mnt/tsworkspace/webapps/studio/package.json webapps/studio/
RUN pnpm install --frozen-lockfile
RUN --mount=type=bind,source=tsworkspace,target=/mnt/tsworkspace \
    cp -r /mnt/tsworkspace/. .
RUN pnpm --filter trogon-atlas-studio run build \
 && pnpm --filter trogon-atlas-studio deploy --legacy --prod /app

FROM node:26-alpine
WORKDIR /app
RUN addgroup -g 101 -S studio && adduser -S -u 100 -G studio studio \
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
