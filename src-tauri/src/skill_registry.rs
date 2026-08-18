//! File-based, version-aware Skill registry.
//!
//! The shared directory is the source of truth.  We only copy complete
//! `SKILL.md` files after a preview, never infer deployable skills from tool
//! traces or overwrite a target without an explicit `overwrite` flag.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SkillItem {
    pub source: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub hash: String,
    pub version: u32,
    pub status: String,
    pub assigned_agents: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SkillManifest {
    version: u32,
    status: String,
    assigned_agents: Vec<String>,
    current_hash: String,
    updated_at: String,
}

const SKILL_CATALOG_SETTING_KEY: &str = "skill_catalog";

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SkillSyncPreview {
    pub target: String,
    pub create: Vec<SkillItem>,
    pub update: Vec<SkillItem>,
    pub unchanged: Vec<SkillItem>,
    pub conflict: Vec<SkillItem>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SkillDocument {
    pub item: SkillItem,
    pub content: String,
}

fn shared_root() -> PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-manager")
        .join("shared-skills")
}

fn manifest_path(skill_path: &Path) -> PathBuf {
    skill_path
        .parent()
        .unwrap_or(skill_path)
        .join("manifest.json")
}

fn version_root(skill_path: &Path) -> PathBuf {
    skill_path.parent().unwrap_or(skill_path).join(".versions")
}

fn default_manifest(hash: &str) -> SkillManifest {
    SkillManifest {
        version: 1,
        status: "draft".into(),
        assigned_agents: Vec::new(),
        current_hash: hash.into(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn read_manifest(skill_path: &Path, hash: &str) -> SkillManifest {
    std::fs::read_to_string(manifest_path(skill_path))
        .ok()
        .and_then(|text| serde_json::from_str::<SkillManifest>(&text).ok())
        .filter(|manifest| manifest.current_hash == hash)
        .unwrap_or_else(|| default_manifest(hash))
}

fn write_manifest(skill_path: &Path, manifest: &SkillManifest) -> Result<(), String> {
    let path = manifest_path(skill_path);
    let parent = path.parent().ok_or("skill manifest has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    std::fs::write(
        path,
        serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn with_manifest(mut item: SkillItem, skill_path: &Path) -> SkillItem {
    let manifest = read_manifest(skill_path, &item.hash);
    item.version = manifest.version;
    item.status = manifest.status;
    item.assigned_agents = manifest.assigned_agents;
    item
}

fn source_roots() -> Vec<(&'static str, PathBuf)> {
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let codex = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"));
    vec![
        ("codex", codex.join("skills")),
        ("claude", home.join(".claude").join("skills")),
        ("qoder", home.join(".qoder").join("skills")),
        ("workbuddy", home.join(".workbuddy").join("skills")),
    ]
}

fn target_root(target: &str) -> Result<PathBuf, String> {
    source_roots()
        .into_iter()
        .find_map(|(name, root)| (name == target).then_some(root))
        .ok_or_else(|| format!("unsupported skill target: {target}"))
}

fn safe_name(name: &str) -> String {
    let value: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    value
        .trim_matches('-')
        .to_string()
        .chars()
        .take(80)
        .collect()
}

fn hash(content: &[u8]) -> String {
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_skill(path: &Path, source: &str) -> Result<SkillItem, String> {
    let content = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let text = String::from_utf8(content.clone())
        .map_err(|_| format!("{} is not UTF-8", path.display()))?;
    let mut name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();
    let mut description = String::new();
    if let Some(frontmatter) = text
        .strip_prefix("---")
        .and_then(|rest| rest.split_once("\n---").map(|(head, _)| head))
    {
        for line in frontmatter.lines() {
            if let Some(value) = line.strip_prefix("name:") {
                name = value.trim().trim_matches(['\'', '"']).to_string();
            }
            if let Some(value) = line.strip_prefix("description:") {
                description = value.trim().trim_matches(['\'', '"']).to_string();
            }
        }
    }
    let name = safe_name(&name);
    if name.is_empty() {
        return Err(format!("invalid skill name in {}", path.display()));
    }
    Ok(SkillItem {
        source: source.to_string(),
        name,
        description,
        path: path.display().to_string(),
        hash: hash(&content),
        version: 1,
        status: "draft".into(),
        assigned_agents: Vec::new(),
    })
}

fn collect_skill_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files(&path, output)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            output.push(path);
        }
    }
    Ok(())
}

fn shared_skills() -> Result<Vec<SkillItem>, String> {
    let root = shared_root();
    let mut files = Vec::new();
    collect_skill_files(&root, &mut files)?;
    let skills = files
        .iter()
        .filter_map(|path| {
            let source = path
                .strip_prefix(&root)
                .ok()
                .and_then(|p| p.components().next())
                .and_then(|c| c.as_os_str().to_str())?;
            parse_skill(path, source)
                .ok()
                .map(|item| with_manifest(item, path))
        })
        .collect::<Vec<_>>();
    Ok(skills)
}

fn cache_skills(skills: &[SkillItem]) {
    if let Some(store) = crate::telemetry_store::shared_store() {
        let _ = store.app_setting_set(SKILL_CATALOG_SETTING_KEY, &skills);
    }
}

fn refresh_skill_cache() -> Result<Vec<SkillItem>, String> {
    let skills = shared_skills()?;
    cache_skills(&skills);
    Ok(skills)
}

/// Import existing skills into the Agent Manager shared directory.  Each
/// source retains its own namespace so equal names never silently overwrite.
#[tauri::command]
pub fn skill_scan() -> Result<Vec<SkillItem>, String> {
    let root = shared_root();
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut imported = Vec::new();
    for (source, scan_root) in source_roots() {
        let mut files = Vec::new();
        collect_skill_files(&scan_root, &mut files)?;
        for path in files {
            let skill = parse_skill(&path, source)?;
            let target = root.join(source).join(&skill.name).join("SKILL.md");
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let content = std::fs::read(&path).map_err(|e| e.to_string())?;
            let next_hash = hash(&content);
            let mut manifest = read_manifest(&target, &next_hash);
            if target.exists() {
                let previous = std::fs::read(&target).map_err(|e| e.to_string())?;
                let previous_hash = hash(&previous);
                if previous_hash != next_hash {
                    let previous_manifest = read_manifest(&target, &previous_hash);
                    let snapshot = version_root(&target).join(format!(
                        "v{}-{}.md",
                        previous_manifest.version,
                        &previous_hash[..12]
                    ));
                    if let Some(parent) = snapshot.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(snapshot, previous).map_err(|e| e.to_string())?;
                    manifest = previous_manifest;
                    manifest.version += 1;
                    manifest.status = "draft".into();
                    manifest.current_hash = next_hash.clone();
                    manifest.updated_at = chrono::Utc::now().to_rfc3339();
                }
            }
            write_safely(&target, &content)?;
            write_manifest(&target, &manifest)?;
            imported.push(with_manifest(
                SkillItem {
                    path: target.display().to_string(),
                    ..skill
                },
                &target,
            ));
        }
    }
    // File copies remain necessary for external Agents, but the database
    // catalog makes reopening the Skill view an O(1) SQLite read.
    let _ = refresh_skill_cache()?;
    Ok(imported)
}

#[tauri::command]
pub fn skill_list() -> Result<Vec<SkillItem>, String> {
    if let Some(store) = crate::telemetry_store::shared_store() {
        if let Some(skills) = store.app_setting_get(SKILL_CATALOG_SETTING_KEY) {
            return Ok(skills);
        }
    }
    refresh_skill_cache()
}

/// Reads only a skill already registered in the managed shared directory; the
/// frontend never supplies a raw filesystem path.
#[tauri::command]
pub fn skill_read(source: String, name: String) -> Result<SkillDocument, String> {
    let allowed_source = ["codex", "claude", "qoder", "workbuddy"].contains(&source.as_str());
    let safe = safe_name(&name);
    if !allowed_source || safe != name || safe.is_empty() {
        return Err("invalid shared skill identifier".into());
    }
    let path = shared_root().join(&source).join(&safe).join("SKILL.md");
    let item = with_manifest(parse_skill(&path, &source)?, &path);
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    Ok(SkillDocument { item, content })
}

#[tauri::command]
pub fn skill_sync_preview(target: String) -> Result<SkillSyncPreview, String> {
    build_preview(&target)
}

#[tauri::command]
pub fn skill_sync_apply(target: String, overwrite: bool) -> Result<SkillSyncPreview, String> {
    let preview = build_preview(&target)?;
    let destination_root = target_root(&target)?;
    for skill in &preview.create {
        let source = PathBuf::from(&skill.path);
        let dest = destination_root.join(&skill.name).join("SKILL.md");
        let content = std::fs::read(source).map_err(|e| e.to_string())?;
        write_safely(&dest, &content)?;
    }
    if overwrite {
        for skill in preview.update.iter().chain(preview.conflict.iter()) {
            let source = PathBuf::from(&skill.path);
            let dest = destination_root.join(&skill.name).join("SKILL.md");
            let content = std::fs::read(source).map_err(|e| e.to_string())?;
            write_safely(&dest, &content)?;
        }
    }
    Ok(preview)
}

fn managed_skill_path(source: &str, name: &str) -> Result<PathBuf, String> {
    let allowed_source = ["codex", "claude", "qoder", "workbuddy"].contains(&source);
    let safe = safe_name(name);
    if !allowed_source || safe != name || safe.is_empty() {
        return Err("invalid shared skill identifier".into());
    }
    let path = shared_root().join(source).join(&safe).join("SKILL.md");
    if !path.exists() {
        return Err("shared skill not found".into());
    }
    Ok(path)
}

fn managed_skill_item(source: &str, name: &str) -> Result<SkillItem, String> {
    let path = managed_skill_path(source, name)?;
    Ok(with_manifest(parse_skill(&path, source)?, &path))
}

#[tauri::command]
pub fn skill_set_status(source: String, name: String, status: String) -> Result<SkillItem, String> {
    if !matches!(status.as_str(), "draft" | "published") {
        return Err("skill status must be draft or published".into());
    }
    let path = managed_skill_path(&source, &name)?;
    let item = parse_skill(&path, &source)?;
    let mut manifest = read_manifest(&path, &item.hash);
    manifest.status = status;
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    write_manifest(&path, &manifest)?;
    let item = managed_skill_item(&source, &name)?;
    let _ = refresh_skill_cache()?;
    Ok(item)
}

#[tauri::command]
pub fn skill_set_assignment(
    source: String,
    name: String,
    target: String,
    equipped: bool,
) -> Result<SkillItem, String> {
    let _ = target_root(&target)?;
    let path = managed_skill_path(&source, &name)?;
    let item = parse_skill(&path, &source)?;
    let mut manifest = read_manifest(&path, &item.hash);
    if equipped {
        if !manifest
            .assigned_agents
            .iter()
            .any(|agent| agent == &target)
        {
            manifest.assigned_agents.push(target);
            manifest.assigned_agents.sort();
        }
    } else {
        manifest.assigned_agents.retain(|agent| agent != &target);
    }
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    write_manifest(&path, &manifest)?;
    let item = managed_skill_item(&source, &name)?;
    let _ = refresh_skill_cache()?;
    Ok(item)
}

#[tauri::command]
pub fn skill_rollback_latest(source: String, name: String) -> Result<SkillItem, String> {
    let path = managed_skill_path(&source, &name)?;
    let mut snapshots = std::fs::read_dir(version_root(&path))
        .map_err(|_| "该 Skill 尚无可回滚版本".to_string())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
        .collect::<Vec<_>>();
    snapshots.sort();
    let snapshot = snapshots.pop().ok_or("该 Skill 尚无可回滚版本")?;
    let previous = std::fs::read(&snapshot).map_err(|e| e.to_string())?;
    let current = std::fs::read(&path).map_err(|e| e.to_string())?;
    let current_hash = hash(&current);
    let restored_hash = hash(&previous);
    let mut manifest = read_manifest(&path, &current_hash);
    let backup =
        version_root(&path).join(format!("v{}-{}.md", manifest.version, &current_hash[..12]));
    std::fs::create_dir_all(version_root(&path)).map_err(|e| e.to_string())?;
    std::fs::write(backup, current).map_err(|e| e.to_string())?;
    write_safely(&path, &previous)?;
    manifest.version += 1;
    manifest.status = "draft".into();
    manifest.current_hash = restored_hash;
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    write_manifest(&path, &manifest)?;
    let item = managed_skill_item(&source, &name)?;
    let _ = refresh_skill_cache()?;
    Ok(item)
}

fn build_preview(target: &str) -> Result<SkillSyncPreview, String> {
    let destination_root = target_root(target)?;
    let mut create = Vec::new();
    let mut update = Vec::new();
    let mut unchanged = Vec::new();
    let mut conflict = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for skill in shared_skills()? {
        // A target's own skills are already present; the registry is for
        // sharing skills across harnesses, not copying a file onto itself.
        if skill.source == target
            || skill.status != "published"
            || !skill.assigned_agents.iter().any(|agent| agent == target)
        {
            continue;
        }
        if let Some(existing_source) = seen.insert(skill.name.clone(), skill.source.clone()) {
            if existing_source != skill.source {
                conflict.push(skill);
                continue;
            }
        }
        let dest = destination_root.join(&skill.name).join("SKILL.md");
        if !dest.exists() {
            create.push(skill);
        } else {
            let existing_hash = std::fs::read(&dest)
                .map(|bytes| hash(&bytes))
                .map_err(|e| e.to_string())?;
            if existing_hash == skill.hash {
                unchanged.push(skill);
            } else {
                update.push(skill);
            }
        }
    }
    Ok(SkillSyncPreview {
        target: target.to_string(),
        create,
        update,
        unchanged,
        conflict,
    })
}

fn write_safely(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("skill path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = path.with_extension("md.tmp");
    std::fs::write(&temp, content).map_err(|e| e.to_string())?;
    // Windows does not allow `rename` to replace an existing destination.
    // The temporary file is fully written before the requested replacement is
    // made, so a failed write never corrupts the prior skill.
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&temp, path).map_err(|e| e.to_string())
}
