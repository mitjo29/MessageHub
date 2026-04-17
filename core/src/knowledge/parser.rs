use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Result of parsing a markdown file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFile {
    /// YAML frontmatter as a generic JSON value (None if file has no frontmatter).
    pub frontmatter: Option<serde_yaml::Value>,
    /// The body split into sections at `#`/`##` headings.
    pub sections: Vec<Section>,
    /// Blake3 hash of the full file content (for incremental-update detection).
    pub content_hash: String,
    /// Approximate total token count (body only — used for budget logging).
    pub total_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    /// The heading that introduced this section (None for content before the first heading).
    pub heading: Option<String>,
    /// The section body text (heading is not included in content).
    pub content: String,
    /// Heading level (1-6). 0 if there's no heading.
    pub level: u8,
    /// Approximate token count for this section.
    pub tokens: usize,
}

/// Parse a markdown file into frontmatter + sections.
///
/// Frontmatter is YAML between `---` delimiters at the top of the file.
/// Sections are split at `#` and `##` heading boundaries. Content before
/// the first heading (e.g., a preamble paragraph after the frontmatter)
/// becomes a section with `heading = None` and `level = 0`.
pub fn parse_markdown_file(content: &str) -> Result<ParsedFile> {
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();

    let (frontmatter, body) = split_frontmatter(content)?;
    let sections = split_sections(body);
    let total_tokens = sections.iter().map(|s| s.tokens).sum();

    Ok(ParsedFile {
        frontmatter,
        sections,
        content_hash,
        total_tokens,
    })
}

/// Split a markdown file into (frontmatter, body).
///
/// Returns `(None, full_content)` if the file doesn't start with `---`.
fn split_frontmatter(content: &str) -> Result<(Option<serde_yaml::Value>, &str)> {
    let trimmed = content.trim_start_matches('\u{feff}'); // Strip BOM if present
    if !trimmed.starts_with("---") {
        return Ok((None, trimmed));
    }

    // Find the closing `---` on its own line.
    // The opening `---` is at position 0.
    let after_opening = &trimmed[3..];
    let after_opening = after_opening.strip_prefix('\n').unwrap_or(after_opening);

    let close_pos = find_frontmatter_close(after_opening);
    match close_pos {
        Some(pos) => {
            let yaml_str = &after_opening[..pos];
            let body_start = pos + after_opening[pos..].find('\n').unwrap_or(pos) + 1;
            let body = after_opening.get(body_start.min(after_opening.len())..).unwrap_or("");

            let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str)
                .map_err(|e| CoreError::VaultParse(format!("invalid YAML frontmatter: {}", e)))?;

            Ok((Some(fm), body))
        }
        None => {
            // No closing `---` found; treat whole file as body (unusual but possible).
            Ok((None, trimmed))
        }
    }
}

/// Find the byte offset of a line that is exactly `---` in `s`.
fn find_frontmatter_close(s: &str) -> Option<usize> {
    let mut pos = 0;
    for line in s.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(|c: char| c == '\n' || c == '\r');
        if trimmed == "---" {
            return Some(pos);
        }
        pos += line.len();
    }
    None
}

/// Split a markdown body into sections at heading boundaries.
///
/// Headings of level 1 or 2 start new sections. Deeper headings (###+) stay
/// inside the current section — section boundaries should be coarse enough
/// that each chunk is substantial but small enough for embedding context.
fn split_sections(body: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_level: u8 = 0;
    let mut current_content = String::new();

    for line in body.lines() {
        let (heading_level, heading_text) = parse_heading(line);

        if let (Some(level), Some(text)) = (heading_level, heading_text) {
            if level <= 2 {
                // Flush the current section before starting a new one.
                if !current_content.trim().is_empty() || current_heading.is_some() {
                    sections.push(make_section(
                        current_heading.take(),
                        current_level,
                        std::mem::take(&mut current_content),
                    ));
                }
                current_heading = Some(text.to_string());
                current_level = level;
                continue;
            }
        }

        current_content.push_str(line);
        current_content.push('\n');
    }

    if !current_content.trim().is_empty() || current_heading.is_some() {
        sections.push(make_section(current_heading, current_level, current_content));
    }

    sections
}

/// Parse a line and return `(level, text)` if it's an ATX heading, else `(None, None)`.
fn parse_heading(line: &str) -> (Option<u8>, Option<&str>) {
    let trimmed = line.trim_start();
    let mut level = 0u8;
    let mut chars = trimmed.chars();
    while chars.next() == Some('#') {
        level += 1;
    }
    if level == 0 || level > 6 {
        return (None, None);
    }
    // Require a space after the #s (ATX heading rule).
    let after_hashes = &trimmed[(level as usize)..];
    if !after_hashes.starts_with(' ') {
        return (None, None);
    }
    let text = after_hashes.trim();
    if text.is_empty() {
        return (None, None);
    }
    (Some(level), Some(text))
}

fn make_section(heading: Option<String>, level: u8, content: String) -> Section {
    let content = content.trim_end().to_string();
    let tokens = approx_token_count(&content);
    Section {
        heading,
        content,
        level,
        tokens,
    }
}

/// Rough token count heuristic: ~4 characters per token for English/French/German prose.
/// This is not a real tokenizer — it's just for budget planning.
pub fn approx_token_count(s: &str) -> usize {
    (s.chars().count() + 3) / 4
}
