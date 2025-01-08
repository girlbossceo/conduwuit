
ARG BUILDPLATFORM

FROM --platform=$BUILDPLATFORM rust:latest AS builder

ARG TARGETPLATFORM
# Determine target platform identifiers
RUN RUST_TARGET=$(case $TARGETPLATFORM in \
    "linux/amd64") echo x86_64-unknown-linux-gnu ;; \
    "linux/arm64") echo aarch64-unknown-linux-gnu ;; \
    *) echo "Unsupported target platform $TARGETPLATFORM" 1>&2; exit 1 ;; \
    esac) && \
    echo "RUST_TARGET=$RUST_TARGET" >> /etc/environment

RUN DEB_ARCH=$(case $TARGETPLATFORM in \
    "linux/amd64") echo amd64 ;; \
    "linux/arm64") echo arm64 ;; \
    *) exit 1 ;; \
    esac) && \
    echo "DEB_ARCH=$DEB_ARCH" >> /etc/environment

RUN GCC_TARGET=$(case $TARGETPLATFORM in \
    "linux/amd64") echo x86-64-linux-gnu ;; \
    "linux/arm64") echo aarch64-linux-gnu ;; \
    *) exit 1 ;; \
    esac) && \
    echo "GCC_TARGET=$GCC_TARGET" >> /etc/environment

# install build-time deps
RUN set -o allexport && \
    . /etc/environment && \
    dpkg --add-architecture $DEB_ARCH && \
    apt-get update && apt-get install -y --no-install-recommends \
        lld \ 
        gcc-$GCC_TARGET g++-$GCC_TARGET \
        libc6-dev-$DEB_ARCH-cross \
        libclang-dev liburing-dev \
        liburing-dev:$DEB_ARCH && \
    rm -rf /var/lib/apt/lists/*

# set linkers, compilers and pkg-config libdir
RUN VARS=$(case $TARGETPLATFORM in \
    "linux/amd64") \
        echo "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc" && \
        echo "CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc" && \
        echo "CXX_x86_64_unknown_linux_gnu=\"x86_64-linux-gnu-g++\"" && \
        echo "PKG_CONFIG_LIBDIR=/usr/lib/x86_64-linux-gnu/pkgconfig" \
    ;; \
    "linux/arm64") \
        echo "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc" && \
        echo "CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc" && \
        echo "CXX_aarch64_unknown_linux_gnu=\"aarch64-linux-gnu-g++\"" && \
        echo "PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig"  \
    ;; \
    *) exit 1 ;; \
    esac) && \
    echo "$VARS" >> /etc/environment

# enable cross-platform linking of libraries
RUN echo "PKG_CONFIG_ALLOW_CROSS=true" >> /etc/environment

# Set up Rust toolchain
WORKDIR /app
COPY ./rust-toolchain.toml .
RUN rustc --version
RUN set -o allexport && \
    . /etc/environment && \
    rustup target add $RUST_TARGET

# Developer tool versions
# renovate: datasource=github-releases depName=cargo-binstall packageName=cargo-bins/cargo-binstall
ENV BINSTALL_VERSION=1.10.17
# renovate: github-releases depName=cargo-sbom packageName=psastras/sbom-rs
ENV CARGO_SBOM_VERSION=0.9.1

RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash
RUN cargo binstall --no-confirm cargo-sbom --version $CARGO_SBOM_VERSION

# Get source
COPY . .

# Build binary
# We disable incremental compilation to save disk space, as it only produces a minimal speedup for this case.
ENV CARGO_INCREMENTAL=0

ARG TARGET_CPU=
RUN if [ -n "${TARGET_CPU}" ]; then \
    echo "CFLAGS='-march=${TARGET_CPU}'" >> /etc/environment && \
    echo "CXXFLAGS='-march=${TARGET_CPU}'" >> /etc/environment && \
    echo "RUSTFLAGS='-C target-cpu=${TARGET_CPU}'" >> /etc/environment; \
fi

RUN cat /etc/environment
RUN mkdir /out
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git/db \
    --mount=type=cache,target=/app/target \
    set -o allexport && \
    . /etc/environment && \
    cargo build --locked --release --target $RUST_TARGET && \
    cp ./target/$RUST_TARGET/release/conduwuit /out/app

RUN cargo sbom > /out/sbom.spdx.json

# find dynamically linked dependencies
RUN mkdir /out/libs \
    && ldd /out/app | grep '=>' | awk '{print $(NF-1)}' | xargs -I {} cp {} /out/libs/
# libraries with a hardcoded path, like ld
# (see for example https://github.com/vlang/v/issues/8682)
# Excluding linux-vdso.so, as it is a part of the kernel
RUN mkdir /out/libs-root \
    && ldd /out/app | grep -v '=>' | grep -v 'linux-vdso.so' | awk '{print $(NF-1)}' | xargs -I {} install -D {} /out/libs-root{}
# RUN ldd /out/app
# ldd /out/app | grep -v 'linux-vdso.so' | awk '{print $(NF-1)}'
# RUN ls /libs

FROM scratch

WORKDIR /

# Copy root certs for tls into image
# You can also mount the certs from the host
# --volume /etc/ssl/certs:/etc/ssl/certs:ro
COPY --from=rust:latest /etc/ssl/certs /etc/ssl/certs

# Copy our build
COPY --from=builder /out/app ./app 
# Copy SBOM
COPY --from=builder /out/sbom.spdx.json ./sbom.spdx.json

# Copy hardcoded dynamic libraries
COPY --from=builder /out/libs-root /
# Copy dynamic libraries
COPY --from=builder /out/libs /libs
# Tell Linux where to find our libraries
ENV LD_LIBRARY_PATH=/libs


CMD ["/app"]
