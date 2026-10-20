# The self-hosted A2 Tools site. Build from the repository root, because the
# server compiles in the meter's parser (../src-tauri) and data (../src/data):
#
#   docker build -f server/Dockerfile -t a2tools-server .
#
# or `docker compose up -d` in server/.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY src-tauri ./src-tauri
COPY src/data ./src/data
COPY src/assets ./src/assets
COPY server ./server
RUN cargo build --release --locked --manifest-path server/Cargo.toml

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home /data --shell /usr/sbin/nologin a2tools \
 && mkdir -p /data && chown a2tools /data
COPY --from=build /src/server/target/release/a2tools-server /usr/local/bin/a2tools-server
USER a2tools
ENV A2_LISTEN=0.0.0.0:8080 \
    A2_DATA_DIR=/data
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD ["/bin/sh", "-c", "exec 3<>/dev/tcp/127.0.0.1/8080 && printf 'GET /healthz HTTP/1.0\\r\\n\\r\\n' >&3 && grep -q ok <&3"]
ENTRYPOINT ["/usr/local/bin/a2tools-server"]
