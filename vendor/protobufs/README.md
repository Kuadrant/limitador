# Vendored protobuf definitions

This directory contains the protobufs definitions needed to build the GRPC
server that implements the Envoy Rate Limit Service protocol. These files are
used from the build script (`build.rs`). The Envoy protobuf definitions have
several dependencies both in the "data-plane-api" repository and external ones.

What I did was clone the 3 repos needed, keep only the .proto files, and remove
all the empty directories. Keeping the directory structure is needed to make
things work.

This solution is not ideal, but I have not found a better way to import proto
definitions and all their dependencies.

Here are the repos and the revisions:

- https://github.com/envoyproxy/data-plane-api.git b4cdc2be93283b5dd59723c4c3f3387580a7031f
- https://github.com/envoyproxy/protoc-gen-validate.git 478e95eb5ebe9afa11d767b6ce53dec79b6cc8c4
- https://github.com/cncf/udpa.git 3b31d022a144b334eb2224838e4d6952ab5253aa

My first solution was to do the clone and the filtering in the build.rs.
However, that does not really work because it means that we need to download
dependencies at build time, which is not supported by docs.rs.
