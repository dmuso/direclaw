use super::paths::MemoryPaths;
use super::retrieval::{
    all_required_bulletin_sections, append_memory_log, hybrid_recall, HybridRecallRequest,
    HybridRecallResult, MemoryCitation, MemoryRecallError, MemoryRecallOptions,
};
use super::MemoryRepository;
use crate::orchestration::workspace_access::WorkspaceAccessContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BulletinSectionName {
    KnowledgeSummary,
    ActiveGoals,
    OpenTodos,
    RecentDecisions,
    PreferenceProfile,
    ConflictsAndUncertainties,
}

impl BulletinSectionName {
    fn key(self) -> &'static str {
        match self {
            Self::KnowledgeSummary => "knowledge_summary",
            Self::ActiveGoals => "active_goals",
            Self::OpenTodos => "open_todos",
            Self::RecentDecisions => "recent_decisions",
            Self::PreferenceProfile => "preference_profile",
            Self::ConflictsAndUncertainties => "conflicts_and_uncertainties",
        }
    }

    fn all() -> [Self; 6] {
        [
            Self::KnowledgeSummary,
            Self::ActiveGoals,
            Self::OpenTodos,
            Self::RecentDecisions,
            Self::PreferenceProfile,
            Self::ConflictsAndUncertainties,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulletinSection {
    pub name: BulletinSectionName,
    #[serde(default)]
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBulletin {
    pub rendered: String,
    #[serde(default)]
    pub citations: Vec<MemoryCitation>,
    #[serde(default)]
    pub sections: Vec<BulletinSection>,
    pub generated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBulletinOptions {
    pub max_chars: usize,
    pub generated_at: i64,
}

impl Default for MemoryBulletinOptions {
    fn default() -> Self {
        Self {
            max_chars: 4_000,
            generated_at: 0,
        }
    }
}

pub fn build_memory_bulletin(
    recall: &HybridRecallResult,
    options: &MemoryBulletinOptions,
) -> MemoryBulletin {
    let mut sections = BulletinSectionName::all()
        .iter()
        .map(|name| BulletinSection {
            name: *name,
            lines: Vec::new(),
        })
        .collect::<Vec<_>>();

    for entry in &recall.memories {
        let summary = entry.snippet.as_deref().unwrap_or(&entry.memory.summary);
        let line = format!("- {} [{}]", summary, entry.memory.memory_id);
        match entry.memory.node_type {
            super::domain::MemoryNodeType::Goal => {
                find_section_mut(&mut sections, BulletinSectionName::ActiveGoals)
                    .lines
                    .push(line)
            }
            super::domain::MemoryNodeType::Todo => {
                find_section_mut(&mut sections, BulletinSectionName::OpenTodos)
                    .lines
                    .push(line)
            }
            super::domain::MemoryNodeType::Decision => {
                find_section_mut(&mut sections, BulletinSectionName::RecentDecisions)
                    .lines
                    .push(line)
            }
            super::domain::MemoryNodeType::Preference => {
                find_section_mut(&mut sections, BulletinSectionName::PreferenceProfile)
                    .lines
                    .push(line)
            }
            _ => find_section_mut(&mut sections, BulletinSectionName::KnowledgeSummary)
                .lines
                .push(line),
        }
        if entry.unresolved_contradiction || entry.memory.confidence < 0.5 {
            find_section_mut(
                &mut sections,
                BulletinSectionName::ConflictsAndUncertainties,
            )
            .lines
            .push(format!("- {} [{}]", summary, entry.memory.memory_id));
        }
    }

    truncate_sections(&mut sections, options.max_chars);

    let rendered = render_sections(&sections);
    let citations = citations_for_sections(&sections, &recall.citations());

    MemoryBulletin {
        rendered,
        citations,
        sections,
        generated_at: options.generated_at,
    }
}

pub fn generate_bulletin_for_message(
    repo: &MemoryRepository,
    paths: &MemoryPaths,
    message_id: &str,
    request: &HybridRecallRequest,
    recall_options: &MemoryRecallOptions,
    bulletin_options: &MemoryBulletinOptions,
    workspace_context: Option<&WorkspaceAccessContext>,
) -> Result<MemoryBulletin, MemoryRecallError> {
    let target = paths.bulletins.join(format!("{message_id}.json"));
    match hybrid_recall(
        repo,
        request,
        recall_options,
        workspace_context,
        &paths.log_file,
    ) {
        Ok(recall) => {
            let bulletin = build_memory_bulletin(&recall, bulletin_options);
            persist_bulletin(&target, &bulletin)?;
            Ok(bulletin)
        }
        Err(err) => {
            append_memory_log(
                &paths.log_file,
                "memory.bulletin.generation_failed",
                &[
                    ("message_id", Value::String(message_id.to_string())),
                    ("error", Value::String(err.to_string())),
                ],
            )?;

            if let Some(previous) = load_latest_bulletin(&paths.bulletins, Some(&target))? {
                append_memory_log(
                    &paths.log_file,
                    "memory.bulletin.fallback",
                    &[
                        ("message_id", Value::String(message_id.to_string())),
                        ("fallback", Value::String("previous_snapshot".to_string())),
                    ],
                )?;
                persist_bulletin(&target, &previous)?;
                return Ok(previous);
            }

            let empty = MemoryBulletin {
                rendered: String::new(),
                citations: Vec::new(),
                sections: BulletinSectionName::all()
                    .iter()
                    .map(|name| BulletinSection {
                        name: *name,
                        lines: Vec::new(),
                    })
                    .collect(),
                generated_at: bulletin_options.generated_at,
            };
            append_memory_log(
                &paths.log_file,
                "memory.bulletin.fallback",
                &[
                    ("message_id", Value::String(message_id.to_string())),
                    ("fallback", Value::String("empty_bulletin".to_string())),
                ],
            )?;
            persist_bulletin(&target, &empty)?;
            Ok(empty)
        }
    }
}

fn find_section_mut(
    sections: &mut [BulletinSection],
    name: BulletinSectionName,
) -> &mut BulletinSection {
    sections
        .iter_mut()
        .find(|section| section.name == name)
        .expect("section must exist")
}

fn truncate_sections(sections: &mut Vec<BulletinSection>, max_chars: usize) {
    if render_sections(sections).len() <= max_chars {
        return;
    }

    let trim_order = [
        BulletinSectionName::KnowledgeSummary,
        BulletinSectionName::PreferenceProfile,
        BulletinSectionName::ConflictsAndUncertainties,
        BulletinSectionName::RecentDecisions,
        BulletinSectionName::OpenTodos,
        BulletinSectionName::ActiveGoals,
    ];

    loop {
        if render_sections(sections).len() <= max_chars {
            return;
        }
        let mut removed = false;
        for section_name in trim_order {
            if let Some(section) = sections
                .iter_mut()
                .find(|section| section.name == section_name)
            {
                if !section.lines.is_empty() {
                    section.lines.pop();
                    removed = true;
                    break;
                }
            }
        }
        if !removed {
            break;
        }
    }

    if render_sections(sections).len() <= max_chars {
        return;
    }

    // Deterministic hard cap fallback for extremely small limits:
    // remove lowest-priority sections entirely, preserving Goal/Todo/Decision longest.
    let section_drop_order = [
        BulletinSectionName::ConflictsAndUncertainties,
        BulletinSectionName::PreferenceProfile,
        BulletinSectionName::KnowledgeSummary,
        BulletinSectionName::RecentDecisions,
        BulletinSectionName::OpenTodos,
        BulletinSectionName::ActiveGoals,
    ];
    for name in section_drop_order {
        if render_sections(sections).len() <= max_chars {
            return;
        }
        if let Some(idx) = sections.iter().position(|section| section.name == name) {
            sections.remove(idx);
        }
    }
}

fn citations_for_sections(
    sections: &[BulletinSection],
    citations: &[MemoryCitation],
) -> Vec<MemoryCitation> {
    let mut ids = BTreeSet::new();
    for section in sections {
        for line in &section.lines {
            if let Some(start) = line.rfind('[') {
                if let Some(end) = line.rfind(']') {
                    if end > start + 1 {
                        ids.insert(line[start + 1..end].to_string());
                    }
                }
            }
        }
    }

    citations
        .iter()
        .filter(|citation| ids.contains(&citation.memory_id))
        .cloned()
        .collect()
}

fn render_sections(sections: &[BulletinSection]) -> String {
    let mut out = String::new();
    for section in sections {
        out.push_str(section.name.key());
        out.push(':');
        out.push('\n');
        if section.lines.is_empty() {
            out.push_str("-\n");
        } else {
            for line in &section.lines {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

fn persist_bulletin(path: &Path, bulletin: &MemoryBulletin) -> Result<(), MemoryRecallError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| MemoryRecallError::LogWrite {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let body =
        serde_json::to_vec_pretty(bulletin).map_err(|source| MemoryRecallError::LogWrite {
            path: path.display().to_string(),
            source: std::io::Error::other(source.to_string()),
        })?;
    fs::write(path, body).map_err(|source| MemoryRecallError::LogWrite {
        path: path.display().to_string(),
        source,
    })
}

fn load_latest_bulletin(
    bulletins_dir: &Path,
    exclude: Option<&Path>,
) -> Result<Option<MemoryBulletin>, MemoryRecallError> {
    if !bulletins_dir.is_dir() {
        return Ok(None);
    }
    let mut candidates: Vec<PathBuf> = fs::read_dir(bulletins_dir)
        .map_err(|source| MemoryRecallError::LogWrite {
            path: bulletins_dir.display().to_string(),
            source,
        })?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    if let Some(path) = exclude {
        candidates.retain(|candidate| candidate != path);
    }

    let mut parsed = Vec::new();
    for path in candidates {
        let raw = fs::read_to_string(&path).map_err(|source| MemoryRecallError::LogWrite {
            path: path.display().to_string(),
            source,
        })?;
        if let Ok(value) = serde_json::from_str::<MemoryBulletin>(&raw) {
            let modified = fs::metadata(&path)
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0);
            parsed.push((value, modified));
        }
    }

    parsed.sort_by(|(a, a_modified), (b, b_modified)| {
        a.generated_at
            .cmp(&b.generated_at)
            .then_with(|| a_modified.cmp(b_modified))
    });

    if let Some((latest, _)) = parsed.pop() {
        return Ok(Some(latest));
    }

    Ok(None)
}

trait RecallResultExt {
    fn citations(&self) -> Vec<MemoryCitation>;
}

impl RecallResultExt for HybridRecallResult {
    fn citations(&self) -> Vec<MemoryCitation> {
        self.memories
            .iter()
            .map(|entry| entry.citation.clone())
            .collect()
    }
}

pub fn required_bulletin_section_names() -> Vec<&'static str> {
    all_required_bulletin_sections().into_keys().collect()
}

pub fn bulletin_to_section_map(bulletin: &MemoryBulletin) -> BTreeMap<String, Vec<String>> {
    bulletin
        .sections
        .iter()
        .map(|section| (section.name.key().to_string(), section.lines.clone()))
        .collect()
}
