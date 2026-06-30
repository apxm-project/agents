/// Display a backend API-key reference safely.
///
/// Registry entries should store `env:VAR` references, not raw keys. Show
/// references verbatim because they are operator-facing identifiers; mask any
/// legacy literal defensively.
pub fn mask_key(reference: &str) -> String {
    if reference.starts_with("env:") {
        return reference.to_string();
    }
    if reference.len() <= 8 {
        "****".to_string()
    } else {
        format!(
            "{}...{}",
            &reference[..4],
            &reference[reference.len() - 4..]
        )
    }
}
