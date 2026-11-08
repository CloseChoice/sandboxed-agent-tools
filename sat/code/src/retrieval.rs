// Syntax selection and filesystem traversal live in the guest, never the host.
use ignore::WalkBuilder;
use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs::File,
    io::Read,
    path::Path,
};
use tree_sitter::{Language, Node, Parser};

const MAX_FILE: usize = 4 * 1024 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_HITS: usize = 10_000;
const MAX_PARSED_FILES: usize = 128;

pub struct Request {
    pub operation: String,
    pub query: Option<String>,
    pub paths: Vec<String>,
    pub mode: String,
    pub context: String,
    pub position: Option<usize>,
    pub include_docs: bool,
    pub include_parent_signature: bool,
    pub max_results: usize,
    pub max_bytes: usize,
    pub format: String,
}

#[derive(Clone, Debug, Serialize)]
struct SourceRange {
    start_byte: usize,
    end_byte: usize,
    start_line: usize,
    end_line: usize,
    start_column_byte: usize,
    end_column_byte: usize,
}

struct Coordinates(Vec<u32>);

impl Coordinates {
    fn new(source: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(i, byte)| (byte == b'\n').then_some((i + 1) as u32)),
        );
        Self(starts)
    }

    fn range(&self, start: usize, end: usize) -> SourceRange {
        let point = |offset: usize| {
            let row = self.0.partition_point(|&start| start as usize <= offset) - 1;
            (row + 1, offset - self.0[row] as usize)
        };
        let (start_line, start_column_byte) = point(start);
        let (end_line, end_column_byte) = point(end);
        SourceRange {
            start_byte: start,
            end_byte: end,
            start_line,
            end_line,
            start_column_byte,
            end_column_byte,
        }
    }
}

#[derive(Serialize)]
struct Definition {
    path: String,
    language: String,
    symbol: String,
    kind: String,
    range: SourceRange,
    definition_range: SourceRange,
    source: Option<String>,
    signature: String,
    parent_signature: Option<String>,
    hits: Vec<SourceRange>,
    file_hash: String,
    parse_errors: bool,
    complete: bool,
    status: &'static str,
}

#[derive(Serialize)]
struct Diagnostic {
    path: String,
    status: &'static str,
    range: Option<SourceRange>,
}

#[derive(Default, Serialize)]
struct Output {
    results: Vec<Definition>,
    diagnostics: Vec<Diagnostic>,
    truncated: bool,
    truncation_reason: Option<&'static str>,
    parsed_files: usize,
}

impl Output {
    fn stop(&mut self, reason: &'static str) {
        self.truncated = true;
        self.truncation_reason = Some(reason);
    }

    fn text(&self) -> String {
        let mut text = String::new();
        for result in &self.results {
            if !text.is_empty() {
                text.push('\n');
            }
            writeln!(
                text,
                "{}:{}-{}  {} [{}]",
                result.path,
                result.range.start_line,
                result.range.end_line,
                result.symbol,
                result.kind
            )
            .unwrap();
            if let Some(parent) = &result.parent_signature {
                writeln!(text, "owner: {parent}").unwrap();
            }
            if let Some(source) = &result.source {
                text.push_str(source);
                if !source.ends_with('\n') {
                    text.push('\n');
                }
            } else {
                writeln!(text, "{}", result.signature).unwrap();
            }
            if result.status != "ok" {
                writeln!(text, "[{}; complete=false]", result.status).unwrap();
            }
            if result.parse_errors {
                text.push_str("[file contains parse errors]\n");
            }
        }
        for diagnostic in &self.diagnostics {
            writeln!(text, "{}: [{}]", diagnostic.path, diagnostic.status).unwrap();
        }
        if self.truncated {
            writeln!(text, "[truncated: {}]", self.truncation_reason.unwrap()).unwrap();
        }
        text.trim_end_matches('\n').into()
    }
}

fn language(path: &Path) -> Option<(&'static str, Language)> {
    Some(match path.extension()?.to_str()? {
        "py" | "pyi" => ("python", tree_sitter_python::LANGUAGE.into()),
        "rs" => ("rust", tree_sitter_rust::LANGUAGE.into()),
        "js" | "jsx" | "mjs" | "cjs" => ("javascript", tree_sitter_javascript::LANGUAGE.into()),
        "ts" | "mts" | "cts" => (
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        ),
        "tsx" => ("tsx", tree_sitter_typescript::LANGUAGE_TSX.into()),
        _ => return None,
    })
}

fn callable(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_item"
            | "function_definition"
            | "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "method_definition"
    )
}

fn owner(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "class_definition"
            | "class_declaration"
            | "class"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "impl_item"
            | "interface_declaration"
            | "enum_declaration"
    )
}

fn signature(node: Node<'_>, source: &str) -> String {
    let mut end = node.child_by_field_name("body").map_or_else(
        || {
            source[node.byte_range()]
                .find('\n')
                .map_or(node.end_byte(), |n| node.start_byte() + n)
        },
        |body| body.start_byte(),
    );
    if matches!(node.kind(), "class_definition" | "function_definition") {
        // Python's block may begin after comments following the header. End
        // the signature at the header's colon, not the first statement.
        let mut cursor = node.walk();
        if let Some(colon) = node.children(&mut cursor).find(|child| child.kind() == ":") {
            end = colon.end_byte();
        }
    }
    // Signatures themselves can be adversarially large. Mark clipping instead
    // of silently claiming to have returned the entire signature.
    let text = source[node.start_byte()..end].trim();
    if text.len() <= 4096 {
        text.into()
    } else {
        format!(
            "{} [signature truncated]",
            text.chars().take(1024).collect::<String>()
        )
    }
}

fn symbol(node: Node<'_>, source: &str) -> String {
    if let Some(name) = node.child_by_field_name("name") {
        return source[name.byte_range()].chars().take(256).collect();
    }
    if let Some(parent) = node.parent() {
        let name = match parent.kind() {
            "variable_declarator" => parent.child_by_field_name("name"),
            "pair" => parent.child_by_field_name("key"),
            "assignment_expression" | "assignment" => parent.child_by_field_name("left"),
            _ => None,
        };
        if let Some(name) = name {
            return source[name.byte_range()].chars().take(256).collect();
        }
    }
    if node.kind() == "impl_item" {
        signature(node, source)
    } else {
        "<anonymous>".into()
    }
}

fn source_node(node: Node<'_>, docs: bool) -> Node<'_> {
    let mut result = node;
    if let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "variable_declarator" | "assignment_expression"
        ) {
            // Keep an assigned arrow/function's binding name with its source.
            result = parent;
            if let Some(declaration) = parent.parent()
                && matches!(
                    declaration.kind(),
                    "lexical_declaration" | "variable_declaration"
                )
                && declaration.named_child_count() == 1
            {
                result = declaration;
            }
        } else if docs && parent.kind() == "decorated_definition" {
            result = parent;
        }
    }
    if let Some(parent) = result.parent()
        && parent.kind() == "export_statement"
    {
        result = parent;
    }
    result
}

fn attached_start(node: Node<'_>, source: &str, docs: bool) -> usize {
    let mut first = node;
    if docs {
        loop {
            let previous = first.prev_named_sibling().or_else(|| {
                // A Python comment above the first statement can be a sibling
                // of the block rather than a child of it.
                first
                    .parent()
                    .filter(|parent| parent.kind() == "block")
                    .and_then(|block| block.prev_named_sibling())
            });
            let Some(previous) = previous else {
                break;
            };
            let is_doc = previous.kind().contains("comment")
                || matches!(previous.kind(), "attribute_item" | "outer_attribute_item");
            let gap = &source[previous.end_byte()..first.start_byte()];
            if !is_doc
                || !gap.chars().all(char::is_whitespace)
                || gap.bytes().filter(|&b| b == b'\n').count() > 1
            {
                break;
            }
            first = previous;
        }
    }
    first.start_byte()
}

fn parent_signature(node: Node<'_>, source: &str) -> Option<String> {
    let mut parent = node.parent();
    while let Some(node) = parent {
        if owner(node) {
            return Some(signature(node, source));
        }
        parent = node.parent();
    }
    None
}

// Iterative traversal avoids using the guest's call stack for deep syntax trees.
fn declarations(root: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = root.walk();
    let mut result = Vec::new();
    loop {
        let node = cursor.node();
        if node.is_named() && (callable(node) || owner(node)) {
            result.push(node);
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return result;
            }
        }
    }
}

fn selected(node: Node<'_>, context: &str) -> bool {
    if context == "class" {
        owner(node)
    } else {
        callable(node)
    }
}

fn validate(request: &Request) -> Result<(), String> {
    if !matches!(
        request.operation.as_str(),
        "search" | "outline" | "read-symbol" | "read-enclosing"
    ) {
        return Err("operation must be search, outline, read-symbol, or read-enclosing".into());
    }
    if !matches!(request.mode.as_str(), "text" | "regex")
        || !matches!(
            request.context.as_str(),
            "function" | "outer-function" | "class"
        )
        || !matches!(request.format.as_str(), "text" | "json")
    {
        return Err("invalid mode, context, or format (see --help)".into());
    }
    if request.paths.is_empty()
        || request.paths.len() > 128
        || !(1..=1000).contains(&request.max_results)
        || !(1..=MAX_OUTPUT).contains(&request.max_bytes)
    {
        return Err(
            "paths must contain 1..128 roots, max-results 1..1000, max-bytes 1..1048576".into(),
        );
    }
    if matches!(request.operation.as_str(), "search" | "read-symbol") {
        if request
            .query
            .as_ref()
            .is_none_or(|q| q.is_empty() || q.len() > 4096)
        {
            return Err("search and read-symbol require a query of 1..4096 bytes".into());
        }
    } else if request.query.is_some() {
        return Err("this operation does not accept a query".into());
    }
    if request.operation == "read-enclosing" {
        if request.position.is_none()
            || request.paths.len() != 1
            || !Path::new(&request.paths[0]).is_file()
        {
            return Err(
                "read-enclosing requires --position and exactly one accessible file in --paths"
                    .into(),
            );
        }
    } else if request.position.is_some() {
        return Err("--position is only valid for read-enclosing".into());
    }
    Ok(())
}

pub fn run(request: Request) -> Result<String, String> {
    validate(&request)?;
    let pattern = if request.operation == "search" {
        let query = request.query.as_ref().unwrap();
        Some(
            Regex::new(&if request.mode == "text" {
                regex::escape(query)
            } else {
                query.clone()
            })
            .map_err(|_| "invalid or oversized regex")?,
        )
    } else {
        None
    };
    let mut walker = WalkBuilder::new(&request.paths[0]);
    for path in &request.paths[1..] {
        walker.add(path);
    }
    walker
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(false)
        .sort_by_file_path(Path::cmp);
    let mut output = Output::default();
    let mut seen_files = BTreeSet::new();
    let mut output_bytes = 0usize;
    let mut hit_count = 0usize;
    for entry in walker.build() {
        let entry = entry.map_err(|_| "cannot traverse search path (missing or denied)")?;
        if entry.error().is_some() {
            return Err("cannot read ignore rules".into());
        }
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            continue;
        }
        if entry.depth() > 0 && !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let Some((language_name, grammar)) = language(path) else {
            if entry.depth() == 0 {
                return Err("unsupported language; use .py, .rs, .js/.jsx, .ts, or .tsx".into());
            }
            continue;
        };
        if !seen_files.insert(path.to_path_buf()) {
            continue;
        }
        if !path.is_file() {
            return Err("search path is not an accessible regular file".into());
        }
        let path_text = path.to_string_lossy().into_owned();
        let file = File::open(path).map_err(|_| "cannot read source file (missing or denied)")?;
        let mut bytes = Vec::new();
        file.take((MAX_FILE + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "cannot read source file")?;
        let source = if bytes.len() > MAX_FILE {
            None
        } else {
            std::str::from_utf8(&bytes).ok()
        };
        let Some(source) = source else {
            output.diagnostics.push(Diagnostic {
                path: path_text,
                status: if bytes.len() > MAX_FILE {
                    "file-size-limit"
                } else {
                    "invalid-utf8"
                },
                range: None,
            });
            if output.diagnostics.len() >= 100 {
                output.stop("diagnostic-limit");
                break;
            }
            continue;
        };
        let mut hits = Vec::new();
        if let Some(pattern) = &pattern {
            for hit in pattern.find_iter(source) {
                if hit_count == MAX_HITS {
                    output.stop("hit-limit");
                    break;
                }
                hit_count += 1;
                hits.push((hit.start(), hit.end()));
            }
            if hits.is_empty() {
                if output.truncated {
                    break;
                } else {
                    continue;
                }
            }
        }
        if request.operation == "read-enclosing" {
            let offset = request.position.unwrap();
            if offset >= source.len() || !source.is_char_boundary(offset) {
                return Err("position must be a UTF-8 character boundary inside the file (zero-based bytes)".into());
            }
            hits.push((offset, offset));
        }
        if output.parsed_files == MAX_PARSED_FILES {
            output.stop("parsed-file-limit");
            break;
        }
        let mut parser = Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|_| "incompatible language grammar")?;
        let tree = parser.parse(source, None).ok_or("parsing interrupted")?;
        output.parsed_files += 1;
        let coordinates = Coordinates::new(source);
        if tree.root_node().has_error() {
            output.diagnostics.push(Diagnostic {
                path: path_text.clone(),
                status: "parse-error",
                range: None,
            });
            if output.diagnostics.len() >= 100 {
                output.stop("diagnostic-limit");
            }
        }
        let nodes = declarations(tree.root_node());
        let mut choices: BTreeMap<usize, (Node<'_>, Vec<SourceRange>)> = BTreeMap::new();
        match request.operation.as_str() {
            "outline" | "read-symbol" => {
                for &node in &nodes {
                    if request.operation == "outline"
                        || symbol(node, source) == *request.query.as_ref().unwrap()
                    {
                        choices.insert(node.start_byte(), (node, Vec::new()));
                    }
                }
            }
            _ => {
                for (start, end) in hits {
                    let mut candidates: Vec<_> = nodes
                        .iter()
                        .copied()
                        .filter(|&node| {
                            let wrapper = source_node(node, request.include_docs);
                            selected(node, &request.context)
                                && attached_start(wrapper, source, request.include_docs) <= start
                                && wrapper.end_byte() >= end
                                && start < wrapper.end_byte()
                        })
                        .collect();
                    candidates.sort_by_key(|node| node.end_byte() - node.start_byte());
                    let chosen = if request.context == "outer-function" {
                        candidates.last()
                    } else {
                        candidates.first()
                    };
                    let hit = coordinates.range(start, end);
                    if let Some(&node) = chosen {
                        choices
                            .entry(node.start_byte())
                            .or_insert_with(|| (node, Vec::new()))
                            .1
                            .push(hit);
                    } else {
                        output.diagnostics.push(Diagnostic {
                            path: path_text.clone(),
                            status: "no-enclosing-definition",
                            range: Some(hit),
                        });
                        if output.diagnostics.len() >= 100 {
                            output.stop("diagnostic-limit");
                            break;
                        }
                    }
                }
            }
        }
        let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
        for (_, (node, hits)) in choices {
            if output.results.len() == request.max_results {
                output.stop("result-limit");
                break;
            }
            let wrapper = source_node(node, request.include_docs);
            let start = attached_start(wrapper, source, request.include_docs);
            let end = wrapper.end_byte();
            let oversized = end - start > request.max_bytes;
            let outline = request.operation == "outline";
            let content = (!outline && !oversized).then(|| source[start..end].to_owned());
            let result = Definition {
                path: path_text.clone(),
                language: language_name.into(),
                symbol: symbol(node, source),
                kind: node.kind().into(),
                range: coordinates.range(start, end),
                definition_range: coordinates.range(node.start_byte(), node.end_byte()),
                source: content,
                signature: signature(node, source),
                parent_signature: request
                    .include_parent_signature
                    .then(|| parent_signature(node, source))
                    .flatten(),
                hits,
                file_hash: hash.clone(),
                parse_errors: tree.root_node().has_error(),
                complete: !outline && !oversized && !node.has_error(),
                status: if outline {
                    "outline"
                } else if oversized {
                    "size-limit"
                } else if node.has_error() {
                    "parse-error"
                } else {
                    "ok"
                },
            };
            // Count full serialized records, not just source, to bound hashes,
            // signatures and hit arrays as well. No partial source is returned.
            let size = serde_json::to_vec(&result)
                .map_err(|_| "cannot encode result")?
                .len();
            if output_bytes.saturating_add(size) > MAX_OUTPUT {
                output.stop("output-byte-limit");
                break;
            }
            output_bytes += size;
            output.results.push(result);
        }
        if output.truncated {
            break;
        }
    }
    if request.format == "json" {
        serde_json::to_string(&output).map_err(|_| "cannot encode results".into())
    } else {
        Ok(output.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_first_method_docs_and_header_are_separate() {
        let source = "class Auth:\n    # Method documentation.\n    @cached\n    def refresh(self):\n        return 'token'\n";
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let nodes = declarations(tree.root_node());
        assert_eq!(nodes.len(), 2);
        assert_eq!(signature(nodes[0], source), "class Auth:");
        let wrapper = source_node(nodes[1], true);
        let start = attached_start(wrapper, source, true);
        assert_eq!(&source[start..start + 23], "# Method documentation.");
        assert_eq!(
            parent_signature(nodes[1], source).as_deref(),
            Some("class Auth:")
        );
        assert_eq!(source_node(nodes[1], false).kind(), "function_definition");
    }

    #[test]
    fn assigned_arrow_keeps_its_binding_and_export() {
        let source = "export const refresh = async () => { return 'token'; };";
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let nodes = declarations(tree.root_node());
        assert_eq!(nodes.len(), 1);
        assert_eq!(symbol(nodes[0], source), "refresh");
        assert_eq!(&source[source_node(nodes[0], true).byte_range()], source);
    }

    #[test]
    fn coordinates_use_utf8_bytes_and_one_based_lines() {
        let source = "é\r\n日本語 token\n";
        let start = source.find("token").unwrap();
        let range = Coordinates::new(source).range(start, start + 5);
        assert_eq!((range.start_line, range.start_column_byte), (2, 10));
        assert_eq!(range.end_byte - range.start_byte, 5);
        let eof = Coordinates::new(source).range(source.len(), source.len());
        assert_eq!((eof.end_line, eof.end_column_byte), (3, 0));
    }
}
