use super::{
    IncomingMessage, OutboundContent, OutgoingMessage, OUTBOUND_MAX_CHARS,
    OUTBOUND_TRUNCATE_KEEP_CHARS, OUTBOUND_TRUNCATION_SUFFIX,
};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub fn extract_inbound_file_tags(message: &str) -> Vec<String> {
    extract_absolute_tags(message, "[file:")
}

pub fn append_inbound_file_tags(message: &str, files: &[String]) -> String {
    let tags: Vec<String> = files
        .iter()
        .map(String::as_str)
        .filter(|path| is_absolute_path(path))
        .map(|path| format!("[file: {path}]"))
        .collect();
    if tags.is_empty() {
        return message.to_string();
    }
    if message.is_empty() {
        return tags.join("\n");
    }
    format!("{message}\n{}", tags.join("\n"))
}

pub fn prepare_outbound_content(raw_message: &str) -> OutboundContent {
    let (stripped, referenced_files, omitted_files) = strip_send_file_tags(raw_message);
    OutboundContent {
        message: truncate_outbound_text(&stripped),
        files: referenced_files,
        omitted_files,
    }
}

pub(crate) fn normalize_outgoing_message(
    outgoing: &OutgoingMessage,
) -> (OutgoingMessage, Vec<String>) {
    let prepared = prepare_outbound_content(&outgoing.message);
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut omitted = prepared.omitted_files;

    for path in &outgoing.files {
        if is_absolute_readable_file(path) {
            if seen.insert(path.clone()) {
                files.push(path.clone());
            }
        } else {
            omitted.push(path.clone());
        }
    }
    for path in prepared.files {
        if seen.insert(path.clone()) {
            files.push(path);
        }
    }

    let mut normalized = outgoing.clone();
    normalized.message = prepared.message;
    normalized.files = files;
    (normalized, omitted)
}

pub(crate) fn normalize_inbound_payload(payload: &mut IncomingMessage) {
    let message_tags = extract_inbound_file_tags(&payload.message);
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for path in &payload.files {
        if is_absolute_path(path) && seen.insert(path.clone()) {
            merged.push(path.clone());
        }
    }
    for path in &message_tags {
        if seen.insert(path.clone()) {
            merged.push(path.clone());
        }
    }

    let existing: HashSet<String> = message_tags.into_iter().collect();
    let missing: Vec<String> = merged
        .iter()
        .filter(|path| !existing.contains(path.as_str()))
        .cloned()
        .collect();

    payload.message = append_inbound_file_tags(&payload.message, &missing);
    payload.files = merged;
}

fn strip_send_file_tags(message: &str) -> (String, Vec<String>, Vec<String>) {
    let mut output = String::with_capacity(message.len());
    let mut files = Vec::new();
    let mut omitted_files = Vec::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = message[cursor..].find("[send_file:") {
        let tag_start = cursor + rel_start;
        output.push_str(&message[cursor..tag_start]);

        let content_start = tag_start + "[send_file:".len();
        if let Some(rel_end) = message[content_start..].find(']') {
            let tag_end = content_start + rel_end;
            let candidate = message[content_start..tag_end].trim();
            if !candidate.is_empty() {
                if is_absolute_readable_file(candidate) {
                    files.push(candidate.to_string());
                } else {
                    omitted_files.push(candidate.to_string());
                }
            }
            cursor = tag_end + 1;
            continue;
        }

        output.push_str(&message[tag_start..]);
        cursor = message.len();
        break;
    }

    if cursor < message.len() {
        output.push_str(&message[cursor..]);
    }

    (output, files, omitted_files)
}

fn truncate_outbound_text(message: &str) -> String {
    if message.chars().count() <= OUTBOUND_MAX_CHARS {
        return message.to_string();
    }

    let mut truncated = String::new();
    truncated.extend(message.chars().take(OUTBOUND_TRUNCATE_KEEP_CHARS));
    truncated.push_str(OUTBOUND_TRUNCATION_SUFFIX);
    truncated
}

fn extract_absolute_tags(message: &str, prefix: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_start) = message[cursor..].find(prefix) {
        let start = cursor + rel_start + prefix.len();
        let Some(rel_end) = message[start..].find(']') else {
            break;
        };
        let end = start + rel_end;
        let candidate = message[start..end].trim();
        if is_absolute_path(candidate) {
            tags.push(candidate.to_string());
        }
        cursor = end + 1;
    }
    tags
}

fn is_absolute_path(path: &str) -> bool {
    Path::new(path).is_absolute()
}

fn is_absolute_readable_file(path: &str) -> bool {
    if !is_absolute_path(path) {
        return false;
    }
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}
