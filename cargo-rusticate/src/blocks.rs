//! Marked blocks: lines that exist only in the template repository, or only
//! when a module is part of the project.
//!
//! A marker is a line of its own holding one comment:
//!
//! ```text
//! <!-- rusticate:template-only:start -->     Markdown, HTML
//! // rusticate:module(postgres):start        Rust
//! # rusticate:module(messaging):end          TOML, YAML, shell
//! -- rusticate:module(example):start         SQL
//! ```
//!
//! Blocks nest. A line is kept when every block around it is kept; marker
//! lines themselves are always removed, so generated projects carry none.

use std::borrow::Cow;

use crate::identity::Problem;

const PREFIX: &str = "rusticate:";
const TEMPLATE_ONLY: &str = "template-only";

/// Comment syntaxes a marker may be written in: (opening, closing).
const COMMENTS: [(&str, &str); 4] = [("<!--", "-->"), ("//", ""), ("#", ""), ("--", "")];

/// What a block is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Block {
    /// Only in the template repository: never generated.
    TemplateOnly,
    /// Only when the named module is part of the project.
    Module(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Start,
    End,
}

/// Keeps the lines whose blocks `keep` accepts. Unchanged text is borrowed.
pub(crate) fn strip<'a>(
    text: &'a str,
    keep: &dyn Fn(&Block) -> bool,
) -> Result<Cow<'a, str>, Problem> {
    if !text.contains(PREFIX) {
        return Ok(Cow::Borrowed(text));
    }
    let mut out = String::with_capacity(text.len());
    let mut open: Vec<(Block, usize, bool)> = Vec::new();
    let mut found_marker = false;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let number = index + 1;
        let problem = |message: String| Problem::new(number, message);
        match parse(line).map_err(problem)? {
            Some((block, Edge::Start)) => {
                found_marker = true;
                let kept = open.last().is_none_or(|(_, _, kept)| *kept) && keep(&block);
                open.push((block, number, kept));
            }
            Some((block, Edge::End)) => {
                found_marker = true;
                match open.pop() {
                    Some((started, _, _)) if started == block => {}
                    Some((started, line, _)) => {
                        return Err(problem(format!(
                            "{} ends here, but {} (line {line}) is still open",
                            describe(&block),
                            describe(&started)
                        )));
                    }
                    None => {
                        return Err(problem(format!("{} end without a start", describe(&block))));
                    }
                }
            }
            None if open.last().is_none_or(|(_, _, kept)| *kept) => out.push_str(line),
            None => {}
        }
    }
    if let Some((block, line, _)) = open.pop() {
        return Err(Problem::new(
            line,
            format!("{} is never closed", describe(&block)),
        ));
    }
    Ok(if found_marker {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    })
}

/// The marker on `line`, if it is one. A comment that starts with the marker
/// prefix but is not a whole, valid marker line is an error, so a misplaced
/// marker can never leak into a project.
fn parse(line: &str) -> Result<Option<(Block, Edge)>, String> {
    let trimmed = line.trim();
    for (open, close) in COMMENTS {
        let Some(rest) = trimmed.strip_prefix(open) else {
            continue;
        };
        let Some(body) = rest.strip_suffix(close).map(str::trim) else {
            continue;
        };
        let Some(marker) = body.strip_prefix(PREFIX) else {
            continue;
        };
        return parse_marker(marker)
            .map(Some)
            .ok_or_else(|| format!("`{body}` is not a valid marker"));
    }
    let introduces_marker = COMMENTS
        .iter()
        .any(|(open, _)| line.contains(&format!("{open} {PREFIX}")));
    if introduces_marker {
        return Err("a marker must be on its own line".to_owned());
    }
    Ok(None)
}

/// `template-only:start`, `module(postgres):end`, ...
fn parse_marker(marker: &str) -> Option<(Block, Edge)> {
    let (kind, edge) = marker.rsplit_once(':')?;
    let edge = match edge {
        "start" => Edge::Start,
        "end" => Edge::End,
        _ => return None,
    };
    if kind == TEMPLATE_ONLY {
        return Some((Block::TemplateOnly, edge));
    }
    let name = kind.strip_prefix("module(")?.strip_suffix(')')?;
    let valid = !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-');
    valid.then(|| (Block::Module(name.to_owned()), edge))
}

fn describe(block: &Block) -> String {
    match block {
        Block::TemplateOnly => "the template-only block".to_owned(),
        Block::Module(name) => format!("the `{name}` block"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keep_postgres(block: &Block) -> bool {
        *block == Block::Module("postgres".to_owned())
    }

    fn render(text: &str) -> String {
        strip(text, &keep_postgres).unwrap().into_owned()
    }

    #[test]
    fn keeps_selected_modules_and_drops_the_rest() {
        let text = "a\n// rusticate:module(postgres):start\nb\n// rusticate:module(postgres):end\n\
                    # rusticate:module(messaging):start\nc\n# rusticate:module(messaging):end\nd\n";
        assert_eq!(render(text), "a\nb\nd\n");
    }

    #[test]
    fn template_only_blocks_are_always_dropped() {
        let text = "intro\n<!-- rusticate:template-only:start -->\n## Install\n\
                    <!-- rusticate:template-only:end -->\n## Next\n";
        assert_eq!(render(text), "intro\n## Next\n");
    }

    #[test]
    fn nested_blocks_need_every_enclosing_block() {
        let text = "-- rusticate:module(messaging):start\n\
                    -- rusticate:module(postgres):start\nx\n-- rusticate:module(postgres):end\n\
                    -- rusticate:module(messaging):end\n\
                    -- rusticate:module(postgres):start\ny\n-- rusticate:module(postgres):end\n";
        assert_eq!(render(text), "y\n");
    }

    #[test]
    fn indented_markers_and_text_without_markers() {
        let text = "fn f() {\n    // rusticate:module(postgres):start\n    db();\n    \
                    // rusticate:module(postgres):end\n}\n";
        assert_eq!(render(text), "fn f() {\n    db();\n}\n");
        let plain = "no markers, but mentions rusticate:module(x):start in prose\n";
        assert!(matches!(
            strip(plain, &keep_postgres).unwrap(),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn malformed_markers_are_problems_with_their_line() {
        for (text, line, message) in [
            (
                "a\n// rusticate:module(postgres):start\n",
                2,
                "never closed",
            ),
            (
                "// rusticate:module(postgres):end\n",
                1,
                "end without a start",
            ),
            (
                "// rusticate:module(postgres):start\n// rusticate:module(messaging):end\n",
                2,
                "still open",
            ),
            ("x(); // rusticate:module(postgres):start\n", 1, "own line"),
            (
                "// rusticate:module(Postgres):start\n",
                1,
                "not a valid marker",
            ),
            (
                "<!-- rusticate:template-only:begin -->\n",
                1,
                "not a valid marker",
            ),
        ] {
            let err = strip(text, &keep_postgres).unwrap_err();
            assert_eq!(err.line(), line, "{text}");
            assert!(err.to_string().contains(message), "{text}: {err}");
        }
    }
}
