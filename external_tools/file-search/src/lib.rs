use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};

/// Search explicit files and stream matching lines to the supplied output.
pub fn search_to(
    query: &str,
    paths: &[String],
    before: usize,
    after: usize,
    show_context: bool,
    output: &mut impl Write,
) -> Result<bool, String> {
    let mut matched = false;
    let mut failed = false;
    let mut printed_any = false;
    for path in paths {
        let result = (|| -> io::Result<()> {
            let file = File::open(&path)?;
            if !file.metadata()?.is_file() {
                return Err(io::Error::other("expected a regular file"));
            }
            let mut pending = VecDeque::new();
            let mut remaining = 0;
            let mut last_printed = None;
            for (index, line) in BufReader::new(file).lines().enumerate() {
                let line = line?;
                if line.contains(&query) {
                    let start = pending.front().map_or(index, |(i, _)| *i);
                    if show_context
                        && printed_any
                        && last_printed.is_none_or(|last| start > last + 1)
                    {
                        writeln!(output, "--")?;
                    }
                    for (i, text) in pending.drain(..) {
                        writeln!(output, "{path}-{}-{text}", i + 1)?;
                    }
                    writeln!(output, "{path}:{} {line}", index + 1)?;
                    matched = true;
                    printed_any = true;
                    last_printed = Some(index);
                    remaining = after;
                } else if remaining > 0 {
                    writeln!(output, "{path}-{}-{line}", index + 1)?;
                    last_printed = Some(index);
                    remaining -= 1;
                } else if before > 0 {
                    pending.push_back((index, line));
                    if pending.len() > before {
                        pending.pop_front();
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("file-search: {path}: {error}");
            failed = true;
        }
    }
    if failed {
        Err("one or more files could not be searched".into())
    } else {
        Ok(matched)
    }
}

#[cfg(target_arch = "wasm32")]
mod component {
    wit_bindgen::generate!({ path: "wit", world: "file-search" });
    struct FileSearch;
    impl Guest for FileSearch {
        fn search(query: String, paths: String, before: Option<u32>, after: Option<u32>, context: u32)
            -> Result<String, String>
        {
            if query.contains(['\n', '\r']) {
                return Err("QUERY must not contain line breaks".into());
            }
            let paths: Vec<String> = paths.lines().map(str::to_owned).collect();
            if paths.is_empty() {
                return Err("provide at least one file path".into());
            }
            let mut output = Vec::new();
            super::search_to(&query, &paths, before.unwrap_or(context) as usize,
                after.unwrap_or(context) as usize,
                before.is_some() || after.is_some() || context > 0, &mut output)?;
            String::from_utf8(output).map_err(|_| "output is not UTF-8".into())
        }
    }
    export!(FileSearch);
}
