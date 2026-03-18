//! Universal Smithy model code generator.
//!
//! Reads an AWS Smithy JSON AST model and generates Rust source files
//! based on a TOML service configuration.

mod codegen;
mod config;
mod model;
mod shapes;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{ServiceConfig, ServiceConfigFile};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Parse named arguments: --config, --model, --output
    let config_path = find_arg(&args, "--config");
    let model_path_arg = find_arg(&args, "--model");
    let output_dir_arg = find_arg(&args, "--output");

    // Determine config path, with backward-compatible fallback
    let config_path = config_path.map_or_else(
        || {
            // Legacy positional args: codegen [model_path] [output_dir]
            // Default to S3 config
            PathBuf::from("services/s3.toml")
        },
        PathBuf::from,
    );

    // Read and parse the TOML config
    let config_toml = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    let config_file: ServiceConfigFile =
        toml::from_str(&config_toml).context("Failed to parse TOML config")?;
    let service_config =
        ServiceConfig::from_file(config_file).context("Failed to build service config")?;

    // Determine model and output paths (named args > legacy positional > defaults)
    let model_path = model_path_arg.map_or_else(
        || {
            args.get(1)
                .filter(|a| !a.starts_with("--"))
                .map_or_else(|| PathBuf::from("smithy-model/s3.json"), PathBuf::from)
        },
        PathBuf::from,
    );

    let output_dir = output_dir_arg.map_or_else(
        || {
            args.get(2).filter(|a| !a.starts_with("--")).map_or_else(
                || PathBuf::from("../crates/ruststack-s3-model/src"),
                PathBuf::from,
            )
        },
        PathBuf::from,
    );

    eprintln!("Reading Smithy model from: {}", model_path.display());
    eprintln!("Writing output to: {}", output_dir.display());

    // Read and parse the Smithy JSON model.
    let model_json = fs::read_to_string(&model_path)
        .with_context(|| format!("Failed to read model file: {}", model_path.display()))?;

    let smithy_model: model::SmithyModel =
        serde_json::from_str(&model_json).context("Failed to parse Smithy JSON model")?;

    eprintln!("Parsed model: {} shapes", smithy_model.shapes.len());

    // Resolve shapes and types.
    let resolved = shapes::resolve_model(&smithy_model, &service_config)
        .context("Failed to resolve model shapes")?;

    eprintln!(
        "Resolved: {} operations, {} enums, {} shared structs, {} input structs, {} output structs",
        resolved.operations.len(),
        resolved.enums.len(),
        resolved.shared_structs.len(),
        resolved.input_structs.len(),
        resolved.output_structs.len(),
    );

    // Generate code.
    let files =
        codegen::generate_all(&resolved, &service_config).context("Failed to generate code")?;

    // Write output files.
    for (rel_path, content) in &files {
        let full_path = output_dir.join(rel_path);
        ensure_parent_dir(&full_path)?;
        fs::write(&full_path, content)
            .with_context(|| format!("Failed to write {}", full_path.display()))?;
        eprintln!("  Wrote: {}", full_path.display());
    }

    eprintln!("Code generation complete. {} files written.", files.len());

    Ok(())
}

/// Find a named argument value (e.g., `--config path/to/file`).
fn find_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Ensure the parent directory of a path exists.
fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }
    Ok(())
}
