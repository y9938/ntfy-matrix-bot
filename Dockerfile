# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.97
ARG ZIG_VERSION=0.16.0
ARG CARGO_ZIGBUILD_VERSION=0.23.0

FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-slim-bookworm AS toolchain
WORKDIR /build

ARG ZIG_VERSION
ARG CARGO_ZIGBUILD_VERSION
ARG TARGETPLATFORM

RUN apt-get update \
 && apt-get install -y --no-install-recommends curl xz-utils pkg-config ca-certificates \
 && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    ARCH="$(uname -m)"; \
    curl -fsSL -o /tmp/cargo-zigbuild.tar.xz \
      "https://github.com/rust-cross/cargo-zigbuild/releases/download/v${CARGO_ZIGBUILD_VERSION}/cargo-zigbuild-${ARCH}-unknown-linux-musl.tar.xz"; \
    tar -xJf /tmp/cargo-zigbuild.tar.xz -C /usr/local/cargo/bin --strip-components=1; \
    rm /tmp/cargo-zigbuild.tar.xz; \
    cargo-zigbuild -V

RUN set -eux; \
    ARCH="$(uname -m)"; \
    [ "$ARCH" = "armv7l" ] && ARCH="arm"; \
    curl -fsSL -o /tmp/zig.tar.xz "https://ziglang.org/download/${ZIG_VERSION}/zig-${ARCH}-linux-${ZIG_VERSION}.tar.xz"; \
    tar -xJf /tmp/zig.tar.xz -C /usr/local; \
    ln -s "/usr/local/zig-${ARCH}-linux-${ZIG_VERSION}/zig" /usr/local/bin/zig; \
    rm /tmp/zig.tar.xz; \
    zig version

ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
RUN --mount=type=bind,source=Cargo.toml,target=Cargo.toml \
    --mount=type=bind,source=Cargo.lock,target=Cargo.lock \
    --mount=type=bind,source=src,target=src \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo fetch --locked

FROM --platform=$BUILDPLATFORM toolchain AS builder
WORKDIR /build

ARG TARGETPLATFORM
ARG TARGETARCH

RUN set -eux; \
    case "$TARGETPLATFORM" in \
        "linux/amd64")   echo "x86_64-unknown-linux-musl" > /rust_target.txt ;; \
        "linux/arm64")   echo "aarch64-unknown-linux-musl" > /rust_target.txt ;; \
        "linux/arm/v7")  echo "armv7-unknown-linux-musleabihf" > /rust_target.txt ;; \
        *) echo "Unsupported TARGETPLATFORM: $TARGETPLATFORM" && exit 1 ;; \
    esac; \
    rustup target add "$(cat /rust_target.txt)"

RUN --mount=type=bind,source=Cargo.toml,target=Cargo.toml \
    --mount=type=bind,source=Cargo.lock,target=Cargo.lock \
    --mount=type=bind,source=src,target=src \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,id=ntfy-matrix-bot-target-${TARGETARCH},sharing=locked \
    set -eux; \
    RUST_TARGET="$(cat /rust_target.txt)"; \
    CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
    CARGO_PROFILE_RELEASE_OPT_LEVEL=3 \
    cargo zigbuild --release --locked --target "$RUST_TARGET" --bin ntfy-matrix-bot \
 && cp "target/${RUST_TARGET}/release/ntfy-matrix-bot" /ntfy-matrix-bot

FROM gcr.io/distroless/static-debian12:nonroot AS runtime
LABEL org.opencontainers.image.source="https://github.com/y9938/ntfy-matrix-bot"
LABEL org.opencontainers.image.description="E2EE Matrix bot for ntfy user provisioning"
LABEL org.opencontainers.image.licenses="MIT"

COPY --from=builder /ntfy-matrix-bot /ntfy-matrix-bot

USER nonroot:nonroot
WORKDIR /data
VOLUME ["/data"]

ENTRYPOINT ["/ntfy-matrix-bot"]
