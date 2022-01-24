# Based on https://shaneutt.com/blog/rust-fast-small-docker-image-builds/

# ------------------------------------------------------------------------------
# Build Stage
# ------------------------------------------------------------------------------

FROM rust:1.58 as limitador-build

RUN apt-get update \
 && apt-get install musl-tools -y

RUN rustup target add x86_64-unknown-linux-musl \
 && rustup component add rustfmt

WORKDIR /usr/src/limitador

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

COPY limitador/Cargo.toml ./limitador/Cargo.toml
COPY limitador-server/Cargo.toml ./limitador-server/Cargo.toml

RUN mkdir -p limitador/src limitador-server/src

RUN echo "fn main() {println!(\"if you see this, the build broke\")}" > limitador/src/main.rs \
 && echo "fn main() {println!(\"if you see this, the build broke\")}" > limitador-server/src/main.rs

RUN cargo build --release --target=x86_64-unknown-linux-musl --all-features

# avoid downloading and compiling all the dependencies when there's a change in
# our code.
RUN rm -f target/x86_64-unknown-linux-musl/release/deps/limitador*

COPY . .

RUN cargo build --release --target=x86_64-unknown-linux-musl --all-features


# ------------------------------------------------------------------------------
# Run Stage
# ------------------------------------------------------------------------------

FROM alpine:3.13

RUN addgroup -g 1000 limitador \
 && adduser -D -s /bin/sh -u 1000 -G limitador limitador

WORKDIR /home/limitador/bin/
ENV PATH="/home/limitador/bin:${PATH}"

COPY --from=limitador-build /usr/src/limitador/limitador-server/examples/limits.yaml ../
COPY --from=limitador-build /usr/src/limitador/target/x86_64-unknown-linux-musl/release/limitador-server ./limitador-server

RUN chown limitador:limitador limitador-server

USER limitador

CMD ["limitador-server"]
