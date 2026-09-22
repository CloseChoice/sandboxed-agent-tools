use std::cell::RefCell;

mod bindings {
    #[cfg(target_arch = "wasm32")]
    use super::Component;
    wit_bindgen::generate!({ path: "wit" });
    #[cfg(target_arch = "wasm32")]
    export!(Component with_types_in self);
}

#[cfg(target_arch = "wasm32")]
use bindings::exports::local::fff_fuzzy::search::Guest;
use bindings::exports::local::fff_fuzzy::search::{GuestFinder, Highlight, Hit};
use neo_frizbee::{Config, Matcher, Scoring, UnicodeMatching};

#[cfg(target_arch = "wasm32")]
struct Component;

#[cfg(target_arch = "wasm32")]
impl Guest for Component {
    type Finder = Finder;

    fn match_paths(paths: Vec<String>, query: String, limit: u32, max_typos: u16)
        -> Result<Vec<Hit>, String>
    {
        let finder = Finder::new();
        finder.set_candidates(paths)?;
        finder.search(query, limit, max_typos)
    }
}

#[derive(Default)]
pub struct Finder {
    candidates: RefCell<Vec<String>>,
}

// Bound matcher matrix dimensions and memory retained by each finder.
const MAX_QUERY_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4096;
const MAX_CANDIDATES: usize = 1_000_000;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

impl GuestFinder for Finder {
    fn new() -> Self {
        Self::default()
    }

    fn set_candidates(&self, paths: Vec<String>) -> Result<(), String> {
        if paths.len() > MAX_CANDIDATES {
            return Err("at most 1000000 candidates are supported".into());
        }
        let mut total = 0usize;
        for path in &paths {
            if path.len() > MAX_PATH_BYTES {
                return Err("candidate exceeds 4096 UTF-8 bytes".into());
            }
            total += path.len();
            if total > MAX_TOTAL_BYTES {
                return Err("candidate text exceeds 64 MiB".into());
            }
        }
        *self.candidates.borrow_mut() = paths;
        Ok(())
    }

    fn search(&self, query: String, limit: u32, max_typos: u16) -> Result<Vec<Hit>, String> {
        if query.len() > MAX_QUERY_BYTES {
            return Err("query exceeds 256 UTF-8 bytes".into());
        }
        if max_typos > 2 {
            return Err("max-typos must be between 0 and 2".into());
        }
        if limit == 0 {
            return Ok(vec![]);
        }
        let candidates = self.candidates.borrow();
        if query.is_empty() {
            return Ok((0..candidates.len().min(limit as usize))
                .map(|id| Hit { id: id as u32, score: 0, highlights: vec![] })
                .collect());
        }

        // FFF also boosts matching case only for queries containing uppercase.
        let uppercase = query.chars().any(char::is_uppercase);
        let config = Config {
            max_typos: Some(max_typos),
            // Character-based matching even for ASCII queries against Unicode paths
            // keeps highlight coordinates unambiguous.
            unicode: UnicodeMatching::Always,
            scoring: Scoring {
                capitalization_bonus: if uppercase { 8 } else { 0 },
                matching_case_bonus: if uppercase { 4 } else { 0 },
                ..Scoring::default()
            },
            ..Config::default()
        };
        let mut matcher = Matcher::new(&query, &config);
        let mut matches = matcher.match_list(&candidates);
        matches.truncate(limit as usize);
        // Alignment is expensive: calculate it only for the returned page.
        let mut results = Vec::with_capacity(matches.len());
        for matched in matches {
            let path = &candidates[matched.index as usize];
            let alignment = matcher.match_one_indices(path, matched.index)
                .ok_or_else(|| "matcher score/alignment disagreement".to_string())?;
            results.push(Hit {
                id: matched.index,
                score: matched.score,
                highlights: byte_ranges(path, alignment.indices),
            });
        }
        Ok(results)
    }
}

fn byte_ranges(path: &str, mut indices: Vec<usize>) -> Vec<Highlight> {
    indices.sort_unstable();
    indices.dedup();
    let mut ranges: Vec<Highlight> = vec![];
    for offset in indices {
        if offset >= path.len() {
            continue;
        }
        // neo_frizbee 0.11 emits byte indices (including every byte of
        // a matched Unicode scalar), despite its older char-index docstring.
        let mut start = offset;
        while !path.is_char_boundary(start) { start -= 1; }
        let mut end = offset + 1;
        while !path.is_char_boundary(end) { end += 1; }
        if let Some(last) = ranges.last_mut()
            && last.end >= start as u32
        {
            last.end = last.end.max(end as u32);
        } else {
            ranges.push(Highlight { start: start as u32, end: end as u32 });
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_state_and_validation() {
        let finder = Finder::new();
        finder.set_candidates(vec!["src/main.rs".into(), "docs/readme.md".into(), "src/main.rs".into()]).unwrap();
        let hits = finder.search("main".into(), 10, 0).unwrap();
        assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(hits[0].highlights.iter().map(|h| (h.start, h.end)).collect::<Vec<_>>(), vec![(4, 8)]);
        assert!(finder.set_candidates(vec!["x".repeat(MAX_PATH_BYTES + 1)]).is_err());
        assert_eq!(finder.search("".into(), 10, 0).unwrap().len(), 3);
        assert!(finder.search("x".repeat(MAX_QUERY_BYTES + 1), 10, 0).is_err());
        finder.set_candidates(vec![]).unwrap();
        assert!(finder.search("main".into(), 10, 0).unwrap().is_empty());
    }

    #[test]
    fn unicode_highlights_are_utf8_ranges() {
        let finder = Finder::new();
        finder.set_candidates(vec!["📁/café.rs".into()]).unwrap();
        for query in ["café", "caf", "é", "rs"] {
            let hits = finder.search(query.into(), 10, 0).unwrap();
            assert_eq!(hits.len(), 1, "query: {query}");
            let path = "📁/café.rs";
            let highlighted: String = hits[0].highlights.iter()
                .map(|h| &path[h.start as usize..h.end as usize]).collect();
            assert_eq!(highlighted, query);
        }
    }

    #[test]
    fn mixed_unicode_widths_and_case_match() {
        let finder = Finder::new();
        for query in ["aé", "éa", "界a😀é", "café", "Éa"] {
            for prefix in ["", "📁/", "abcdefghijklmnop/"] {
                let path = format!("{prefix}{query}/tail");
                finder.set_candidates(vec![path.clone()]).unwrap();
                for typos in 0..=2 {
                    assert_eq!(finder.search(query.into(), 10, typos).unwrap().len(), 1,
                        "query={query}, path={path}, typos={typos}");
                }
            }
        }
        finder.set_candidates(vec!["éa".into()]).unwrap();
        assert_eq!(finder.search("Éa".into(), 10, 0).unwrap().len(), 1);
    }

    #[test]
    fn typo_budget_and_limits() {
        let finder = Finder::new();
        finder.set_candidates(vec!["schema.rs".into()]).unwrap();
        assert!(finder.search("schexma".into(), 10, 0).unwrap().is_empty());
        assert_eq!(finder.search("schexma".into(), 10, 1).unwrap().len(), 1);
        assert!(finder.search("schema".into(), 0, 0).unwrap().is_empty());
        assert!(finder.search("schema".into(), 10, 3).is_err());
    }
}
