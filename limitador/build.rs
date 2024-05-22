use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    generate_protobuf()
}

fn generate_protobuf() -> Result<(), Box<dyn Error>> {
    if cfg!(feature = "distributed_storage") {
        let proto_path: &Path = "proto/distributed.proto".as_ref();

        let proto_dir = proto_path
            .parent()
            .expect("proto file should reside in a directory");

        tonic_build::configure()
            .protoc_arg("--experimental_allow_proto3_optional")
            .compile(&[proto_path], &[proto_dir])?;
    }

    Ok(())
}
