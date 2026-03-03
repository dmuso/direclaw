use crate::local_llm::{LlamaCppRuntime, LocalLlmInferenceConfig};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulletinPreprocessOutput {
    pub rendered: String,
    pub citations: Vec<String>,
}

pub fn preprocess_memory_bulletin(
    runtime: &LlamaCppRuntime,
    bulletin: &str,
    citations: &[String],
    inference: &LocalLlmInferenceConfig,
) -> Result<BulletinPreprocessOutput, String> {
    let prompt = format!(
        "{}\n\nBulletin:\n{}",
        runtime.memory_bulletin_prompt_template(),
        bulletin,
    );
    let generated = runtime.generate(&prompt)?;
    let deduped = dedupe_lines(&generated, inference.bulletin_dedup_similarity_threshold);
    let rendered = cap_chars(&deduped.join("\n"), inference.max_output_chars);
    Ok(BulletinPreprocessOutput {
        rendered,
        citations: dedupe_citations(citations),
    })
}

pub fn preprocess_thread_context(
    runtime: &LlamaCppRuntime,
    thread_context: &str,
    inference: &LocalLlmInferenceConfig,
) -> Result<String, String> {
    let prompt = format!(
        "{}\n\nThread Context:\n{}",
        runtime.thread_context_prompt_template(),
        thread_context,
    );
    let generated = runtime.generate(&prompt)?;
    let mut lines = dedupe_lines(&generated, inference.context_dedup_similarity_threshold);
    preserve_identifiers(thread_context, &mut lines);
    Ok(cap_chars(&lines.join("\n"), inference.max_output_chars))
}

fn dedupe_citations(citations: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for citation in citations {
        let trimmed = citation.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            deduped.push(trimmed.to_string());
        }
    }
    deduped
}

fn dedupe_lines(input: &str, similarity_threshold: f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in input.lines() {
        let candidate = line.trim();
        if candidate.is_empty() {
            continue;
        }
        let mut duplicate = false;
        for existing in &out {
            if near_duplicate(existing, candidate, similarity_threshold) {
                duplicate = true;
                break;
            }
        }
        if !duplicate {
            out.push(candidate.to_string());
        }
    }
    out
}

fn preserve_identifiers(source: &str, lines: &mut Vec<String>) {
    let mut markers: Vec<String> = source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.contains("run_id=") || trimmed.contains("step_id=") {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect();
    markers.dedup();
    for marker in markers {
        if !lines.iter().any(|line| line.contains(&marker)) {
            lines.push(marker);
        }
    }
}

fn near_duplicate(left: &str, right: &str, threshold: f32) -> bool {
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    let left_norm = normalize_for_similarity(left);
    let right_norm = normalize_for_similarity(right);
    if left_norm.is_empty() || right_norm.is_empty() {
        return false;
    }
    let similarity = jaccard_similarity(&left_norm, &right_norm);
    similarity >= threshold
}

fn normalize_for_similarity(input: &str) -> HashSet<String> {
    input
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect()
}

fn jaccard_similarity(left: &HashSet<String>, right: &HashSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count() as f32;
    let union = left.union(right).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn cap_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::near_duplicate;

    #[test]
    fn near_duplicate_detects_same_text_with_minor_variation() {
        assert!(near_duplicate(
            "Deployment failed due to missing migration",
            "deployment failed due to missing migrations",
            0.5,
        ));
    }
}
