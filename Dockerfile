# ------------------------------------------------------------------------------
# Build Stage
# ------------------------------------------------------------------------------

FROM alpine:3.16 as limitador-build

ARG RUSTC_VERSION=1.58.1
RUN apk update \
    && apk upgrade \
    && apk add build-base binutils-gold openssl3-dev protoc curl \
    && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --no-modify-path --profile minimal --default-toolchain ${RUSTC_VERSION} -c rustfmt -y

WORKDIR /usr/src/limitador

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

COPY limitador/Cargo.toml ./limitador/Cargo.toml
COPY limitador-server/Cargo.toml ./limitador-server/Cargo.toml

RUN mkdir -p limitador/src limitador-server/src

RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > limitador/src/main.rs \
    && echo "fn main() {println!(\"if you see this, the build broke\")}" > limitador-server/src/main.rs

RUN source $HOME/.cargo/env \
    && cargo build --release --all-features

# avoid downloading and compiling all the dependencies when there's a change in
# our code.
RUN rm -f target/release/deps/limitador*

COPY . .

RUN source $HOME/.cargo/env \
    && cargo build --release --all-features

# ------------------------------------------------------------------------------
# Run Stage
# ------------------------------------------------------------------------------

FROM alpine:3.16

RUN addgroup -g 1000 limitador \
    && adduser -D -s /bin/sh -u 1000 -G limitador limitador

WORKDIR /home/limitador/bin/
ENV PATH="/home/limitador/bin:${PATH}"

COPY --from=limitador-build /usr/src/limitador/limitador-server/examples/limits.yaml ../
COPY --from=limitador-build /usr/src/limitador/target/release/limitador-server ./limitador-server

RUN chown limitador:limitador limitador-server

USER limitador

CMD ["limitador-server"]
