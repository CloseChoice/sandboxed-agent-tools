mod retrieval;

wit_bindgen::generate!({ world: "code" });

struct Code;
impl Guest for Code {
    fn run(
        operation: String,
        query: Option<String>,
        paths: Option<Vec<String>>,
        mode: String,
        context: String,
        position: Option<u32>,
        include_docs: bool,
        include_parent_signature: bool,
        max_results: u32,
        max_bytes: u32,
        format: String,
        extract: String,
    ) -> Result<String, String> {
        retrieval::run(retrieval::Request {
            operation,
            query,
            paths: paths.unwrap_or_else(|| vec![".".into()]),
            mode,
            context,
            position: position.map(|n| n as usize),
            include_docs,
            include_parent_signature,
            max_results: max_results as usize,
            max_bytes: max_bytes as usize,
            format,
            extract,
        })
    }
}
export!(Code);
