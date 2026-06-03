use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

const EXAMPLES_DIR: &str = "examples";
const SKILL_URI_PREFIX: &str = "skill://";
const SKILL_ROOT_FLAG: &str = "--skill-root";
const SKILL_ROOTS_ENV: &str = "APXM_SKILL_ROOTS";
const BUILTIN_SKILL_ROOT_ENV: &str = env!("APXM_BUILTIN_SKILL_ROOT");
const USER_INSTALL_DIR: &str = ".apxm/libs";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MimeType {
    Json,
    Markdown,
    Toml,
    PlainText,
}

impl MimeType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Markdown => "text/markdown",
            Self::Toml => "application/toml",
            Self::PlainText => "text/plain",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StaticSkillResource {
    Source,
    Manifest,
    Prompt,
    Schema,
}

impl StaticSkillResource {
    const FILE_RESOURCES: [Self; 3] = [Self::Source, Self::Prompt, Self::Schema];

    const fn relative_path(self) -> &'static str {
        match self {
            Self::Source => "SKILL.md",
            Self::Manifest => "_manifest",
            Self::Prompt => "prompt.md",
            Self::Schema => "schema.json",
        }
    }

    const fn mime_type(self) -> MimeType {
        match self {
            Self::Source | Self::Prompt => MimeType::Markdown,
            Self::Manifest | Self::Schema => MimeType::Json,
        }
    }

    fn from_path(path: &Path) -> Option<Self> {
        Self::FILE_RESOURCES
            .into_iter()
            .chain([Self::Manifest])
            .find(|resource| path == Path::new(resource.relative_path()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SkillResourcePath {
    Static(StaticSkillResource),
    Example(PathBuf),
}

impl SkillResourcePath {
    fn parse(path: &str) -> Result<Self, SkillResourceError> {
        let candidate = validate_resource_path(path)?;
        if let Some(resource) = StaticSkillResource::from_path(&candidate) {
            return Ok(Self::Static(resource));
        }
        if candidate
            .components()
            .next()
            .is_some_and(|component| component.as_os_str() == EXAMPLES_DIR)
        {
            return Ok(Self::Example(candidate));
        }
        Err(SkillResourceError::NotFound(format!(
            "unsupported skill resource path: {path}"
        )))
    }

    fn relative_path(&self) -> &Path {
        match self {
            Self::Static(resource) => Path::new(resource.relative_path()),
            Self::Example(path) => path.as_path(),
        }
    }

    fn display_path(&self) -> &str {
        match self {
            Self::Static(resource) => resource.relative_path(),
            Self::Example(path) => path.to_str().unwrap_or(EXAMPLES_DIR),
        }
    }

    fn mime_type(&self) -> MimeType {
        match self {
            Self::Static(resource) => resource.mime_type(),
            Self::Example(path) => mime_type_for_path(path),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResourcePackage {
    pub(crate) skill_id: String,
    pub(crate) version: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) package_dir: PathBuf,
    pub(crate) manifest: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SkillResource {
    pub(crate) uri: String,
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(rename = "mimeType")]
    pub(crate) mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SkillResourceContent {
    pub(crate) uri: String,
    #[serde(rename = "mimeType")]
    pub(crate) mime_type: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SkillResourceError {
    InvalidUri(String),
    NotFound(String),
    Ambiguous(String),
    Io(String),
}

impl std::fmt::Display for SkillResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUri(message)
            | Self::NotFound(message)
            | Self::Ambiguous(message)
            | Self::Io(message) => f.write_str(message),
        }
    }
}

pub(crate) fn builtin_skill_root() -> PathBuf {
    PathBuf::from(BUILTIN_SKILL_ROOT_ENV)
}

pub(crate) fn parse_skill_roots(args: &[String]) -> Vec<PathBuf> {
    let mut roots = parse_cli_skill_roots(args);

    if let Some(env_roots) = std::env::var_os(SKILL_ROOTS_ENV) {
        roots.extend(std::env::split_paths(&env_roots));
    }

    if let Some(home) = std::env::var_os("HOME") {
        let user_libs = PathBuf::from(home).join(USER_INSTALL_DIR);
        if user_libs.is_dir() {
            roots.push(user_libs);
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        if roots.is_empty() {
            let repo_skills = current_dir.join(".agents").join("skills");
            if repo_skills.is_dir() {
                roots.push(repo_skills);
            }
        }
    }

    dedupe_paths(roots)
}

pub(crate) fn prepend_builtin_skill_root(configured_roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut roots = vec![builtin_skill_root()];
    roots.extend(configured_roots);
    dedupe_paths(roots)
}

pub(crate) fn parse_cli_skill_roots(args: &[String]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if arg == SKILL_ROOT_FLAG
            && let Some(root) = args.get(index + 1)
        {
            roots.push(PathBuf::from(root));
        }
    }
    dedupe_paths(roots)
}

// Used by the sibling `apxm_mcp` binary (which #[path]-includes this module)
// and by in-crate tests; invisible to the `apxm-server` binary's dead_code lint.
#[allow(dead_code)]
pub(crate) fn scan_resource_packages(roots: &[PathBuf]) -> Vec<ResourcePackage> {
    let mut package_dirs = Vec::new();
    for root in roots {
        find_manifest_dirs(root, &mut package_dirs);
    }
    package_dirs.sort();
    package_dirs
        .into_iter()
        .filter_map(|package_dir| resource_package_from_manifest(&package_dir))
        .collect()
}

pub(crate) fn list_skill_resources(packages: &[ResourcePackage]) -> Vec<SkillResource> {
    let uri_ids = uri_ids(packages);
    let mut resources = Vec::new();
    for package in packages {
        let Some(uri_id) = uri_ids.get(&package_key(package)) else {
            continue;
        };
        for resource in StaticSkillResource::FILE_RESOURCES {
            push_file_resource(package, uri_id, resource, &mut resources);
        }
        let manifest_resource = StaticSkillResource::Manifest;
        resources.push(SkillResource {
            uri: skill_uri(uri_id, manifest_resource.relative_path()),
            name: resource_name(package, manifest_resource.relative_path()),
            description: package
                .description
                .as_ref()
                .map(|description| format!("Normalized skill manifest: {description}"))
                .or_else(|| Some("Normalized skill manifest".to_string())),
            mime_type: manifest_resource.mime_type().as_str().to_string(),
        });
        list_example_resources(package, uri_id, &mut resources);
    }
    resources.sort_by(|left, right| left.uri.cmp(&right.uri));
    resources
}

pub(crate) fn resolve_skill_uri(
    packages: &[ResourcePackage],
    uri: &str,
) -> Result<SkillResourceContent, SkillResourceError> {
    let (requested_id, resource_path) = parse_skill_uri(uri)?;
    let resource_path = SkillResourcePath::parse(&resource_path)?;
    let package = find_package(packages, requested_id)?;
    let resolved_uri = skill_uri(
        &resource_uri_id(packages, package),
        resource_path.display_path(),
    );

    if resource_path == SkillResourcePath::Static(StaticSkillResource::Manifest) {
        let text = serde_json::to_string_pretty(&package.manifest).map_err(|error| {
            SkillResourceError::Io(format!("failed to encode manifest: {error}"))
        })?;
        return Ok(SkillResourceContent {
            uri: resolved_uri,
            mime_type: resource_path.mime_type().as_str().to_string(),
            text,
        });
    }

    let path = package.package_dir.join(resource_path.relative_path());
    if !path.is_file() || is_symlink(&path) {
        return Err(SkillResourceError::NotFound(format!(
            "skill resource not found: {uri}"
        )));
    }
    let text = fs::read_to_string(&path).map_err(|error| {
        SkillResourceError::Io(format!("failed to read skill resource {uri}: {error}"))
    })?;
    Ok(SkillResourceContent {
        uri: resolved_uri,
        mime_type: resource_path.mime_type().as_str().to_string(),
        text,
    })
}

pub(crate) fn resource_package(
    skill_id: impl Into<String>,
    version: Option<String>,
    display_name: Option<String>,
    description: Option<String>,
    package_dir: PathBuf,
    manifest: JsonValue,
) -> ResourcePackage {
    ResourcePackage {
        skill_id: skill_id.into(),
        version,
        display_name,
        description,
        package_dir,
        manifest,
    }
}

// Helper for `scan_resource_packages`; reachable only via that public entry
// point (used by `apxm_mcp` binary + tests).
#[allow(dead_code)]
fn resource_package_from_manifest(package_dir: &Path) -> Option<ResourcePackage> {
    let manifest_path = package_dir.join(apxm_skill::MANIFEST_FILE);
    let manifest = apxm_skill::parse_manifest_file(&manifest_path).ok()?;
    let manifest_value = serde_json::to_value(&manifest).ok()?;
    Some(resource_package(
        manifest.skill_id,
        Some(manifest.version),
        manifest.display_name,
        manifest.description,
        package_dir.to_path_buf(),
        manifest_value,
    ))
}

fn push_file_resource(
    package: &ResourcePackage,
    uri_id: &str,
    resource: StaticSkillResource,
    resources: &mut Vec<SkillResource>,
) {
    let relative = resource.relative_path();
    let path = package.package_dir.join(relative);
    if !path.is_file() || is_symlink(&path) {
        return;
    }
    resources.push(SkillResource {
        uri: skill_uri(uri_id, relative),
        name: resource_name(package, relative),
        description: resource_description(package, relative),
        mime_type: resource.mime_type().as_str().to_string(),
    });
}

fn list_example_resources(
    package: &ResourcePackage,
    uri_id: &str,
    resources: &mut Vec<SkillResource>,
) {
    let examples_dir = package.package_dir.join(EXAMPLES_DIR);
    if !examples_dir.is_dir() || is_symlink(&examples_dir) {
        return;
    }
    let mut files = Vec::new();
    collect_regular_files(&examples_dir, &mut files);
    files.sort();
    for path in files {
        let Ok(relative_to_examples) = path.strip_prefix(&examples_dir) else {
            continue;
        };
        let relative = Path::new(EXAMPLES_DIR).join(relative_to_examples);
        let Some(relative_text) = relative.to_str() else {
            continue;
        };
        let resource_path = SkillResourcePath::Example(relative.clone());
        resources.push(SkillResource {
            uri: skill_uri(uri_id, relative_text),
            name: resource_name(package, relative_text),
            description: resource_description(package, relative_text),
            mime_type: resource_path.mime_type().as_str().to_string(),
        });
    }
}

fn collect_regular_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_symlink(&path) {
            continue;
        }
        if path.is_dir() {
            collect_regular_files(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn parse_skill_uri(uri: &str) -> Result<(&str, String), SkillResourceError> {
    let Some(rest) = uri.strip_prefix(SKILL_URI_PREFIX) else {
        return Err(SkillResourceError::InvalidUri(format!(
            "skill resource URI must start with {SKILL_URI_PREFIX}"
        )));
    };
    let Some((skill_id, resource_path)) = rest.split_once('/') else {
        return Err(SkillResourceError::InvalidUri(
            "skill resource URI must include a resource path".to_string(),
        ));
    };
    if skill_id.trim().is_empty() || resource_path.trim().is_empty() {
        return Err(SkillResourceError::InvalidUri(
            "skill resource URI has an empty skill id or resource path".to_string(),
        ));
    }
    Ok((skill_id, resource_path.to_string()))
}

fn find_package<'a>(
    packages: &'a [ResourcePackage],
    requested_id: &str,
) -> Result<&'a ResourcePackage, SkillResourceError> {
    let (skill_id, requested_version) = split_requested_id(requested_id);
    let mut matches: Vec<&ResourcePackage> = packages
        .iter()
        .filter(|package| package.skill_id == skill_id)
        .filter(|package| {
            requested_version.is_none() || package.version.as_deref() == requested_version
        })
        .collect();

    match matches.len() {
        0 => Err(SkillResourceError::NotFound(format!(
            "skill resource package not found: {requested_id}"
        ))),
        1 => Ok(matches.remove(0)),
        _ => Err(SkillResourceError::Ambiguous(format!(
            "skill id has multiple versions; request {skill_id}@<version>"
        ))),
    }
}

fn validate_resource_path(path: &str) -> Result<PathBuf, SkillResourceError> {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        return Err(SkillResourceError::InvalidUri(
            "skill resource path must be relative".to_string(),
        ));
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(SkillResourceError::InvalidUri(
                    "skill resource path must not contain traversal components".to_string(),
                ));
            }
        }
    }
    Ok(candidate)
}

// Helper for `scan_resource_packages`; reachable only via that public entry
// point (used by `apxm_mcp` binary + tests).
#[allow(dead_code)]
fn find_manifest_dirs(root: &Path, packages: &mut Vec<PathBuf>) {
    if !root.exists() || is_symlink(root) {
        return;
    }
    if root.join(apxm_skill::MANIFEST_FILE).is_file() {
        packages.push(root.to_path_buf());
        return;
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && !is_symlink(&path) {
            dirs.push(path);
        }
    }
    dirs.sort();
    for dir in dirs {
        find_manifest_dirs(&dir, packages);
    }
}

fn uri_ids(packages: &[ResourcePackage]) -> HashMap<(String, Option<String>, PathBuf), String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for package in packages {
        *counts.entry(package.skill_id.as_str()).or_default() += 1;
    }

    let mut ids = HashMap::new();
    for package in packages {
        let id = if counts
            .get(package.skill_id.as_str())
            .copied()
            .unwrap_or_default()
            > 1
        {
            match package.version.as_deref() {
                Some(version) => format!("{}@{}", package.skill_id, version),
                None => package.skill_id.clone(),
            }
        } else {
            package.skill_id.clone()
        };
        ids.insert(package_key(package), id);
    }
    ids
}

fn resource_uri_id(packages: &[ResourcePackage], package: &ResourcePackage) -> String {
    uri_ids(packages)
        .remove(&package_key(package))
        .unwrap_or_else(|| package.skill_id.clone())
}

fn package_key(package: &ResourcePackage) -> (String, Option<String>, PathBuf) {
    (
        package.skill_id.clone(),
        package.version.clone(),
        package.package_dir.clone(),
    )
}

pub(crate) fn skill_uri(uri_id: &str, relative: &str) -> String {
    format!("{SKILL_URI_PREFIX}{uri_id}/{relative}")
}

fn resource_name(package: &ResourcePackage, relative: &str) -> String {
    let name = package
        .display_name
        .as_deref()
        .unwrap_or(package.skill_id.as_str());
    format!("{name}: {relative}")
}

fn resource_description(package: &ResourcePackage, relative: &str) -> Option<String> {
    package
        .description
        .as_ref()
        .map(|description| format!("{description} ({relative})"))
}

fn mime_type_for_path(path: &Path) -> MimeType {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => MimeType::Json,
        Some("md") => MimeType::Markdown,
        Some("toml") => MimeType::Toml,
        Some("txt") => MimeType::PlainText,
        _ => MimeType::PlainText,
    }
}

fn split_requested_id(requested_id: &str) -> (&str, Option<&str>) {
    requested_id
        .rsplit_once('@')
        .map_or((requested_id, None), |(skill_id, version)| {
            (skill_id, Some(version))
        })
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeMap::new();
    for path in paths {
        seen.entry(path.clone()).or_insert(path);
    }
    seen.into_values().collect()
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_resource_skill(root: &Path, package_name: &str, version: &str, source: &str) {
        let skill_dir = root.join(package_name);
        fs::create_dir_all(skill_dir.join(EXAMPLES_DIR)).expect("skill dir");
        fs::write(
            skill_dir.join(apxm_skill::MANIFEST_FILE),
            format!(
                r#"
skill_id = "demo-skill"
version = "{version}"
display_name = "Demo Skill"
description = "Demo resource package"
entry_flow = "main"
"#
            ),
        )
        .expect("manifest");
        fs::write(
            skill_dir.join(StaticSkillResource::Source.relative_path()),
            source,
        )
        .expect("source");
        fs::write(skill_dir.join(EXAMPLES_DIR).join("demo.json"), "{}\n").expect("example");
    }

    #[test]
    fn builtin_skill_root_points_at_checked_in_bundle_dir() {
        let root = builtin_skill_root();

        assert!(
            root.ends_with("skills"),
            "builtin skill root should be the crate skills directory: {}",
            root.display()
        );
    }

    #[test]
    fn rejects_resource_path_traversal() {
        let traversal_uri = skill_uri("demo", "../secret.txt");
        let err = resolve_skill_uri(&[], &traversal_uri).expect_err("traversal");

        assert!(matches!(err, SkillResourceError::InvalidUri(_)));
    }

    #[test]
    fn lists_and_reads_basic_skill_resources() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_resource_skill(temp.path(), "demo", "0.1.0", "# Demo\n");
        let packages = scan_resource_packages(&[temp.path().to_path_buf()]);

        let resources = list_skill_resources(&packages);
        let source_uri = skill_uri("demo-skill", StaticSkillResource::Source.relative_path());
        let example_relative = Path::new(EXAMPLES_DIR).join("demo.json");
        let example_uri = skill_uri(
            "demo-skill",
            example_relative.to_str().expect("example URI"),
        );

        assert!(
            resources.iter().any(|resource| resource.uri == source_uri),
            "resources: {resources:?}"
        );
        assert!(
            resources.iter().any(|resource| resource.uri == example_uri),
            "resources: {resources:?}"
        );
        let source = resolve_skill_uri(&packages, &source_uri).expect("source resource");
        assert_eq!(source.mime_type, MimeType::Markdown.as_str());
        assert_eq!(source.text, "# Demo\n");
    }

    #[test]
    fn duplicate_skill_ids_list_versioned_uris_and_require_versioned_reads() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_resource_skill(temp.path(), "demo-v1", "0.1.0", "# Demo v1\n");
        write_resource_skill(temp.path(), "demo-v2", "0.2.0", "# Demo v2\n");
        let packages = scan_resource_packages(&[temp.path().to_path_buf()]);

        let resources = list_skill_resources(&packages);
        let v1_uri = skill_uri(
            "demo-skill@0.1.0",
            StaticSkillResource::Source.relative_path(),
        );
        let v2_uri = skill_uri(
            "demo-skill@0.2.0",
            StaticSkillResource::Source.relative_path(),
        );
        assert!(
            resources.iter().any(|resource| resource.uri == v1_uri),
            "resources: {resources:?}"
        );
        assert!(
            resources.iter().any(|resource| resource.uri == v2_uri),
            "resources: {resources:?}"
        );

        let unversioned = resolve_skill_uri(
            &packages,
            &skill_uri("demo-skill", StaticSkillResource::Source.relative_path()),
        )
        .expect_err("unversioned duplicate skill id should be ambiguous");
        assert!(matches!(unversioned, SkillResourceError::Ambiguous(_)));

        let versioned = resolve_skill_uri(&packages, &v2_uri).expect("versioned source resource");
        assert_eq!(versioned.uri, v2_uri);
        assert_eq!(versioned.text, "# Demo v2\n");
    }
}
