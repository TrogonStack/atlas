use std::path::PathBuf;

use anyhow::{Context as _, Result};

use crate::{export, manifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Stdout,
    Write,
    Check,
}

pub struct FileOutcome {
    pub path: PathBuf,
    pub original: String,
    pub canonical: String,
}

impl FileOutcome {
    pub fn changed(&self) -> bool {
        self.original != self.canonical
    }
}

pub fn canonicalize(text: &str, source: &str) -> Result<String> {
    let manifests = manifest::parse_str(text, source)?;
    let types = trogon_atlas_core::transcode::TranscodePool::builtin()?;
    let mut out = String::new();
    for (i, loaded) in manifests.iter().enumerate() {
        if i > 0 {
            out.push_str("---\n");
        }
        let (_, _, doc) = export::entity_to_manifest(&loaded.entity, &types)?;
        out.push_str(&serde_yaml::to_string(&doc).context("rendering canonical YAML")?);
    }
    Ok(out)
}

pub fn format_paths(paths: &[PathBuf], force: bool) -> Result<Vec<FileOutcome>> {
    let files = manifest::collect_yaml_files(paths)?;
    let mut outcomes = Vec::with_capacity(files.len());
    for path in files {
        let original = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if !force {
            if let Some(line) = find_comment(&original) {
                crate::invalid_input!(
                    "{}:{line}: contains a YAML comment; canonical formatting has no slot to \
                     keep it, so it would be silently dropped. Rerun with --force to format \
                     anyway, or keep the file hand-formatted.",
                    path.display()
                );
            }
        }
        let canonical = canonicalize(&original, &path.display().to_string())?;
        outcomes.push(FileOutcome {
            path,
            original,
            canonical,
        });
    }
    Ok(outcomes)
}

#[derive(PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

/// Over-eager by design: block-scalar bodies are not tracked, so a literal
/// `#` there is a false refusal (fixed with `--force`), never a dropped comment.
fn find_comment(text: &str) -> Option<usize> {
    let mut quote = Quote::None;
    let mut line = 1usize;
    let mut at_boundary = true;
    let mut scalar_start = true;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Quote::Single => {
                if c == '\'' {
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                    } else {
                        quote = Quote::None;
                    }
                } else if c == '\n' {
                    line += 1;
                }
                at_boundary = false;
                scalar_start = false;
            }
            Quote::Double => {
                if c == '\\' {
                    if chars.next() == Some('\n') {
                        line += 1;
                    }
                } else if c == '"' {
                    quote = Quote::None;
                } else if c == '\n' {
                    line += 1;
                }
                at_boundary = false;
                scalar_start = false;
            }
            Quote::None => match c {
                '\'' if scalar_start => quote = Quote::Single,
                '"' if scalar_start => quote = Quote::Double,
                '#' if at_boundary => return Some(line),
                '\n' => {
                    line += 1;
                    at_boundary = true;
                    scalar_start = true;
                }
                ' ' | '\t' => at_boundary = true,
                ':' | '-' | ',' | '[' | '{' | '?' => {
                    at_boundary = false;
                    scalar_start = true;
                }
                _ => {
                    at_boundary = false;
                    scalar_start = false;
                }
            },
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn finds_leading_comment() {
        assert_eq!(find_comment("# hello\nkey: value\n"), Some(1));
    }

    #[test]
    fn finds_trailing_comment_after_whitespace() {
        assert_eq!(find_comment("key: value  # trailing\n"), Some(1));
    }

    #[test]
    fn ignores_hash_inside_double_quotes() {
        assert_eq!(find_comment("key: \"a # b\"\n"), None);
    }

    #[test]
    fn ignores_hash_inside_single_quotes() {
        assert_eq!(find_comment("key: 'a # b'\n"), None);
    }

    #[test]
    fn ignores_hash_not_preceded_by_whitespace() {
        assert_eq!(find_comment("key: http://example.com#fragment\n"), None);
    }

    #[test]
    fn handles_escaped_quote_inside_double_quoted_string() {
        assert_eq!(find_comment("key: \"a \\\" # b\"\n"), None);
    }

    #[test]
    fn handles_doubled_single_quote_inside_single_quoted_string() {
        assert_eq!(find_comment("key: 'it''s # fine'\n"), None);
    }

    #[test]
    fn finds_comment_after_apostrophe_in_plain_scalar() {
        assert_eq!(find_comment("title: Fan's account  # note\n"), Some(1));
    }

    #[test]
    fn ignores_hash_inside_quoted_list_item() {
        assert_eq!(find_comment("- 'a # b'\n- [\"c # d\"]\n"), None);
    }

    #[test]
    fn reports_line_number_of_comment() {
        assert_eq!(find_comment("key: value\nother: value\n# here\n"), Some(3));
    }

    #[test]
    fn canonicalize_rejects_unknown_kind_like_parse_does() {
        let err = canonicalize(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Widget\nmetadata: {namespace: a, name: b}\n",
            "t",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("unknown kind"));
    }

    #[test]
    fn canonicalize_is_idempotent_and_sorts_keys() {
        let text = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { name: order.placed, namespace: shop }
spec: { title: Order placed }
";
        let once = canonicalize(text, "t").expect("canonicalize");
        let twice = canonicalize(&once, "t").expect("canonicalize again");
        assert_eq!(once, twice);
        assert!(once.find("apiVersion").unwrap() < once.find("kind").unwrap());
    }

    #[test]
    fn canonicalize_keeps_multi_document_order() {
        let text = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: shop, name: a }
spec: { title: A }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: { namespace: shop, name: b }
spec: { title: B }
";
        let out = canonicalize(text, "t").expect("canonicalize");
        assert_eq!(out.matches("---").count(), 1);
        assert!(out.find("kind: Event").unwrap() < out.find("kind: Command").unwrap());
    }
}
