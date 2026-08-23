use crate::common::agents_root;

/// Every JSON document published under `contracts/<directory>/`, sorted.
#[must_use]
pub(crate) fn published_contract_files(directory: &str) -> Vec<String> {
    let root = agents_root().join("contracts").join(directory);
    let mut files: Vec<String> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("read {}: {e}", root.display()))
        .map(|entry| entry.expect("directory entry").file_name())
        .filter_map(|name| name.to_str().map(str::to_owned))
        .filter(|name| name.ends_with(".json"))
        .collect();
    files.sort();
    files
}
