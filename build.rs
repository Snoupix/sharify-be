use std::env;
use std::fs;
use std::process::{Command, Stdio};

const PROTO_DIR: &str = "proto/";
const PROTO_TS_OUT: &str = "./proto_ts";
const PROTOC_TS_PLUGIN: &str = concat!(
    std::env!("HOME"),
    "/.local/share/pnpm/global/5/node_modules/ts-proto/protoc-gen-ts_proto"
);

fn main() -> std::io::Result<()> {
    let proto_files = fs::read_dir(PROTO_DIR)?
        .filter_map(|entry| {
            entry
                .map(|file| format!("{PROTO_DIR}{}", file.file_name().to_str().unwrap()))
                .ok()
        })
        .filter(|file_path| file_path.ends_with(".proto"))
        .collect::<Vec<_>>();

    prost_build::compile_protos(&proto_files, &[PROTO_DIR])?;

    for file in proto_files {
        println!("cargo::rerun-if-changed={file}");

        let ts_compile = Command::new("protoc")
            .args(&[
                format!("--plugin={}", PROTOC_TS_PLUGIN),
                format!("--ts_proto_out={}", PROTO_TS_OUT),
                format!("-I={}", PROTO_DIR),
                file,
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .status();

        if ts_compile.is_err()
            || ts_compile.is_ok_and(|status| status.code().is_some_and(|code| code != 0))
        {
            eprintln!("Failed to compile .proto file to TS");
        }
    }

    let out_dir = env::var("OUT_DIR").unwrap();

    let output_files = fs::read_dir(out_dir.clone())?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();

    for file in output_files {
        if file.file_name() == "_.rs" {
            continue;
        }

        fs::copy(
            file.path().to_str().unwrap(),
            format!("src/proto/{}", file.file_name().to_str().unwrap()),
        )?;
    }

    Ok(())
}
