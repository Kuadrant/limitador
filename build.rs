fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .out_dir("src/protobufs")
        .compile(
            &[
                "envoy/service/ratelimit/v3/rls.proto",
                "envoy/service/ratelimit/v2/rls.proto",
            ],
            &[
                "envoy-xds-grpc/data-plane-api",
                "envoy-xds-grpc/googleapis",
                "envoy-xds-grpc/protoc-gen-validate",
                "envoy-xds-grpc/udpa",
            ],
        )?;
    Ok(())
}
