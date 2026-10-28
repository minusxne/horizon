# skyline-server in a container (docs/server.md, section 3). Build context: the repository root:
#   docker build -f packaging/server/Dockerfile -t skyline-server .
#
# The image holds none of Minecraft's files. Prepare them on the host from your own Minecraft 26.3
# (skyline-prepare --prepare, see packaging/PLAY.txt) and mount the folder it writes, read-only:
#   docker run -d --name horizon -p 25565:25565 --stop-timeout 180 \
#     -v horizon-data:/data \
#     -v /path/to/.local/skyline/26.3:/opt/horizon/.local/skyline/26.3:ro \
#     -e SKYLINE_EULA=true -e SKYLINE_WHITELIST=true -e SKYLINE_PLAYERS=Alex,Steve -e SKYLINE_OP=Alex \
#     skyline-server
#
# SKYLINE_EULA=true accepts the Minecraft EULA (https://aka.ms/MinecraftEULA) for this server. The first
# start writes /data/server.toml from SKYLINE_* (EULA, WORLD, SEED, PORT, WHITELIST, PLAYERS, OP); later
# starts use the file. Manage it with `docker exec horizon skyline-server status|cmd|backup|logs ...`.
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
# The server build of the game (no sound, no controllers: it needs neither ALSA nor udev) as skyline-engine.
RUN cargo build --release --locked -p horizon-client --no-default-features --bin horizon-client \
    && cargo build --release --locked -p horizon-server --bin skyline-server \
    && mkdir /out && cp target/release/horizon-client /out/skyline-engine && cp target/release/horizon-server /out/

FROM debian:bookworm-slim
RUN useradd --system --home-dir /data --shell /usr/sbin/nologin horizon \
    && mkdir -p /data /opt/horizon/.local/skyline \
    && chown horizon:horizon /data
COPY --from=build /out/ /opt/horizon/
ENV PATH=/opt/horizon:$PATH \
    SKYLINE_NO_AUDIO=1
WORKDIR /data
VOLUME /data
EXPOSE 25565
USER horizon
# SIGTERM makes skyline-server stop the game cleanly (it saves first): give it --stop-timeout 180.
CMD ["sh", "-c", "test -f server.toml || skyline-server init --yes || exit 1; exec skyline-server run"]
