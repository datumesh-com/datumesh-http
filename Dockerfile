# The datumesh-mesh image: built in rust:1.99, run on debian:trixie-slim (the same Debian
# release, so the same glibc) as an unprivileged user. Configuration from the environment
# (see the README); the signing key and the API keys from files with mode 0600 owned by the
# container's user (uid 10001), e.g. a volume at /run/secrets.
FROM rust:1.99-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --bin datumesh-mesh \
 && strip target/release/datumesh-mesh

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --user-group --no-create-home --shell /usr/sbin/nologin mesh
COPY --from=build /src/target/release/datumesh-mesh /usr/local/bin/datumesh-mesh
USER 10001:10001
ENV DATUMESH_LISTEN=0.0.0.0:8080 \
    RUST_LOG=info
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=5s --retries=3 CMD ["datumesh-mesh", "health"]
ENTRYPOINT ["datumesh-mesh"]
CMD ["serve"]
