fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .out_dir("src/envoy_rls/protobufs")
        .compile(
            &["envoy/service/ratelimit/v3/rls.proto"],
            &[
                "vendor/protobufs/data-plane-api",
                "vendor/protobufs/protoc-gen-validate",
                "vendor/protobufs/xds",
            ],
        )?;
    Ok(())
}
