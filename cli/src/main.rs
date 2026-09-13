use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::process::ExitCode;

fn run() -> Result<bool, String> {
    let mut args = std::env::args().skip(1);
    let mut before = None;
    let mut after = None;
    let mut context = 0;
    let mut show_context = false;
    let query = loop {
        let arg = args
            .next()
            .ok_or("usage: file-search [-A N] [-B N] [-C N] [--] QUERY FILE...")?;
        if arg == "--help" || arg == "-h" {
            println!(
                "usage: file-search [-A N] [-B N] [-C N] [--] QUERY FILE...\n-A N: lines after; -B N: lines before; -C N: both sides.\n-A and -B override -C for their side. Options precede QUERY; -A2 also works.\nLiteral, case-sensitive UTF-8 search. Matches: FILE:LINE:TEXT; context: FILE-LINE-TEXT.\nExit codes: 0 matches, 1 no matches or error (errors go to stderr). Files require WASI directory grants."
            );
            return Ok(true);
        }
        if arg == "--" {
            break args.next().ok_or("missing QUERY after --")?;
        }
        if arg.starts_with('-') && arg != "-" {
            let (flag, inline) = if let Some(value) = arg.strip_prefix("-A") {
                ('A', value)
            } else if let Some(value) = arg.strip_prefix("-B") {
                ('B', value)
            } else if let Some(value) = arg.strip_prefix("-C") {
                ('C', value)
            } else {
                return Err(format!(
                    "unknown option {arg}; use -- before a query starting with -"
                ));
            };
            let value = if inline.is_empty() {
                args.next()
                    .ok_or_else(|| format!("missing count for -{flag}"))?
            } else {
                inline.to_owned()
            };
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!(
                    "invalid count for -{flag}: {value}; expected a non-negative integer"
                ));
            }
            let count: usize = value
                .parse()
                .map_err(|_| format!("count too large for -{flag}: {value}"))?;
            show_context = true;
            match flag {
                'A' => after = Some(count),
                'B' => before = Some(count),
                _ => context = count,
            }
        } else {
            break arg;
        }
    };
    let before = before.unwrap_or(context);
    let after = after.unwrap_or(context);
    let paths: Vec<_> = args.collect();
    if paths.is_empty() {
        return Err("provide at least one file path".into());
    }
    if query.contains(['\n', '\r']) {
        return Err("QUERY must not contain line breaks".into());
    }
    let stdout = io::stdout();
    let mut output = stdout.lock();
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
                    writeln!(output, "{path}:{}:{line}", index + 1)?;
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

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("file-search: {error}");
            ExitCode::from(1)
        }
    }
}
use std::collections::VecDeque;
