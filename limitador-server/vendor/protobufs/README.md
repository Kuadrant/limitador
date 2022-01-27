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

- https://github.com/envoyproxy/data-plane-api.git 863f431e56cecb1114a122eb467db7739b5df154
- https://github.com/envoyproxy/protoc-gen-validate.git 7898287a95aefb07aeff95f5f17b8d422d4a5ded
- https://github.com/cncf/xds.git 4a2b9fdd466b16721f8c058d7cadf5a54e229d66

My first solution was to do the clone and the filtering in the build.rs.
However, that does not really work because it means that we need to download
dependencies at build time, which is not supported by docs.rs.

## Deprecated UDPA definitions

The UDPA repository has been replaced by the XDS one, but some definitions still reference
UDPA. When you update the XDS repository, take into account that you must move the UDPA
definitions around to ensure the code generation picks them up.
