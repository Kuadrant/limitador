use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .out_dir(env::var("OUT_DIR").expect("OUT_DIR not set during compilation!"))
        .compile(
            &["envoy/service/ratelimit/v3/rls.proto"],
            &[
                "vendor/protobufs/data-plane-api",
                "vendor/protobufs/protoc-gen-validate",
                "vendor/protobufs/udpa",
            ],
        )?;
    Ok(())
}
