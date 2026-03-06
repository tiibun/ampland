use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let source = manifest_dir.join("src/embedded_shim_main.rs");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("out dir"));
    let target = env::var("TARGET").expect("target");
    let rustc = env::var("RUSTC").expect("rustc");
    let output = out_dir.join(if target.contains("windows") {
        "ampland-shim.exe"
    } else {
        "ampland-shim"
    });

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=build.rs");

    let status = Command::new(rustc)
        .arg("--crate-name")
        .arg("ampland_embedded_shim")
        .arg("--edition=2021")
        .arg("--crate-type")
        .arg("bin")
        .arg("--target")
        .arg(&target)
        .arg("-o")
        .arg(&output)
        .arg(&source)
        .status()
        .expect("compile embedded shim");

    if !status.success() {
        panic!("failed to compile embedded shim");
    }

    println!(
        "cargo:rustc-env=AMPLAND_EMBEDDED_SHIM_PATH={}",
        output.display()
    );
}
