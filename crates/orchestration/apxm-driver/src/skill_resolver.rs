//! Skill resolver for assembling node workspaces.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, Value};

pub struct SkillResolver {
    skills_dir: PathBuf,
    skills: HashMap<String, PathBuf>,
}

impl SkillResolver {
    pub fn new(project_root: &Path) -> io::Result<Self> {
        let skills_dir = project_root.join(".agents/skills");
        if !skills_dir.is_dir() {
            return Ok(Self {
                skills_dir,
                skills: HashMap::new(),
            });
        }

        let mut skills = HashMap::new();
        Self::scan_skills_recursive(&skills_dir, &skills_dir, &mut skills)?;

        Ok(Self { skills_dir, skills })
    }

    fn scan_skills_recursive(
        base: &Path,
        current: &Path,
        skills: &mut HashMap<String, PathBuf>,
    ) -> io::Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let skill_md = path.join("SKILL.md");
            if skill_md.is_file() {
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                skills.insert(rel, path.clone());
            }

            Self::scan_skills_recursive(base, &path, skills)?;
        }

        Ok(())
    }

    pub fn resolve(
        &self,
        op_type: AISOperationType,
        attributes: &HashMap<String, Value>,
    ) -> Vec<PathBuf> {
        let mut resolved = Vec::new();

        if op_type == AISOperationType::SpawnAgent {
            if let Some(profile) = string_attr(attributes, graph_attrs::PROFILE) {
                self.push_skill(&mut resolved, profile);
            }
        }

        if op_type == AISOperationType::Communicate {
            if let Some(recipient) = string_attr(attributes, graph_attrs::RECIPIENT) {
                self.push_skill(&mut resolved, recipient);
            }
        }

        if op_type == AISOperationType::InvTool {
            if let Some(capability) = string_attr(attributes, graph_attrs::CAPABILITY) {
                self.push_skill(&mut resolved, capability);
            }
        }

        self.push_skill(&mut resolved, &op_type_key(op_type));
        resolved
    }

    pub fn skill_name(&self, skill_path: &Path) -> String {
        skill_path
            .strip_prefix(&self.skills_dir)
            .unwrap_or(skill_path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/")
    }

    pub fn copy_skill_to(&self, skill_path: &Path, target_dir: &Path) -> io::Result<()> {
        let skill_name = self.skill_name(skill_path);
        let destination = target_dir.join(&skill_name);
        copy_dir_recursive(skill_path, &destination)
    }

    fn push_skill(&self, resolved: &mut Vec<PathBuf>, key: &str) {
        if let Some(path) = self.skills.get(key)
            && !resolved.iter().any(|existing| existing == path)
        {
            resolved.push(path.clone());
        }
    }
}

fn string_attr<'a>(attributes: &'a HashMap<String, Value>, key: &str) -> Option<&'a str> {
    attributes.get(key).and_then(|value| value.as_str())
}

fn op_type_key(op_type: AISOperationType) -> String {
    let debug = format!("{:?}", op_type);
    let mut result = String::with_capacity(debug.len() + 4);
    for (idx, ch) in debug.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if idx > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if src_path.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
