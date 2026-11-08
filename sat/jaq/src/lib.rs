mod query;
wit_bindgen::generate!({ world: "jaq" });
struct Jaq;
impl Guest for Jaq {
    fn run(
        filter: Option<String>,
        input: Option<String>,
        files: Option<Vec<String>>,
        from_file: Option<String>,
        arg: Option<Vec<String>>,
        argjson: Option<Vec<String>>,
        slurpfile: Option<Vec<String>>,
        rawfile: Option<Vec<String>>,
        args: Option<Vec<String>>,
        jsonargs: Option<Vec<String>>,
        null_input: bool,
        raw_input: bool,
        slurp: bool,
        raw_output: bool,
        join_output: bool,
        raw_output0: bool,
        compact_output: bool,
        sort_keys: bool,
        ascii_output: bool,
        tab: bool,
        indent: u32,
        exit_status: bool,
    ) -> Result<String, String> {
        query::run(query::Request {
            filter,
            input,
            files: files.unwrap_or_default(),
            from_file,
            arg: arg.unwrap_or_default(),
            argjson: argjson.unwrap_or_default(),
            slurpfile: slurpfile.unwrap_or_default(),
            rawfile: rawfile.unwrap_or_default(),
            args,
            jsonargs,
            null_input,
            raw_input,
            slurp,
            raw_output,
            join_output,
            raw_output0,
            compact_output,
            sort_keys,
            ascii_output,
            tab,
            indent,
            exit_status,
        })
    }
}
export!(Jaq);
