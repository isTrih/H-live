# syntax=docker/dockerfile:1
FROM node:24.22.0-bookworm-slim AS web
WORKDIR /build
COPY package.json package-lock.json ./
RUN npm ci --ignore-scripts
COPY src ./src
COPY index.html vite.config.js ./
COPY release-notes ./release-notes
RUN npm run build:server

FROM rust:1.99.1-bookworm AS rust
WORKDIR /build
COPY crates ./crates
COPY packaging/server/collect-licenses.sh /collect-licenses.sh
COPY packaging/extra-licenses /extra-licenses
RUN cargo build --manifest-path crates/Cargo.toml -p h-live-server --release --locked \
    && sh /collect-licenses.sh /build/crates/Cargo.toml /dependency-licenses

FROM debian:bookworm-20261005-slim
LABEL org.opencontainers.image.source="https://github.com/isTrih/H-live" org.opencontainers.image.version="0.1.0" org.opencontainers.image.licenses="MIT"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 hlive && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin hlive \
    && install -d -o 10001 -g 10001 -m 0700 /var/lib/h-live /run/h-live-secrets \
    && touch /var/lib/h-live/.volume /run/h-live-secrets/.volume \
    && chown 10001:10001 /var/lib/h-live/.volume /run/h-live-secrets/.volume
WORKDIR /opt/h-live
COPY --from=rust /build/crates/target/release/h-live-server /usr/local/bin/h-live-server
COPY --from=web /build/dist-web ./ui
COPY --from=web /build/node_modules/@tauri-apps/api/LICENSE* ./licenses/javascript/tauri-api/
COPY --from=web /build/node_modules/@tauri-apps/plugin-updater/LICENSE* ./licenses/javascript/tauri-updater/
COPY --from=rust /dependency-licenses ./licenses/dependencies
COPY licenses ./licenses/upstream
COPY src/fonts/licenses ./licenses/fonts
COPY LICENSE DISCLAIMER.md ./
COPY docs/server ./docs
USER 10001:10001
ENV H_LIVE_BIND=0.0.0.0:8080 H_LIVE_DATA_DIR=/var/lib/h-live H_LIVE_SECRETS_DIR=/run/h-live-secrets H_LIVE_UI_DIR=/opt/h-live/ui
EXPOSE 8080
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 CMD ["h-live-server", "healthcheck"]
ENTRYPOINT ["h-live-server"]
CMD ["serve"]
