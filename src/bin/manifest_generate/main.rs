mod common;
mod node;
mod python;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::common::{parse_args, write_manifest};

fn main() {
    if let Err(err) = run() {
        eprintln!("manifest-generate: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let output_dir = parse_args()?;
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| err.to_string())?;

    let node_manifest = node::generate_node_manifest(&generated_at)?;
    write_manifest(&output_dir.join("node.toml"), &node_manifest)?;

    let python_manifest = python::generate_python_manifest(&generated_at)?;
    write_manifest(&output_dir.join("python.toml"), &python_manifest)?;

    println!(
        "Wrote {} and {}",
        output_dir.join("node.toml").display(),
        output_dir.join("python.toml").display()
    );

    Ok(())
}
