// Test executable transport. Rust integration tests supply all scenario data.
use std::{
    env, fs,
    io::{Read, Write},
    process::Command,
    thread,
    time::Duration,
};
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--hold") {
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    let root = std::path::PathBuf::from(env::var_os("FIXTURE_ROOT").unwrap());
    let agent = env::args().next().unwrap().ends_with("codex");
    let log = if agent { "agents" } else { "calls" };
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(log))
        .unwrap();
    file.write_all(
        format!(
            "{}\t{}\n",
            std::process::id(),
            if agent {
                String::new()
            } else {
                args.join("\t")
            }
        )
        .as_bytes(),
    )
    .unwrap();
    if agent {
        let mut prompt = String::new();
        std::io::stdin().read_to_string(&mut prompt).unwrap();
        let stem = prompt
            .split("Phase stem: ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        let output = &args[args.iter().position(|a| a == "-o").unwrap() + 1];
        fs::copy(root.join(stem), output).unwrap();
        print!("{}", fs::read_to_string(root.join("events")).unwrap());
        return;
    }
    let boundary = env::var("FIXTURE_BOUNDARY").unwrap_or_default();
    let binary = env::var_os("FIXTURE_RDM").unwrap();
    if boundary == "missing" && args.first().map(String::as_str) == Some("commit") {
        let result = Command::new(&binary)
            .args([
                "phase",
                "remove",
                "phase-1-first",
                "--roadmap",
                "example",
                "--project",
                "fixture",
            ])
            .env("RDM_SESSION", "concurrent-remover")
            .output()
            .unwrap();
        assert!(result.status.success());
    }
    let result = Command::new(binary).args(&args).output().unwrap();
    if !result.status.success() {
        std::io::stderr().write_all(&result.stderr).unwrap();
        std::process::exit(result.status.code().unwrap_or(1));
    }
    let stop = (boundary == "commit" && args.first().map(String::as_str) == Some("commit"))
        || (boundary == "update"
            && args.get(0).map(String::as_str) == Some("phase")
            && args.get(1).map(String::as_str) == Some("update"));
    if stop {
        let child = Command::new(env::current_exe().unwrap())
            .arg("--hold")
            .spawn()
            .unwrap();
        fs::write(
            root.join("boundary"),
            format!(
                "{}\n{}\n{}",
                std::process::id(),
                child.id(),
                env::var("RDM_SESSION").unwrap()
            ),
        )
        .unwrap();
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    std::io::stdout().write_all(&result.stdout).unwrap();
}
