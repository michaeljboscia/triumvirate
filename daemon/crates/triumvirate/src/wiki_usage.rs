//! Wiki-usage evidence for one peer call (docs/briefs/wiki-usage-measurement-brief.md, step two).
//!
//! Raw evidence only, never a verdict. The jury on the design (2026-09-19) was unanimous that
//! classification happens at REPORT time so a better rule can be re-run over old events, so
//! nothing here decides "used", "recitation" or "wiki subject". Ids and counts only: no page
//! text, no prompt text, no response text.
//!
//! The page list comes from the wiki's own `_menu.json`, the same file the Claude-side scanner
//! (`mneme-bosciamem/ops/usage_events.py:page_ids`) reads, and the detector is that scanner's
//! bare-id pattern `(?<![\w/-])(ids)(?![\w-])`. `scripts/fixtures/wiki-detector-cases.json` is
//! checked by both implementations, so they cannot silently disagree about what a page id is.

use std::path::{Path, PathBuf};

pub(crate) struct Wiki {
    pub dir: PathBuf,
    pub pages: Vec<String>,
    /// The date on the map's first line (`generated YYYY-MM-DD`), when it has one.
    pub generated: Option<String>,
    /// The map's size in bytes. With `generated`, enough to tell one map from the next.
    pub map_bytes: u64,
}

/// `TRIUMVIRATE_WIKI_DIR`, else `$HOME/projects/mneme-bosciamem`, where the map every peer
/// carries is compiled. Absolute or an error: with HOME cleared the fallback became the relative
/// `projects/mneme-bosciamem`, resolved against whatever the cwd happened to be.
pub(crate) fn wiki_dir() -> Result<PathBuf, String> {
    let dir = match std::env::var("TRIUMVIRATE_WIKI_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("projects")
            .join("mneme-bosciamem"),
    };
    if dir.is_absolute() {
        Ok(dir)
    } else {
        Err(format!(
            "wiki dir `{}` is not absolute (HOME unset?); set TRIUMVIRATE_WIKI_DIR",
            dir.display()
        ))
    }
}

/// Loads the page list and map identity. An error is returned, never an empty list: an empty
/// page list would score every call zero, which reads as "nobody used the wiki".
pub(crate) fn load_wiki(dir: &Path) -> Result<Wiki, String> {
    let menu_path = dir.join("_menu.json");
    let menu: serde_json::Value = std::fs::read_to_string(&menu_path)
        .map_err(|e| format!("{}: {e}", menu_path.display()))
        .and_then(|s| serde_json::from_str(&s).map_err(|e| format!("{}: {e}", menu_path.display())))?;
    let pages: Vec<String> = menu
        .as_object()
        .ok_or_else(|| format!("{} is not a JSON object", menu_path.display()))?
        .keys()
        .filter(|k| !k.starts_with('_'))
        .cloned()
        .collect();
    if pages.is_empty() {
        return Err(format!("{} lists no pages", menu_path.display()));
    }
    let index_path = dir.join("_index.md");
    let index = std::fs::read(&index_path).map_err(|e| format!("{}: {e}", index_path.display()))?;
    let first_line = String::from_utf8_lossy(&index).lines().next().unwrap_or("").to_string();
    let generated = first_line
        .split("generated ")
        .nth(1)
        .map(|rest| rest.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect::<String>())
        .filter(|d| !d.is_empty());
    Ok(Wiki {
        dir: dir.to_path_buf(),
        pages,
        generated,
        map_bytes: index.len() as u64,
    })
}

/// Python's `\w` for a `str` pattern: alphanumeric or underscore.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Every page id named in `text`, in order, repeats included. The same scan as Python's
/// `finditer` over the alternation: left to right, non-overlapping, first listed id that fits
/// wins at each position.
pub(crate) fn page_ids_in(text: &str, pages: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    let mut prev: Option<char> = None;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let boundary_before = !prev.is_some_and(|c| is_word(c) || c == '/' || c == '-');
        let hit = boundary_before
            .then(|| {
                pages.iter().find(|p| {
                    rest.starts_with(p.as_str())
                        && !rest[p.len()..].chars().next().is_some_and(|c| is_word(c) || c == '-')
                })
            })
            .flatten();
        if let Some(p) = hit {
            found.push(p.clone());
            prev = p.chars().last();
            i += p.len();
        } else {
            let c = rest.chars().next().expect("i is on a char boundary below len");
            prev = Some(c);
            i += c.len_utf8();
        }
    }
    found
}

/// Absolute paths named in a prompt. Paths, not prose: the report's wiki-as-subject rule reads
/// these instead of the prompt, which is never stored.
pub(crate) fn paths_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | '<' | '>' | ','))
        .map(|t| t.trim_end_matches(['.', ':', ';']))
        .filter(|t| (t.starts_with('/') || t.starts_with("~/")) && t[1..].contains('/'))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out.truncate(50);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../../scripts/fixtures/wiki-detector-cases.json");

    #[test]
    fn detector_agrees_with_the_shared_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert!(!cases.is_empty(), "an empty fixture proves nothing");
        // The fixture's own ids, which are real ids from _menu.json. Reading the live menu here
        // would make the test depend on this machine.
        let mut pages: Vec<String> = Vec::new();
        for c in cases {
            for id in c["want"].as_array().unwrap() {
                let id = id.as_str().unwrap().to_string();
                if !pages.contains(&id) {
                    pages.push(id);
                }
            }
        }
        let mut failures = Vec::new();
        for c in cases {
            let got = page_ids_in(c["text"].as_str().unwrap(), &pages);
            let want: Vec<String> =
                c["want"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
            if got != want {
                failures.push(format!("{}: got {got:?}, want {want:?}", c["label"]));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn load_wiki_refuses_an_empty_menu_rather_than_scoring_zero() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("_menu.json"), r#"{"_comment": "x"}"#).unwrap();
        std::fs::write(dir.path().join("_index.md"), "# map\n").unwrap();
        assert!(load_wiki(dir.path()).is_err());
        assert!(load_wiki(&dir.path().join("absent")).is_err());
    }

    #[test]
    fn load_wiki_reads_pages_and_the_map_date() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("_menu.json"), r#"{"_comment": "x", "a-page": {}, "b-page": {}}"#).unwrap();
        std::fs::write(
            dir.path().join("_index.md"),
            "# bosciamem wiki index (2 pages, generated 2026-09-19)\n",
        )
        .unwrap();
        let w = load_wiki(dir.path()).unwrap();
        assert_eq!(w.pages, vec!["a-page", "b-page"]);
        assert_eq!(w.generated.as_deref(), Some("2026-09-19"));
        assert_eq!(w.map_bytes, 55);
    }

    #[test]
    fn wiki_dir_refuses_a_relative_path() {
        let _guard = crate::tests::env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let saved = (std::env::var_os("TRIUMVIRATE_WIKI_DIR"), std::env::var_os("HOME"));
        // SAFETY: serialised by the binary-wide env lock and restored below.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_WIKI_DIR");
            std::env::set_var("HOME", "");
        }
        let got = wiki_dir();
        unsafe {
            match saved.0 { Some(v) => std::env::set_var("TRIUMVIRATE_WIKI_DIR", v), None => std::env::remove_var("TRIUMVIRATE_WIKI_DIR") }
            match saved.1 { Some(v) => std::env::set_var("HOME", v), None => std::env::remove_var("HOME") }
        }
        assert!(got.is_err(), "{got:?}");
    }

    #[test]
    fn paths_in_keeps_paths_and_drops_prose() {
        let got = paths_in("Review `/Users/x/projects/mneme-bosciamem/aws-operations.md`, and ~/a/b. Not a/b or /root.");
        assert_eq!(got, vec!["/Users/x/projects/mneme-bosciamem/aws-operations.md", "~/a/b"]);
    }
}
