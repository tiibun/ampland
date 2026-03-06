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
    let mut outcomes = Vec::new();

    if matches!(args.tool, None | Some(ToolSelector::Node)) {
        let path = args.output_dir.join("node.toml");
        eprintln!("manifest-generate: node generation started");
        let manifest = node::generate_node_manifest(&generated_at)?;
        let outcome = write_manifest(&path, &manifest)?;
        eprintln!(
            "manifest-generate: node generation finished ({})",
            outcome.label()
        );
        outcomes.push(outcome.summary(&path));
    }

    if matches!(args.tool, None | Some(ToolSelector::Python)) {
        let path = args.output_dir.join("python.toml");
        eprintln!("manifest-generate: python generation started");
        let manifest = python::generate_python_manifest(&generated_at)?;
        let outcome = write_manifest(&path, &manifest)?;
        eprintln!(
            "manifest-generate: python generation finished ({})",
            outcome.label()
        );
        outcomes.push(outcome.summary(&path));
    }

    println!("{}", outcomes.join(", "));

    Ok(())
}
