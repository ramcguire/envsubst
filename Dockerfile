# syntax=docker/dockerfile:1

# Override at build time: --build-arg DISTROLESS_VARIANT=static-debian13 --build-arg DISTROLESS_TAG=nonroot
ARG DISTROLESS_VARIANT=static-debian12
ARG DISTROLESS_TAG=latest

# Verify distroless signature
FROM --platform=$BUILDPLATFORM cgr.dev/chainguard/cosign:latest AS verifier
ARG DISTROLESS_VARIANT
ARG DISTROLESS_TAG
RUN cosign verify \
      --certificate-oidc-issuer=https://accounts.google.com \
      --certificate-identity=keyless@distroless.iam.gserviceaccount.com \
      "gcr.io/distroless/${DISTROLESS_VARIANT}:${DISTROLESS_TAG}"

# Build static binary with cargo-zigbuild to produce a full static musl binary
FROM --platform=$BUILDPLATFORM ghcr.io/rust-cross/cargo-zigbuild:latest AS builder
# If signature verification failed, this COPY fails and the build is aborted.
COPY --from=verifier /etc/os-release /tmp/distroless-verified

ARG TARGETARCH

RUN case "$TARGETARCH" in \
      amd64)  echo x86_64-unknown-linux-musl      ;; \
      arm64)  echo aarch64-unknown-linux-musl      ;; \
      arm)    echo armv7-unknown-linux-musleabihf  ;; \
      *)      echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac > /rust_target

RUN rustup target add "$(cat /rust_target)"

WORKDIR /app

# Cache dependency compilation separately
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main(){}' > src/main.rs \
 && cargo zigbuild --release --target "$(cat /rust_target)" \
 && rm -rf src

COPY src ./src
RUN touch src/main.rs \
 && cargo zigbuild --release --target "$(cat /rust_target)" \
 && cp "target/$(cat /rust_target)/release/envsubst" /envsubst

# Final image
FROM gcr.io/distroless/${DISTROLESS_VARIANT}:${DISTROLESS_TAG}
COPY --from=builder /envsubst /usr/local/bin/envsubst
ENTRYPOINT ["/usr/local/bin/envsubst"]
