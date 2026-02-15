use std::fs;
use std::path::Path;

pub const OUTBOUND_MAX_CHARS: usize = 4000;
pub const OUTBOUND_TRUNCATE_KEEP_CHARS: usize = 3900;
pub const OUTBOUND_TRUNCATION_SUFFIX: &str = "\n\n[Response truncated...]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundContent {
    pub message: String,
    pub files: Vec<String>,
    pub omitted_files: Vec<String>,
}

pub fn prepare_outbound_content(raw_message: &str) -> OutboundContent {
    let (stripped, referenced_files, omitted_files) = strip_send_file_tags(raw_message);
    OutboundContent {
        message: truncate_outbound_text(&stripped),
        files: referenced_files,
        omitted_files,
    }
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

fn is_absolute_readable_file(path: &str) -> bool {
    if !is_absolute_path(path) {
        return false;
    }
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn is_absolute_path(path: &str) -> bool {
    Path::new(path).is_absolute()
}
