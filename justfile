IMAGE           := "ghcr.io/y9938/ntfy-matrix-bot"

NATIVE_PLATFORM := "linux/" + `uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/'`
ALL_PLATFORMS   := "linux/amd64,linux/arm64"
BUILDER         := "ntfy-matrix-bot-builder"

# Create the docker-container builder instance
setup:
    docker buildx inspect {{ BUILDER }} > /dev/null 2>&1 \
        || docker buildx create --name {{ BUILDER }} --driver docker-container --bootstrap

# Build image locally for host architecture
build: setup
    docker buildx build \
        --builder {{ BUILDER }} \
        --platform {{ NATIVE_PLATFORM }} \
        --tag {{ IMAGE }}:dev \
        --load \
        .

# Push multi-arch manifest (amd64 + arm64) to registry
push tag: setup
    docker buildx build \
        --builder {{ BUILDER }} \
        --platform {{ ALL_PLATFORMS }} \
        --tag {{ IMAGE }}:{{ tag }} \
        --tag {{ IMAGE }}:latest \
        --push \
        .

# Free BuildKit cache
prune:
    docker buildx prune --builder {{ BUILDER }} -f
