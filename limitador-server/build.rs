use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    set_git_hash("LIMITADOR_GIT_HASH");
    set_profile("LIMITADOR_PROFILE");
    set_features("LIMITADOR_FEATURES");
    generate_protobuf()
}

fn generate_protobuf() -> Result<(), Box<dyn Error>> {
    let original_out_dir = PathBuf::from(env::var("OUT_DIR")?);
    tonic_build::configure()
        .build_server(true)
        .file_descriptor_set_path(original_out_dir.join("rls.bin"))
        .compile_protos(
            &["envoy/service/ratelimit/v3/rls.proto"],
            &[
                "vendor/protobufs/data-plane-api",
                "vendor/protobufs/protoc-gen-validate",
                "vendor/protobufs/xds",
            ],
        )?;

    tonic_build::configure()
        .build_server(true)
        .file_descriptor_set_path(original_out_dir.join("kuadrantrls.bin"))
        .compile_protos(
            &["kuadrantrls.proto"],
            &[
                "proto",
                "vendor/protobufs/data-plane-api",
                "vendor/protobufs/protoc-gen-validate",
                "vendor/protobufs/xds",
            ],
        )?;
    Ok(())
}

fn set_profile(env: &str) {
    if let Ok(profile) = std::env::var("PROFILE") {
        println!("cargo:rustc-env={env}={profile}");
    }
}

fn set_features(env: &str) {
    let mut features = vec![];
    if cfg!(feature = "distributed_storage") {
        features.push("+distributed");
    }
    println!("cargo:rustc-env={env}={features:?}");
}

fn set_git_hash(env: &str) {
    let git_sha = Command::new("/usr/bin/git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|x| String::from_utf8(x.stdout).ok())
        .map(|sha| sha[..8].to_owned());

    if let Some(sha) = git_sha {
        let dirty = Command::new("git")
            .args(["diff", "--stat"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| !matches!(output.stdout.len(), 0));

        match dirty {
            Some(true) => println!("cargo:rustc-env={env}={sha}-dirty"),
            Some(false) => println!("cargo:rustc-env={env}={sha}"),
            _ => unreachable!("How can we have a git hash, yet not know if the tree is dirty?"),
        }
    } else {
        let fallback = option_env!("GITHUB_SHA")
            .map(|sha| if sha.len() > 8 { &sha[..8] } else { sha })
            .unwrap_or("NO_SHA");
        println!("cargo:rustc-env={env}={fallback}");
    }
}
