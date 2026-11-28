//! Builds the test binaries `cargo test` would run with the arguments given, then runs them all at
//! once, as cargo would one after another: each from its package's folder, its output shown whole
//! once it is done, in the order cargo built them. Fails if the build or any binary fails.

use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Output, Stdio};

use cargo_metadata::Message;

fn main() -> ExitCode {
    let cargo_args: Vec<String> = std::env::args().skip(1).collect();
    let Some(binaries) = built(&cargo_args) else {
        return ExitCode::FAILURE;
    };
    let ran: Vec<(TestBinary, std::io::Result<Output>)> = std::thread::scope(|scope| {
        let running: Vec<_> = binaries
            .into_iter()
            .map(|binary| {
                scope.spawn(move || {
                    let output = Command::new(&binary.executable)
                        .current_dir(&binary.folder)
                        .output();
                    (binary, output)
                })
            })
            .collect();
        running
            .into_iter()
            .map(|run| run.join().expect("a test binary's runner thread"))
            .collect()
    });
    let mut failed = Vec::new();
    for (binary, output) in ran {
        eprintln!(
            "     Running {} ({})",
            binary.source.display(),
            binary.executable.display()
        );
        match output {
            Ok(output) => {
                std::io::stdout().write_all(&output.stdout).expect("stdout");
                std::io::stderr().write_all(&output.stderr).expect("stderr");
                if !output.status.success() {
                    failed.push(binary.source);
                }
            }
            Err(error) => {
                eprintln!("{} didn't start: {error}", binary.executable.display());
                failed.push(binary.source);
            }
        }
    }
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!("test binaries failed:");
        for source in failed {
            eprintln!("  {}", source.display());
        }
        ExitCode::FAILURE
    }
}

struct TestBinary {
    source: PathBuf,
    executable: PathBuf,
    folder: PathBuf,
}

/// The test binaries of a `cargo test` with `cargo_args`, built; none if the build fails.
fn built(cargo_args: &[String]) -> Option<Vec<TestBinary>> {
    let mut build = Command::new("cargo")
        .arg("test")
        .args(cargo_args)
        .args(["--no-run", "--message-format=json-render-diagnostics"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("cargo starts");
    let messages = BufReader::new(build.stdout.take().expect("cargo's output"));
    let binaries = Message::parse_stream(messages)
        .filter_map(|message| match message.expect("a message from cargo") {
            Message::CompilerArtifact(artifact) if artifact.profile.test => {
                let executable = artifact.executable?;
                Some(TestBinary {
                    source: artifact.target.src_path.into(),
                    executable: executable.into(),
                    folder: artifact.manifest_path.parent()?.into(),
                })
            }
            _ => None,
        })
        .collect();
    build
        .wait()
        .expect("cargo finishes")
        .success()
        .then_some(binaries)
}
