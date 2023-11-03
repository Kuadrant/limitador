# ------------------------------------------------------------------------------
# Build Stage
# ------------------------------------------------------------------------------

FROM registry.access.redhat.com/ubi9/ubi:9.2 as limitador-build
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

ARG RUSTC_VERSION=1.72.0

# the powertools repo is required for protobuf-c and protobuf-devel
RUN dnf -y --setopt=install_weak_deps=False --setopt=tsflags=nodocs install \
      https://mirror.stream.centos.org/9-stream/BaseOS/`arch`/os/Packages/centos-gpg-keys-9.0-23.el9.noarch.rpm \
      https://mirror.stream.centos.org/9-stream/BaseOS/`arch`/os/Packages/centos-stream-repos-9.0-23.el9.noarch.rpm \
 && dnf -y --setopt=install_weak_deps=False --setopt=tsflags=nodocs install epel-release \
 && dnf config-manager --set-enabled crb

RUN PKGS="protobuf-devel git clang perl" \
    && dnf install --nodocs --assumeyes $PKGS \
    && rpm --verify --nogroup --nouser $PKGS \
    && yum -y clean all

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --no-modify-path --profile minimal --default-toolchain ${RUSTC_VERSION} -c rustfmt -y

WORKDIR /usr/src/limitador

ARG GITHUB_SHA
ENV GITHUB_SHA=${GITHUB_SHA:-unknown}
ENV RUSTFLAGS="-C target-feature=-crt-static"

COPY . .

RUN source $HOME/.cargo/env \
    && cargo build --release

# ------------------------------------------------------------------------------
# Run Stage
# ------------------------------------------------------------------------------

FROM registry.access.redhat.com/ubi9/ubi-minimal:9.2

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

CMD ["limitador-server"]
