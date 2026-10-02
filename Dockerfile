# Build from the repository root with the submodules checked out:
#
#   git submodule update --init
#   docker build -t documentation-server .
#
# Optional build secrets:
#   github_token -- only if a git dependency is private (wyvrn-cloud/didcomm is public):
#                   --secret id=github_token,env=GITHUB_TOKEN
#   ca_bundle    -- extra CA certificates, for building behind a TLS-intercepting proxy:
#                   --secret id=ca_bundle,src=/path/to/ca.pem
# Neither ends up in any image layer.
FROM rust:1-slim-bookworm AS builder

# git: cargo fetches the didcomm git dependency with the git CLI (.cargo/config.toml).
# cmake/gcc/perl: aws-lc-rs (rustls' crypto provider) builds a C library.
RUN apt-get update && apt-get install -y --no-install-recommends git cmake gcc perl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY src src

# aws-lc-rs' cmake build ignores cargo's -j; cap it so constrained builders don't OOM.
ENV CMAKE_BUILD_PARALLEL_LEVEL=2
RUN --mount=type=secret,id=github_token,required=false \
    --mount=type=secret,id=ca_bundle,required=false \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    set -e; \
    if [ -s /run/secrets/ca_bundle ]; then \
        cat /etc/ssl/certs/ca-certificates.crt /run/secrets/ca_bundle > /tmp/ca.pem; \
        export SSL_CERT_FILE=/tmp/ca.pem GIT_SSL_CAINFO=/tmp/ca.pem CARGO_HTTP_CAINFO=/tmp/ca.pem; \
    fi; \
    if [ -s /run/secrets/github_token ]; then \
        export GIT_CONFIG_COUNT=1 \
            GIT_CONFIG_KEY_0="url.https://x-access-token:$(cat /run/secrets/github_token)@github.com/.insteadOf" \
            GIT_CONFIG_VALUE_0="https://github.com/"; \
    fi; \
    cargo build --release --locked -j 2

FROM debian:bookworm-slim
# ca-certificates: did:web resolution and replies to HTTPS endpoints.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /app docserver
WORKDIR /app
COPY --from=builder /build/target/release/documentation-server /usr/local/bin/documentation-server
COPY config config
COPY mappings mappings
COPY schemas schemas
# Only what the index reads: protocol definitions, and the spec's Markdown (not its
# ~90 MB of images or the rendered HTML).
# The image has no .git, so the sources' commits come from this file (index.revisions).
COPY sources/revisions.toml sources/revisions.toml
COPY sources/didcomm.org/site/content/protocols sources/didcomm.org/site/content/protocols
COPY sources/didcomm-messaging/specs.json sources/didcomm-messaging/specs.json
COPY sources/didcomm-messaging/docs/spec-files sources/didcomm-messaging/docs/spec-files
COPY sources/didcomm-messaging/docs/spec-files-v2.0-snapshot sources/didcomm-messaging/docs/spec-files-v2.0-snapshot
COPY sources/didcomm-messaging/docs/spec-files-v2.1-snapshot sources/didcomm-messaging/docs/spec-files-v2.1-snapshot
# The identity lives in a volume so the server keeps its DID across container restarts.
RUN mkdir -p data && chown docserver data
VOLUME /app/data
USER docserver
EXPOSE 8080
ENTRYPOINT ["documentation-server"]
