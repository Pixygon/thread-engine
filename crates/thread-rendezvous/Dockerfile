# thread-rendezvous — the reference P2P rendezvous. Builds just this crate from
# the workspace, runs it on :4100 (plain ws; terminate TLS at the proxy / Coolify).
FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p thread-rendezvous

FROM debian:stable-slim
RUN useradd -m rendezvous
COPY --from=build /src/target/release/thread-rendezvous /usr/local/bin/thread-rendezvous
USER rendezvous
ENV THREAD_RENDEZVOUS_ADDR=0.0.0.0:4100
EXPOSE 4100
CMD ["thread-rendezvous"]
