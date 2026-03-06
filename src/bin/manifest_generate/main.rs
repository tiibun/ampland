mod common;
mod node;
mod python;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::common::{parse_args, selected_tools_label, write_manifest, ToolSelector};

fn main() {
    if let Err(err) = run() {
        eprintln!("manifest-generate: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    eprintln!(
        "manifest-generate: generating {} into {}",
        selected_tools_label(args.tool),
        args.output_dir.display()
    );
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| err.to_string())?;
    let mut written = Vec::new();

    if matches!(args.tool, None | Some(ToolSelector::Node)) {
        let path = args.output_dir.join("node.toml");
        eprintln!("manifest-generate: node generation started");
        let manifest = node::generate_node_manifest(&generated_at)?;
        write_manifest(&path, &manifest)?;
        eprintln!("manifest-generate: node generation finished");
        written.push(path);
    }

    if matches!(args.tool, None | Some(ToolSelector::Python)) {
        let path = args.output_dir.join("python.toml");
        eprintln!("manifest-generate: python generation started");
        let manifest = python::generate_python_manifest(&generated_at)?;
        write_manifest(&path, &manifest)?;
        eprintln!("manifest-generate: python generation finished");
        written.push(path);
    }

    let summary = written
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(" and ");
    println!("Wrote {summary}");

    Ok(())
}
