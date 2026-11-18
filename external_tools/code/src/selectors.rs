//! Path selection is performed inside the guest and never grants authority.
use globset::{GlobBuilder, GlobMatcher};
use regex::Regex;
use std::path::{Path, PathBuf};

#[derive(Clone)]
enum Pattern {
    Glob(GlobMatcher),
    Regex(Regex),
}

impl Pattern {
    fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() || text.len() > 4096 {
            return Err("path patterns must contain 1..4096 bytes".into());
        }
        if let Some(expr) = text.strip_prefix("regex:") {
            Regex::new(expr)
                .map(Self::Regex)
                .map_err(|_| "invalid path regex".into())
        } else {
            GlobBuilder::new(text.strip_prefix("glob:").unwrap_or(text))
                .literal_separator(true)
                .backslash_escape(false)
                .build()
                .map(|glob| Self::Glob(glob.compile_matcher()))
                .map_err(|_| "invalid path glob".into())
        }
    }

    fn matches(&self, path: &Path) -> bool {
        // Use '/' for matching on every platform.
        let normalized = path.to_string_lossy().replace('\\', "/");
        match self {
            Self::Glob(pattern) => pattern.is_match(&normalized),
            Self::Regex(pattern) => pattern.is_match(&normalized),
        }
    }
}

#[derive(Clone)]
struct Selector {
    root: PathBuf,
    pattern: Option<Pattern>,
    relative: bool,
}

#[derive(Clone)]
pub struct Selection {
    pub roots: Vec<PathBuf>,
    selectors: Vec<Selector>,
    exclusions: Vec<(Pattern, bool)>,
}

impl Selection {
    pub fn new(paths: &[String], exclude: &[String]) -> Result<Self, String> {
        let mut selectors = Vec::new();
        let mut roots = Vec::new();
        for text in paths {
            if text.is_empty() || text.len() > 4096 {
                return Err("path selectors must contain 1..4096 bytes".into());
            }
            let selector = if let Some(spec) = text.strip_prefix("regex:") {
                let (root, expr) = spec
                    .split_once("::")
                    .ok_or("regex selectors require regex:ROOT::EXPR")?;
                if root.is_empty() || expr.is_empty() {
                    return Err("regex selectors need a nonempty root and expression".into());
                }
                Selector {
                    root: root.into(),
                    pattern: Some(Pattern::parse(&format!("regex:{expr}"))?),
                    relative: true,
                }
            } else {
                let pattern = text.strip_prefix("glob:").unwrap_or(text);
                // Existing paths remain literal, including filenames with metacharacters.
                if !text.starts_with("glob:") && Path::new(text).exists() {
                    Selector {
                        root: text.into(),
                        pattern: None,
                        relative: false,
                    }
                } else if let Some(meta) = pattern.find(['*', '?', '[', '{']) {
                    let prefix = &pattern[..meta];
                    let root = prefix
                        .rfind('/')
                        .map(|i| if i == 0 { "/" } else { &prefix[..i] })
                        .unwrap_or(".");
                    Selector {
                        root: root.into(),
                        pattern: Some(Pattern::parse(text)?),
                        relative: false,
                    }
                } else {
                    // Validate explicitly prefixed patterns even when they have no wildcards.
                    let matcher = text
                        .starts_with("glob:")
                        .then(|| Pattern::parse(text))
                        .transpose()?;
                    Selector {
                        root: pattern.into(),
                        pattern: matcher,
                        relative: false,
                    }
                }
            };
            if !roots.contains(&selector.root) {
                roots.push(selector.root.clone());
            }
            selectors.push(selector);
        }
        if roots.is_empty() {
            return Err("at least one path selector is required".into());
        }
        let exclusions = exclude
            .iter()
            .map(|text| {
                let basename = !text.starts_with("regex:") && !text.contains('/');
                Ok((Pattern::parse(text)?, basename))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            roots,
            selectors,
            exclusions,
        })
    }

    pub fn excluded(&self, path: &Path) -> bool {
        // WalkBuilder always visits explicit roots; enforce excluded ancestors
        // for their descendants too, even when that root bypasses pruning.
        path.ancestors()
            .take_while(|candidate| self.roots.iter().any(|root| candidate.starts_with(root)))
            .any(|candidate| {
                self.exclusions.iter().any(|(pattern, basename)| {
                    pattern.matches(candidate)
                        || (*basename
                            && candidate
                                .file_name()
                                .is_some_and(|name| pattern.matches(Path::new(name))))
                        || self.roots.iter().any(|root| {
                            candidate
                                .strip_prefix(root)
                                .is_ok_and(|relative| pattern.matches(relative))
                        })
                })
            })
    }

    pub fn includes(&self, path: &Path) -> bool {
        !self.excluded(path)
            && self.selectors.iter().any(|selector| {
                let Ok(relative) = path.strip_prefix(&selector.root) else {
                    return false;
                };
                selector.pattern.as_ref().is_none_or(|pattern| {
                    pattern.matches(if selector.relative { relative } else { path })
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selection(paths: &[&str], excludes: &[&str]) -> Selection {
        Selection::new(
            &paths.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &excludes.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
        .unwrap()
    }
    #[test]
    fn recursive_globs_and_regex_roots_are_combined() {
        let s = selection(
            &[
                "glob:/project/src/**/*.ts",
                "regex:/project/tests::(^|/)test_[^/]+\\.py$",
            ],
            &[],
        );
        assert!(s.includes(Path::new("/project/src/a.ts")));
        assert!(s.includes(Path::new("/project/src/nested/a.ts")));
        assert!(s.includes(Path::new("/project/tests/test_a.py")));
        assert!(!s.includes(Path::new("/project/src/a.js")));
        assert!(!s.includes(Path::new("/other/test_a.py")));
    }
    #[test]
    fn exclusions_win_and_prune_directories() {
        let s = selection(
            &["/project", "/project/node_modules"],
            &["node_modules", "**/*.test.ts", "regex:(^|/)generated(/|$)"],
        );
        assert!(s.excluded(Path::new("/project/node_modules")));
        assert!(!s.includes(Path::new("/project/node_modules/dep.ts")));
        assert!(s.excluded(Path::new("/project/src/a.test.ts")));
        assert!(s.excluded(Path::new("/project/src/generated/a.ts")));
        assert!(s.includes(Path::new("/project/src/a.ts")));
    }
    #[test]
    fn glob_syntax_and_invalid_selectors() {
        let s = selection(&["glob:/project/*.{ts,tsx}", "glob:/other/a?.[jt]s"], &[]);
        assert!(s.includes(Path::new("/project/a.tsx")));
        assert!(!s.includes(Path::new("/project/nested/a.ts")));
        assert!(s.includes(Path::new("/other/ab.js")));
        for bad in [
            "glob:/project/[",
            "regex:/project::[",
            "regex:missing-root",
            "",
        ] {
            assert!(Selection::new(&[bad.into()], &[]).is_err(), "{bad}");
        }
        assert!(Selection::new(&["/project".into()], &["regex:[".into()]).is_err());
    }
}
