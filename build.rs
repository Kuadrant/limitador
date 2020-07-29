use std::env;
use std::process::Command;

struct ProtoBufRepo {
    pub name: String,
    pub url: String,
    pub revision: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = env::var("OUT_DIR").unwrap();

    let repos = vec![
        ProtoBufRepo {
            name: "data-plane-api".to_string(),
            url: "https://github.com/envoyproxy/data-plane-api.git".to_string(),
            revision: "b4cdc2be93283b5dd59723c4c3f3387580a7031f".to_string(),
        },
        ProtoBufRepo {
            name: "protoc-gen-validate".to_string(),
            url: "https://github.com/envoyproxy/protoc-gen-validate.git".to_string(),
            revision: "478e95eb5ebe9afa11d767b6ce53dec79b6cc8c4".to_string(),
        },
        ProtoBufRepo {
            name: "udpa".to_string(),
            url: "https://github.com/cncf/udpa.git".to_string(),
            revision: "3b31d022a144b334eb2224838e4d6952ab5253aa".to_string(),
        },
    ];

    for repo in &repos {
        Command::new("git")
            .args(&["clone", &repo.url])
            .arg(&repo_out_dir(&out_dir, &repo.name))
            .status()
            .unwrap();

        Command::new("git")
            .args(&["checkout", &repo.revision])
            .current_dir(&repo_out_dir(&out_dir, &repo.name))
            .status()
            .unwrap();
    }

    tonic_build::configure()
        .build_server(true)
        .out_dir("src/protobufs")
        .compile(
            &[
                "envoy/service/ratelimit/v3/rls.proto",
                "envoy/service/ratelimit/v2/rls.proto",
            ],
            &[
                &repo_out_dir(&out_dir, &repos[0].name),
                &repo_out_dir(&out_dir, &repos[1].name),
                &repo_out_dir(&out_dir, &repos[2].name),
            ],
        )?;
    Ok(())
}

fn repo_out_dir(script_out_dir: &str, repo_name: &str) -> String {
    format!("{}/protobufs/{}", script_out_dir, repo_name)
}
