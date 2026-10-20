# The golden-image reference device (lavapipe) in a container, so new GPU
# goldens can be written before a push instead of from a failed CI run.
# Matches the CI "GPU tests (lavapipe)" job: ubuntu-22.04 with its
# mesa-vulkan-drivers, and stable Rust. Used by scripts/goldens-lavapipe.sh.
FROM ubuntu:22.04

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates curl build-essential cmake pkg-config python3 \
        mesa-vulkan-drivers vulkan-tools \
    && rm -rf /var/lib/apt/lists/*

RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
ENV PATH=/root/.cargo/bin:$PATH
