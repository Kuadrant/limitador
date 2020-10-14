# Based on https://shaneutt.com/blog/rust-fast-small-docker-image-builds/

# ------------------------------------------------------------------------------
# Build Stage
# ------------------------------------------------------------------------------

FROM rust:1.45 as limitador-build

RUN apt-get update \
 && apt-get install musl-tools -y

RUN rustup target add x86_64-unknown-linux-musl \
 && rustup component add rustfmt

WORKDIR /usr/src/limitador

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

COPY limitador/Cargo.toml ./limitador/Cargo.toml
COPY limitador-envoy-rls/Cargo.toml ./limitador-envoy-rls/Cargo.toml
COPY limitador-http-server/Cargo.toml ./limitador-http-server/Cargo.toml

RUN mkdir -p limitador/src limitador-envoy-rls/src limitador-http-server/src

RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > limitador/src/main.rs \
 && echo "fn main() {println!(\"if you see this, the build broke\")}" > limitador-envoy-rls/src/main.rs \
 && echo "fn main() {println!(\"if you see this, the build broke\")}" > limitador-http-server/src/main.rs

RUN RUSTFLAGS=-Clinker=musl-gcc cargo build --release --target=x86_64-unknown-linux-musl

# avoid downloading and compiling all the dependencies when there's a change in
# our code.
RUN rm -f target/x86_64-unknown-linux-musl/release/deps/limitador*

COPY . .

RUN RUSTFLAGS=-Clinker=musl-gcc cargo build --release --target=x86_64-unknown-linux-musl


# ------------------------------------------------------------------------------
# Run Stage
# ------------------------------------------------------------------------------

FROM alpine:3.12

RUN addgroup -g 1000 limitador \
 && adduser -D -s /bin/sh -u 1000 -G limitador limitador

WORKDIR /home/limitador/bin/
ENV PATH="/home/limitador/bin:${PATH}"

COPY --from=limitador-build /usr/src/limitador/limitador-envoy-rls/examples/limits.yaml ../
COPY --from=limitador-build /usr/src/limitador/target/x86_64-unknown-linux-musl/release/limitador-envoy-rls ./envoy-rls
COPY --from=limitador-build /usr/src/limitador/target/x86_64-unknown-linux-musl/release/limitador-http-server ./http-server

RUN chown limitador:limitador envoy-rls http-server

USER limitador

ENV LIMITS_FILE=/home/limitador/limits.yaml

CMD ["envoy-rls"]
