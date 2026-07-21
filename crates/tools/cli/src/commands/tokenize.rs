//! Token counting helpers exposed through the CLI.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;

const TOKEN_ACCOUNTING_UNAVAILABLE: &str =
    "standalone token accounting requires configured tokenizer evidence";

pub fn tokenize_command(
    text: Option<String>,
    file: Option<PathBuf>,
    model: Option<String>,
    json_output: bool,
) -> Result<()> {
    let input = match (text, file) {
        (Some(text), None) => text,
        (None, Some(path)) => std::fs::read_to_string(&path)
            .map_err(|err| anyhow::anyhow!("failed to read {}: {err}", path.display()))?,
        (None, None) => anyhow::bail!("provide text or --file"),
        (Some(_), Some(_)) => anyhow::bail!("provide either text or --file, not both"),
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "model": model,
                "tokenizer": "unavailable",
                "tokens": null,
                "reason": TOKEN_ACCOUNTING_UNAVAILABLE,
                "chars": input.chars().count(),
                "bytes": input.len(),
            }))?
        );
    } else {
        println!("tokens: unavailable");
        println!("tokenizer: unavailable");
        println!("reason: {TOKEN_ACCOUNTING_UNAVAILABLE}");
        if let Some(model) = model {
            println!("model: {model}");
        }
    }

    Ok(())
}
