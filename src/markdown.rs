//! Just enough Markdown structure for the index: YAML frontmatter, headings (ATX `#`
//! and setext underlines, ignoring anything inside fenced code), sections, and fenced
//! code blocks.

/// One heading and everything under it up to the next heading of the same or a higher
/// level (so it includes its subsections).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The heading in lower kebab-case, made unique with `-2`, `-3`, ... in document
    /// order (see `documentation/1.0`'s Design By Contract).
    pub id: String,
    pub title: String,
    pub level: u8,
    /// The section's content without its own heading line.
    pub markdown: String,
}

/// Split `---`-delimited YAML frontmatter off the start of `text`.
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        return (None, text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return (Some(&rest[..offset]), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, text)
}

struct Heading {
    title: String,
    level: u8,
    /// Line index of the heading's first line, and of the first line after it.
    start: usize,
    body_start: usize,
}

fn fence_marker(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    ["```", "~~~"].into_iter().find(|m| trimmed.starts_with(m))
}

fn headings(lines: &[&str]) -> Vec<Heading> {
    let mut found = Vec::new();
    let mut fence: Option<&str> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(marker) = fence_marker(line) {
            fence = match fence {
                Some(open) if open == marker => None,
                Some(open) => Some(open),
                None => Some(marker),
            };
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if let Some((level, title)) = atx_heading(line) {
            found.push(Heading { title, level, start: i, body_start: i + 1 });
        } else if i > 0 && !lines[i - 1].trim().is_empty() && fence_marker(lines[i - 1]).is_none() {
            // Setext: a paragraph line underlined with === (h1) or --- (h2).
            let underline = line.trim();
            let level = if !underline.is_empty() && underline.chars().all(|c| c == '=') {
                Some(1)
            } else if underline.len() >= 3 && underline.chars().all(|c| c == '-') {
                Some(2)
            } else {
                None
            };
            let previous_is_heading = found.last().is_some_and(|h: &Heading| h.start == i - 1);
            if let (Some(level), false) = (level, previous_is_heading) {
                found.push(Heading {
                    title: lines[i - 1].trim().to_string(),
                    level,
                    start: i - 1,
                    body_start: i + 1,
                });
            }
        }
    }
    found
}

fn atx_heading(line: &str) -> Option<(u8, String)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim();
    (!title.is_empty()).then(|| (hashes as u8, title.to_string()))
}

/// All sections of `body`, in document order.
pub fn sections(body: &str) -> Vec<Section> {
    let lines: Vec<&str> = body.lines().collect();
    let found = headings(&lines);
    let mut seen = std::collections::HashMap::<String, usize>::new();
    found
        .iter()
        .enumerate()
        .map(|(n, heading)| {
            let end = found[n + 1..]
                .iter()
                .find(|next| next.level <= heading.level)
                .map_or(lines.len(), |next| next.start);
            let base = slugify(&heading.title);
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            let id = if *count == 1 { base } else { format!("{base}-{count}") };
            Section {
                id,
                title: heading.title.clone(),
                level: heading.level,
                markdown: lines[heading.body_start.min(end)..end].join("\n").trim().to_string(),
            }
        })
        .collect()
}

/// A heading's section id: lower kebab-case of its text, keeping only ASCII letters and
/// digits (`` `query` Message Type `` → `query-message-type`).
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if (c.is_whitespace() || c == '-' || c == '_') && !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    }
}

/// The contents of every fenced code block in `body`.
pub fn code_blocks(body: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut open: Option<(&str, Vec<&str>)> = None;
    for line in body.lines() {
        match (&mut open, fence_marker(line)) {
            (None, Some(marker)) => open = Some((marker, Vec::new())),
            (Some((marker, content)), Some(closing)) if *marker == closing && line.trim() == closing => {
                blocks.push(content.join("\n"));
                open = None;
            }
            (Some((_, content)), _) => content.push(line),
            (None, None) => {}
        }
    }
    blocks
}

/// Role names in a "Roles" section: backticked lower-kebab-case words, in order of
/// first appearance (didcomm.org's convention is to introduce each role as `` `name` ``).
pub fn backticked_names(markdown: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (i, part) in markdown.split('`').enumerate() {
        let is_name = i % 2 == 1
            && part.starts_with(|c: char| c.is_ascii_lowercase())
            && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if is_name && !names.iter().any(|n| n == part) {
            names.push(part.to_string());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_is_split_off() {
        let (yaml, body) = split_frontmatter("---\ntitle: X\n---\n\n## Roles\n");
        assert_eq!(yaml, Some("title: X\n"));
        assert_eq!(body, "\n## Roles\n");
        assert_eq!(split_frontmatter("## No frontmatter"), (None, "## No frontmatter"));
    }

    #[test]
    fn sections_nest_and_ignore_code() {
        let body = "intro\n## Roles\nr\n## Message Reference\n### `ping`\n```json\n# not a heading\n```\n### ping\nx\n## Roles\nagain";
        let sections = sections(body);
        let ids: Vec<_> = sections.iter().map(|s| (s.id.as_str(), s.level)).collect();
        assert_eq!(ids, [("roles", 2), ("message-reference", 2), ("ping", 3), ("ping-2", 3), ("roles-2", 2)]);
        // A section includes its subsections, and stops at the next same-level heading.
        assert!(sections[1].markdown.contains("# not a heading"));
        assert!(sections[1].markdown.ends_with('x'));
        assert_eq!(sections[0].markdown, "r");
    }

    #[test]
    fn setext_headings_count() {
        let sections = sections("DIDComm Messaging v2.1\n==================\n\ntext\n\nSub\n---\nmore");
        assert_eq!(sections[0].title, "DIDComm Messaging v2.1");
        assert_eq!(sections[0].level, 1);
        assert_eq!(sections[1].id, "sub");
        assert_eq!(sections[1].markdown, "more");
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Basic Walkthrough"), "basic-walkthrough");
        assert_eq!(slugify("`query` Message Type"), "query-message-type");
        assert_eq!(slugify("Problem Reports & ACKs"), "problem-reports-acks");
        assert_eq!(slugify("1 Out of Scope"), "1-out-of-scope");
    }

    #[test]
    fn code_blocks_and_roles() {
        let body = "a\n```json\n{\"type\": \"t\"}\n```\n~~~\nx\n~~~\n";
        assert_eq!(code_blocks(body), ["{\"type\": \"t\"}", "x"]);
        assert_eq!(
            backticked_names("- `mediator`: the agent. - `recipient`: the `formal name` of `mediator`"),
            ["mediator", "recipient"]
        );
    }
}
