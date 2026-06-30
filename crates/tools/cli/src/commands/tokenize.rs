//! Token counting helpers exposed through the CLI.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;

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

    let tokenizer = apxm_compiler::token_estimate::tokenizer_name_for_model(model.as_deref());
    let tokens = apxm_compiler::token_estimate::count_text_tokens(model.as_deref(), &input);

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "model": model,
                "tokenizer": tokenizer,
                "tokens": tokens,
                "chars": input.chars().count(),
                "bytes": input.len(),
            }))?
        );
    } else {
        println!("tokens: {tokens}");
        println!("tokenizer: {tokenizer}");
        if let Some(model) = model {
            println!("model: {model}");
        }
    }

    Ok(())
}
