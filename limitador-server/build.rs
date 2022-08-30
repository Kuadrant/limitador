use std::error::Error;
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    set_git_hash("LIMITADOR_GIT_HASH");
    set_profile("LIMITADOR_PROFILE");
    set_features("LIMITADOR_FEATURES");
    generate_protobuf()
}

fn generate_protobuf() -> Result<(), Box<dyn Error>> {
    tonic_build::configure().build_server(true).compile(
        &["envoy/service/ratelimit/v3/rls.proto"],
        &[
            "vendor/protobufs/data-plane-api",
            "vendor/protobufs/protoc-gen-validate",
            "vendor/protobufs/xds",
        ],
    )?;
    Ok(())
}

fn set_features(env: &str) {
    if cfg!(feature = "infinispan") {
        println!("cargo:rustc-env={}=[+infinispan]", env);
    }
}

fn set_profile(env: &str) {
    if let Ok(profile) = std::env::var("PROFILE") {
        println!("cargo:rustc-env={}={}", env, profile);
    }
}

fn set_git_hash(env: &str) {
    let git_hash = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|x| String::from_utf8(x.stdout).ok())
        .map(|hash| hash[..8].to_owned());

    if let Some(hash) = git_hash {
        let dirty = Command::new("git")
            .args(&["diff", "--stat"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| !matches!(output.stdout.len(), 0));

        match dirty {
            Some(true) => println!("cargo:rustc-env={}={}-dirty", env, hash),
            Some(false) => println!("cargo:rustc-env={}={}", env, hash),
            _ => unreachable!("How can we have a git hash, yet not know if the tree is dirty?"),
        }
    } else {
        println!("cargo:rustc-env={}=unknown", env);
    }
}
