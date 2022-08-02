use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let git_hash = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|x| String::from_utf8(x.stdout).ok())
        .map(|hash| hash[..8].to_owned());

    let dirty = Command::new("git")
        .args(&["diff", "--stat"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !matches!(output.stdout.len(), 0));

    if Some(true) == dirty && git_hash.is_some() {
        println!(
            "cargo:rustc-env=LIMITADOR_GIT_HASH={}-dirty",
            git_hash.unwrap_or_else(|| "unknown".to_owned())
        );
    } else {
        println!(
            "cargo:rustc-env=LIMITADOR_GIT_HASH={}",
            git_hash.unwrap_or_else(|| "unknown".to_owned())
        );
    }

    if let Ok(profile) = std::env::var("PROFILE") {
        println!("cargo:rustc-env=LIMITADOR_PROFILE={}", profile);
    }

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
