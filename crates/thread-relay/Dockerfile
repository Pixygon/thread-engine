# thread-relay — the reference presence relay. Builds just this crate from the
# workspace, runs it on :4000 (plain ws; terminate TLS at the proxy / Coolify).
FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p thread-relay

FROM debian:stable-slim
RUN useradd -m relay
COPY --from=build /src/target/release/thread-relay /usr/local/bin/thread-relay
USER relay
ENV THREAD_RELAY_ADDR=0.0.0.0:4000
# THREAD_RELAY_AOI=80   (metres; 0 = unlimited)   THREAD_RELAY_TICK=15  (Hz)
EXPOSE 4000
CMD ["thread-relay"]
