# ------------------------------------------------------------------------------
# Build Stage
# ------------------------------------------------------------------------------

FROM registry.access.redhat.com/ubi10/ubi-minimal:10.1 AS limitador-build

ARG RUSTC_VERSION=1.92.0
ARG PROTOC_VERSION=3.19.4

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:${PATH}

RUN microdnf --assumeyes install --nodocs \
        gcc gcc-c++ make clang clang-devel openssl-devel pkgconf-pkg-config \
        tar gzip unzip \
    && microdnf clean all

RUN case "$(uname -m)" in \
        x86_64) RUSTUP_ARCH=x86_64-unknown-linux-gnu; PROTOC_ARCH=x86_64 ;; \
        aarch64) RUSTUP_ARCH=aarch64-unknown-linux-gnu; PROTOC_ARCH=aarch_64 ;; \
        ppc64le) RUSTUP_ARCH=powerpc64le-unknown-linux-gnu; PROTOC_ARCH=ppcle_64 ;; \
        s390x) RUSTUP_ARCH=s390x-unknown-linux-gnu; PROTOC_ARCH=s390_64 ;; \
        *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;; \
    esac \
    && cd /tmp \
    && curl --fail --show-error --location --proto '=https' --tlsv1.2 \
        --output rustup-init "https://static.rust-lang.org/rustup/dist/${RUSTUP_ARCH}/rustup-init" \
    && curl --fail --show-error --location --proto '=https' --tlsv1.2 \
        --output rustup-init.sha256 "https://static.rust-lang.org/rustup/dist/${RUSTUP_ARCH}/rustup-init.sha256" \
    && sha256sum -c rustup-init.sha256 \
    && chmod +x rustup-init \
    && ./rustup-init --no-modify-path --profile minimal --default-toolchain "${RUSTC_VERSION}" -y \
    && curl --fail --show-error --location --proto '=https' --tlsv1.2 \
        --output protoc.zip "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-${PROTOC_ARCH}.zip" \
    && unzip -q protoc.zip -d /usr/local \
    && rm rustup-init rustup-init.sha256 protoc.zip

WORKDIR /usr/src/limitador

ARG GITHUB_SHA
ARG CARGO_ARGS
ENV GITHUB_SHA=${GITHUB_SHA:-unknown}
ENV RUSTFLAGS="-C target-feature=-crt-static"

# We set the env here just to make sure that the build is invalidated if the args change
ENV CARGO_ARGS=${CARGO_ARGS}

# The following allows us to cache the Cargo dependency downloads with image layers
COPY Cargo.toml Cargo.lock ./
COPY limitador/Cargo.toml ./limitador/
COPY limitador-server/Cargo.toml ./limitador-server/
RUN mkdir -p limitador-server/src && echo 'fn main() {}' > limitador-server/src/main.rs
RUN cargo build --release ${CARGO_ARGS}

COPY ./limitador ./limitador
COPY ./limitador-server ./limitador-server

RUN cargo build --release ${CARGO_ARGS}

# ------------------------------------------------------------------------------
# Run Stage
# ------------------------------------------------------------------------------

FROM registry.access.redhat.com/ubi10/ubi-minimal:10.1

# shadow-utils is required for `useradd`
RUN PKGS="libgcc libstdc++ shadow-utils" \
    && microdnf --assumeyes install --nodocs $PKGS \
    && rpm --verify --nogroup --nouser $PKGS \
    && microdnf -y clean all
RUN useradd -u 1000 -s /bin/sh -m -d /home/limitador limitador

WORKDIR /home/limitador/bin/
ENV PATH="/home/limitador/bin:${PATH}"

COPY --from=limitador-build /usr/src/limitador/limitador-server/examples/limits.yaml ../
COPY --from=limitador-build /usr/src/limitador/target/release/limitador-server ./limitador-server

RUN chown -R limitador:root /home/limitador \
    && chmod -R 750 /home/limitador

USER limitador

ENTRYPOINT ["limitador-server"]
