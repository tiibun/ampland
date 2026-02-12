use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use toml::value::Table;
use toml::Value;

fn main() {
    if let Err(err) = run() {
        eprintln!("manifest-merge: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (output, inputs) = parse_args()?;
    if inputs.is_empty() {
        return Err("no input manifests provided".to_string());
    }

    let mut merged = load_manifest(&inputs[0])?;
    let mut seen = collect_tool_names(&merged)?;

    for path in inputs.iter().skip(1) {
        let next = load_manifest(path)?;
        merge_into(&mut merged, next, &mut seen)?;
    }

    let output_text = toml::to_string_pretty(&merged).map_err(|err| err.to_string())?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    fs::write(output, output_text).map_err(|err| err.to_string())?;
    Ok(())
}

fn parse_args() -> Result<(PathBuf, Vec<PathBuf>), String> {
    let mut args = env::args().skip(1);
    let mut output = None;
    let mut inputs = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--output" {
            let value = args
                .next()
                .ok_or_else(|| "missing value for --output".to_string())?;
            output = Some(PathBuf::from(value));
        } else {
            inputs.push(PathBuf::from(arg));
        }
    }

    Ok((output.unwrap_or_else(|| PathBuf::from("installers.toml")), inputs))
}

fn load_manifest(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    toml::from_str(&text).map_err(|err| err.to_string())
}

fn merge_into(merged: &mut Value, next: Value, seen: &mut HashSet<String>) -> Result<(), String> {
    let merged_table = table_mut(merged)?;
    let next_table = table(&next)?;

    let merged_version = get_version(merged_table)?;
    let next_version = get_version(next_table)?;
    if merged_version != next_version {
        return Err(format!(
            "manifest version mismatch: {merged_version} != {next_version}"
        ));
    }

    let merged_generated = get_generated_at(merged_table)?;
    let next_generated = get_generated_at(next_table)?;
    if next_generated > merged_generated {
        merged_table.insert("generated_at".to_string(), Value::String(next_generated));
    }

    let mut next_tools = tools_from_value(next)?;
    let merged_tools = tools_mut(merged_table)?;
    for tool in next_tools.drain(..) {
        let name = tool_name(&tool)?;
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate tool in manifests: {name}"));
        }
        merged_tools.push(tool);
    }

    Ok(())
}

fn collect_tool_names(value: &Value) -> Result<HashSet<String>, String> {
    let table = table(value)?;
    let tools = tools(table)?;
    let mut seen = HashSet::new();
    for tool in tools {
        let name = tool_name(tool)?;
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate tool in manifests: {name}"));
        }
    }
    Ok(seen)
}

fn table(value: &Value) -> Result<&Table, String> {
    value
        .as_table()
        .ok_or_else(|| "manifest is not a table".to_string())
}

fn table_mut(value: &mut Value) -> Result<&mut Table, String> {
    value
        .as_table_mut()
        .ok_or_else(|| "manifest is not a table".to_string())
}

fn tools(table: &Table) -> Result<&Vec<Value>, String> {
    table
        .get("tool")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifest.tool must be an array".to_string())
}

fn tools_mut(table: &mut Table) -> Result<&mut Vec<Value>, String> {
    table
        .get_mut("tool")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "manifest.tool must be an array".to_string())
}

fn tools_from_value(mut value: Value) -> Result<Vec<Value>, String> {
    let table = value
        .as_table_mut()
        .ok_or_else(|| "manifest is not a table".to_string())?;
    match table.remove("tool") {
        Some(Value::Array(tools)) => Ok(tools),
        _ => Err("manifest.tool must be an array".to_string()),
    }
}

fn tool_name(value: &Value) -> Result<String, String> {
    let table = value
        .as_table()
        .ok_or_else(|| "tool entry is not a table".to_string())?;
    table
        .get("name")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .ok_or_else(|| "tool.name is required".to_string())
}

fn get_version(table: &Table) -> Result<i64, String> {
    table
        .get("version")
        .and_then(Value::as_integer)
        .ok_or_else(|| "manifest.version is required".to_string())
}

fn get_generated_at(table: &Table) -> Result<String, String> {
    table
        .get("generated_at")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .ok_or_else(|| "manifest.generated_at is required".to_string())
}
