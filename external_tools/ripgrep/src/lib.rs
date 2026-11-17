use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use ignore::WalkBuilder;
use serde::Serialize;
use std::{fmt::Write as _, io, path::Path};

wit_bindgen::generate!({ world: "ripgrep" });

const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Serialize)]
struct Line {
    path: String,
    line: u64,
    text: String,
    matched: bool,
}

#[derive(Default, Serialize)]
struct Output {
    results: Vec<Line>,
    // Conservative: true means a limit stopped the search, even if there were
    // no further matches. It never claims an incomplete search is complete.
    truncated: bool,
    skipped_paths: usize,
    #[serde(skip)]
    text_bytes: usize,
}

impl Output {
    fn text(&self, context: bool) -> String {
        let mut text = String::new();
        let mut previous: Option<&Line> = None;
        for line in &self.results {
            if previous.is_none_or(|prev| prev.path != line.path) {
                if previous.is_some() {
                    text.push('\n');
                }
                writeln!(text, "{}", line.path).unwrap();
            } else if context && previous.is_some_and(|prev| line.line != prev.line + 1) {
                text.push_str("--\n");
            }
            let separator = if line.matched { ':' } else { '-' };
            writeln!(text, "{}{separator}{}", line.line, line.text).unwrap();
            previous = Some(line);
        }
        if self.truncated {
            text.push_str("[results truncated; increase --max-results or narrow the search]\n");
        }
        if self.skipped_paths > 0 {
            writeln!(
                text,
                "[skipped {} paths: permission denied]",
                self.skipped_paths
            )
            .unwrap();
        }
        // SAT supplies the final newline when presenting a WIT string.
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }
}

struct Collector<'a> {
    output: &'a mut Output,
    path: &'a str,
    limit: usize,
}

impl Collector<'_> {
    fn push(&mut self, line: u64, bytes: &[u8], matched: bool) -> bool {
        let text = String::from_utf8_lossy(bytes);
        let text = text.strip_suffix('\n').unwrap_or(&text);
        let text = text.strip_suffix('\r').unwrap_or(text);
        // Include repeated paths in the budget as well as line contents.
        let size = text.len().saturating_add(self.path.len());
        if self.output.results.len() >= self.limit
            || self.output.text_bytes.saturating_add(size) > MAX_TEXT_BYTES
        {
            self.output.truncated = true;
            return false;
        }
        self.output.text_bytes += size;
        self.output.results.push(Line {
            path: self.path.into(),
            line,
            text: text.into(),
            matched,
        });
        if self.output.results.len() == self.limit {
            self.output.truncated = true;
            return false;
        }
        true
    }
}

impl Sink for Collector<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, mat: &SinkMatch<'_>) -> io::Result<bool> {
        Ok(self.push(mat.line_number().unwrap(), mat.bytes(), true))
    }

    fn context(&mut self, _: &Searcher, ctx: &SinkContext<'_>) -> io::Result<bool> {
        Ok(self.push(ctx.line_number().unwrap(), ctx.bytes(), false))
    }
}

struct Ripgrep;

impl Guest for Ripgrep {
    fn run(
        pattern: String,
        paths: Option<Vec<String>>,
        ignore_case: bool,
        fixed_strings: bool,
        hidden: bool,
        no_ignore: bool,
        context: u32,
        max_results: u32,
        format: String,
    ) -> Result<String, String> {
        if !matches!(format.as_str(), "text" | "json") {
            return Err("format must be text or json".into());
        }
        if context > 100 || !(1..=10_000).contains(&max_results) {
            return Err("context must be 0..100 and max-results must be 1..10000".into());
        }
        if pattern.len() > 4096 {
            return Err("pattern exceeds 4096 bytes".into());
        }
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(ignore_case)
            .fixed_strings(fixed_strings)
            .multi_line(true)
            .line_terminator(Some(b'\n'))
            .build(&pattern)
            .map_err(|_| "invalid or oversized regex pattern")?;
        let paths = paths.unwrap_or_else(|| vec![".".into()]);
        if paths.is_empty() || paths.len() > 128 {
            return Err("supply 1..128 search paths".into());
        }
        let mut walker = WalkBuilder::new(&paths[0]);
        for path in &paths[1..] {
            walker.add(path);
        }
        // Use only ignore files inside the selected tree. Never consult host
        // config, parent directories, or Git's external exclude configuration.
        walker
            .parents(false)
            .git_global(false)
            .git_exclude(false)
            .require_git(false)
            .hidden(!hidden)
            .ignore(!no_ignore)
            .git_ignore(!no_ignore)
            .follow_links(false)
            .sort_by_file_path(Path::cmp);
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .before_context(context as usize)
            .after_context(context as usize)
            .binary_detection(BinaryDetection::quit(b'\0'))
            .heap_limit(Some(16 * 1024 * 1024))
            .build();
        let mut output = Output::default();
        for entry in walker.build() {
            // Omit OS error strings, which may contain host paths. Permission
            // denials skip only the affected entry; unrelated traversal errors fail.
            let entry = match entry {
                Ok(entry) => entry,
                Err(error)
                    if error
                        .io_error()
                        .is_some_and(|error| error.kind() == io::ErrorKind::PermissionDenied) =>
                {
                    output.skipped_paths += 1;
                    continue;
                }
                Err(_) => return Err("cannot traverse search path (missing or denied)".into()),
            };
            if entry.error().is_some() {
                return Err("cannot read ignore rules".into());
            }
            if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                continue;
            }
            if entry.depth() > 0 && !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            // Explicit symlink inputs can be followed by the WASI filesystem;
            // the host still confines their targets to the granted roots.
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                match std::fs::metadata(entry.path()) {
                    Ok(metadata) if metadata.is_file() => {}
                    Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                        output.skipped_paths += 1;
                        continue;
                    }
                    _ => return Err("search path is not an accessible regular file".into()),
                }
            }
            let path = entry.path().to_string_lossy();
            let sink = Collector {
                output: &mut output,
                path: &path,
                limit: max_results as usize,
            };
            match searcher.search_path(&matcher, entry.path(), sink) {
                Ok(()) => {}
                // The host rejects hard-linked files as well as other denied reads.
                // Keep searching readable files without weakening that protection.
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    output.skipped_paths += 1;
                    continue;
                }
                Err(_) => {
                    return Err("cannot search file (missing or line exceeds buffer limit)".into());
                }
            }
            if output.truncated {
                break;
            }
        }
        if format == "json" {
            serde_json::to_string(&output).map_err(|_| "cannot encode search results".into())
        } else {
            Ok(output.text(context > 0))
        }
    }
}

export!(Ripgrep);
