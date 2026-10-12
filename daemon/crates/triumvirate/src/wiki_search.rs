//! Search over the bosciamem wiki, behind the `wiki_search` MCP tool.
//!
//! Why this exists: the ceiling probes on 2026-09-19 showed the peers do not read wiki pages,
//! they `grep` the wiki directory and answer from the output. Grok answered six of six hinted
//! probes that way with zero pages opened. They are reaching for a search interface by hand, so
//! this gives them one, and the bridge sees the call instead of inferring it from shell
//! arguments.
//!
//! Deliberately not a vector index. Term matching over 20 markdown files is milliseconds, has no
//! model, no embedding service and no cache to go stale, and the wiki's own claims are verbatim
//! quotes, so the words in a question tend to be the words on the page.

use std::path::Path;

/// One page's matching lines.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PageHit {
    pub page: String,
    /// A Mneme reading page (`reading/<id>.md`, a saved bookmark), not a rule page.
    pub reading: bool,
    pub score: usize,
    /// (1-based line number, line text), best first.
    pub lines: Vec<(usize, String)>,
}

const MAX_TERMS: usize = 12;
const MAX_LINE_CHARS: usize = 400;

/// Query terms: lowercase, 3 characters or more, de-duplicated, capped.
///
/// Short tokens are dropped because "is", "on" and "we" match every page and would flatten the
/// ranking. A query of only short words yields no terms, and the caller says so rather than
/// returning the whole wiki.
pub(crate) fn terms(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in query.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
        let t = raw.trim_matches(|c: char| c == '-' || c == '_').to_lowercase();
        if t.chars().count() >= 3 && !out.contains(&t) {
            out.push(t);
        }
        if out.len() == MAX_TERMS {
            break;
        }
    }
    out
}

/// Does `hay` contain `term` as a WHOLE word?
///
/// A plain substring match ranked "supports" as a hit for `port`, and the first live call put a
/// 1050-line page of the word "supports" above the page that held the answer. Same boundary rule
/// as the page-id detector in `wiki_usage`.
fn contains_word(hay: &str, term: &str) -> bool {
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(rel) = hay[from..].find(term) {
        let start = from + rel;
        let end = start + term.len();
        let before_ok = start == 0 || !(bytes[start - 1] as char).is_alphanumeric();
        let after_ok = end == hay.len() || !(bytes[end] as char).is_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// Mneme's reading pages: saved bookmarks, explicitly NOT rules, listed in
/// `reading/_pages.json` as `[{"id": ...}, ...]`. They are kept out of `_menu.json` on purpose
/// (the Mneme judge files notes into menu pages), so they get their own list.
///
/// `reading/` is git-ignored and exists only on the Mac, so a missing list is the normal case
/// elsewhere and returns nothing, silently. A list that exists and does not parse is logged and
/// skipped: it must not take the rule pages' search down with it.
pub(crate) fn reading_pages(dir: &Path) -> Vec<String> {
    let path = dir.join("reading").join("_pages.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let ids: Option<Vec<String>> = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| {
        v.as_array()?
            .iter()
            .map(|p| p.get("id")?.as_str().map(str::to_string))
            .collect()
    });
    ids.unwrap_or_else(|| {
        tracing::warn!(list = %path.display(), "reading page list does not parse as [{{\"id\": ...}}]; reading pages not searched");
        Vec::new()
    })
}

/// Rank pages by how many of the query's terms they cover, then by the best line.
///
/// Coverage first, volume second, and deliberately so: the first live call ranked a 1050-line
/// page that matched ONE term hundreds of times above the short page that matched all three.
/// A page holding every word of the question is the one being looked for. The page id counts
/// as text, which is what makes "aws" find `aws-operations` when the body never says it.
///
/// `reading` pages (from `reading_pages`) are scored the same way and come back marked. On an
/// equal score a rule page ranks first: a standing rule outranks a bookmark about the subject.
pub(crate) fn search(
    dir: &Path,
    pages: &[String],
    reading: &[String],
    query: &str,
    max_pages: usize,
    max_lines: usize,
) -> Vec<PageHit> {
    let terms = terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let rule_pages = pages.iter().map(|p| (p, false, dir.join(format!("{p}.md"))));
    let reading_dir = dir.join("reading");
    let reading_pages = reading.iter().map(|p| (p, true, reading_dir.join(format!("{p}.md"))));
    let mut hits: Vec<PageHit> = Vec::new();
    for (page, is_reading, path) in rule_pages.chain(reading_pages) {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let id_lower = page.to_lowercase();
        let mut scored: Vec<(usize, usize, String)> = Vec::new(); // (hits, line no, text)
        for (i, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            let n = terms.iter().filter(|t| contains_word(&lower, t)).count();
            if n > 0 {
                let mut text = line.trim().to_string();
                if text.chars().count() > MAX_LINE_CHARS {
                    text = text.chars().take(MAX_LINE_CHARS).collect::<String>() + "...";
                }
                scored.push((n, i + 1, text));
            }
        }
        let id_hits = terms.iter().filter(|t| contains_word(&id_lower, t)).count();
        if scored.is_empty() && id_hits == 0 {
            continue;
        }
        // Coverage: how many DISTINCT query terms this page holds anywhere, id included.
        let covered = terms
            .iter()
            .filter(|t| contains_word(&id_lower, t) || text.to_lowercase().split('\n').any(|l| contains_word(l, t)))
            .count();
        let best_line = scored.iter().map(|(n, _, _)| *n).max().unwrap_or(0);
        let total: usize = scored.iter().map(|(n, _, _)| n).sum();
        // Volume cannot outrank coverage: a page matching every term wins whatever its length.
        let score = covered * 1_000 + best_line * 100 + total.min(99) + id_hits;
        // Best lines first, and ties keep file order so a page reads in its own order.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.truncate(max_lines);
        scored.sort_by_key(|(_, i, _)| *i);
        hits.push(PageHit {
            page: page.clone(),
            reading: is_reading,
            score,
            lines: scored.into_iter().map(|(_, i, t)| (i, t)).collect(),
        });
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.reading.cmp(&b.reading)).then(a.page.cmp(&b.page)));
    hits.truncate(max_pages);
    hits
}

/// What the peer sees. Page ids are named so a follow-up read is one step, and the file path is
/// spelled out because a peer that wants the whole page should open it, not search again.
pub(crate) fn render(dir: &Path, hits: &[PageHit], terms_used: &[String]) -> String {
    if hits.is_empty() {
        return format!(
            "No wiki page matched [{}]. The wiki is at {}; every page is `<page-id>.md`. Try \
             different words, or say you do not know rather than guessing.",
            terms_used.join(", "),
            dir.display()
        );
    }
    let mut out = format!(
        "{} wiki page(s) matched [{}]. Each claim on a page carries a verbatim quote and its \
         source note id. Each heading names the page's file in {}: open it for the whole page.\n",
        hits.len(),
        terms_used.join(", "),
        dir.display()
    );
    if hits.iter().any(|h| h.reading) {
        out.push_str(
            "Pages marked READING are saved bookmarks Mike has not adopted: cite them as \
             reading, never as a rule or a decision.\n",
        );
    }
    for hit in hits {
        if hit.reading {
            out.push_str(&format!(
                "\n## {} (reading/{}.md) READING: a saved bookmark, not a rule\n",
                hit.page, hit.page
            ));
        } else {
            out.push_str(&format!("\n## {} ({}.md)\n", hit.page, hit.page));
        }
        for (line_no, text) in &hit.lines {
            out.push_str(&format!("  L{line_no}: {text}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wiki() -> (tempfile::TempDir, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("home-infra.md"),
            "---\nid: home-infra\n---\nLangfuse runs on the homebox at port 3300 behind a tunnel.\nThe homebox is 192.168.2.110.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("aws-operations.md"),
            "---\ntitle: Cloud\n---\nSQS queues should use long polling.\nCloudFront needs OAC.\n",
        )
        .unwrap();
        (dir, vec!["home-infra".to_string(), "aws-operations".to_string()])
    }

    #[test]
    fn finds_the_page_that_holds_the_answer() {
        let (dir, pages) = wiki();
        let hits = search(dir.path(), &pages, &[], "What port does Langfuse run on?", 3, 5);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].page, "home-infra");
        assert!(hits[0].lines.iter().any(|(_, t)| t.contains("3300")), "{hits:?}");
    }

    /// The first live call: "langfuse port homebox" ranked a 1050-line page above the page
    /// holding the answer, because "supports" contains "port" hundreds of times.
    /// RED IF: term matching loses its word boundaries, or volume outranks coverage again.
    #[test]
    fn a_long_page_of_near_misses_loses_to_the_page_that_answers() {
        let dir = tempfile::tempdir().unwrap();
        let noise = "entailment: supports, reporting on imports, and the port is open\n".repeat(400);
        std::fs::write(dir.path().join("evidence.md"), noise).unwrap();
        std::fs::write(
            dir.path().join("home-infra.md"),
            "Langfuse runs on the homebox at port 3300.\n",
        )
        .unwrap();
        let pages = vec!["evidence".to_string(), "home-infra".to_string()];
        let hits = search(dir.path(), &pages, &[], "langfuse port homebox", 3, 5);
        assert_eq!(hits[0].page, "home-infra", "{hits:?}");
        assert!(!contains_word("supports", "port"));
        assert!(contains_word("on port 3300", "port"));
        // The noise page matches ONE term 400 times and still loses to the page holding all three.
        assert_eq!(hits.iter().position(|h| h.page == "evidence"), Some(1), "{hits:?}");
    }

    #[test]
    fn ranks_the_page_whose_id_matches_first() {
        let (dir, pages) = wiki();
        let hits = search(dir.path(), &pages, &[], "aws operations sqs", 3, 5);
        assert_eq!(hits[0].page, "aws-operations", "{hits:?}");
    }

    /// The page id is searchable text. "aws" appears nowhere in that page's body, so without
    /// the id counting as a line the page scores zero and the answer is "no page matched".
    #[test]
    fn a_word_that_appears_only_in_a_page_id_still_finds_that_page() {
        let (dir, pages) = wiki();
        let hits = search(dir.path(), &pages, &[], "aws", 3, 5);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].page, "aws-operations");
    }

    #[test]
    fn a_query_of_only_short_words_matches_nothing_rather_than_everything() {
        let (dir, pages) = wiki();
        assert!(terms("is it on us or in it").is_empty());
        assert!(search(dir.path(), &pages, &[], "is it on us", 3, 5).is_empty());
    }

    #[test]
    fn no_match_says_so_and_names_the_wiki() {
        let (dir, pages) = wiki();
        let hits = search(dir.path(), &pages, &[], "kubernetes helm chart", 3, 5);
        assert!(hits.is_empty());
        let text = render(dir.path(), &hits, &terms("kubernetes helm chart"));
        assert!(text.contains("No wiki page matched"), "{text}");
        assert!(text.contains(&dir.path().display().to_string()), "{text}");
    }

    #[test]
    fn lines_come_back_in_file_order_with_their_numbers() {
        let (dir, pages) = wiki();
        let hits = search(dir.path(), &pages, &[], "homebox langfuse 192.168.2.110", 3, 5);
        let nums: Vec<usize> = hits[0].lines.iter().map(|(n, _)| *n).collect();
        let mut sorted = nums.clone();
        sorted.sort_unstable();
        assert_eq!(nums, sorted, "{hits:?}");
        let text = render(dir.path(), &hits, &terms("homebox"));
        assert!(text.contains("L4:") && text.contains("home-infra.md"), "{text}");
    }

    fn with_reading(dir: &Path, list: &str, pages: &[(&str, &str)]) {
        let r = dir.join("reading");
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(r.join("_pages.json"), list).unwrap();
        for (id, body) in pages {
            std::fs::write(r.join(format!("{id}.md")), body).unwrap();
        }
    }

    /// Mneme handoff, 2026-10-11: reading pages are searched from `reading/_pages.json`, come
    /// back labelled as reading, and lose a tie to a rule page.
    /// RED IF: a reading hit is unlabelled, a tie puts the bookmark above the rule, or a page
    /// that is in the reading folder but not in `_pages.json` is searched.
    #[test]
    fn reading_pages_are_searched_labelled_and_lose_ties_to_rules() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("orchestration.md"), "Prefect runs the scheduled flows.\n").unwrap();
        with_reading(
            dir.path(),
            r#"[{"id": "reading-loops", "title": "Reading: Loops", "count": 3, "unit": "claims"}]"#,
            &[
                ("reading-loops", "Prefect runs the scheduled flows.\n"),
                ("reading-unlisted", "Prefect runs the scheduled flows.\n"),
            ],
        );
        let reading = reading_pages(dir.path());
        assert_eq!(reading, vec!["reading-loops".to_string()]);
        let pages = vec!["orchestration".to_string()];
        // Same text, same score: the rule page must come first.
        let hits = search(dir.path(), &pages, &reading, "prefect scheduled flows", 5, 5);
        let order: Vec<(&str, bool)> = hits.iter().map(|h| (h.page.as_str(), h.reading)).collect();
        assert_eq!(order, vec![("orchestration", false), ("reading-loops", true)], "{hits:?}");
        assert_eq!(hits[0].score, hits[1].score, "the fixture must be a real tie");

        let text = render(dir.path(), &hits, &terms("prefect scheduled flows"));
        assert!(text.contains("## reading-loops (reading/reading-loops.md) READING: a saved bookmark, not a rule"), "{text}");
        assert!(text.contains("## orchestration (orchestration.md)\n"), "{text}");
        assert!(text.contains("never as a rule"), "{text}");

        // A better-matching bookmark still outranks a weaker rule page: only TIES go to rules.
        let hits = search(dir.path(), &pages, &reading, "prefect loops", 5, 5);
        assert_eq!(hits[0].page, "reading-loops", "{hits:?}");
    }

    /// RED IF: a missing or malformed `_pages.json` errors, or stops the rule pages being searched.
    #[test]
    fn a_missing_or_broken_reading_list_is_a_no_op() {
        let (dir, pages) = wiki();
        assert!(reading_pages(dir.path()).is_empty(), "no reading folder at all");
        with_reading(dir.path(), "not json", &[]);
        assert!(reading_pages(dir.path()).is_empty(), "a list that does not parse");
        with_reading(dir.path(), r#"[{"title": "no id"}]"#, &[]);
        assert!(reading_pages(dir.path()).is_empty(), "an entry without an id");
        let hits = search(dir.path(), &pages, &reading_pages(dir.path()), "langfuse", 3, 5);
        assert_eq!(hits.len(), 1, "the rule pages are still searched: {hits:?}");
        assert!(!render(dir.path(), &hits, &terms("langfuse")).contains("READING"));
    }

    #[test]
    fn a_long_line_is_truncated_rather_than_dumped() {
        let dir = tempfile::tempdir().unwrap();
        let long = "langfuse ".repeat(200);
        std::fs::write(dir.path().join("p.md"), format!("{long}\n")).unwrap();
        let hits = search(dir.path(), &["p".to_string()], &[], "langfuse", 3, 5);
        assert!(hits[0].lines[0].1.chars().count() <= MAX_LINE_CHARS + 3, "{:?}", hits[0].lines[0]);
    }
}
