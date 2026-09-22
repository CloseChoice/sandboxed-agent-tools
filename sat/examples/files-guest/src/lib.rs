wit_bindgen::generate!({ world: "files-demo" });
struct Files;
impl Guest for Files {
    fn run(operation: String, path: String, text: String) -> Result<String, String> {
        match operation.as_str() {
            "socket" => Ok(if std::net::TcpStream::connect(&path).is_err() {
                "denied"
            } else {
                "connected"
            }
            .into()),
            "read" => documents::read(&path),
            "list" => documents::list(&path).map(|names| names.join("\n")),
            "write-dir" => documents::write(&path, &text).map(|_| "written".into()),
            "input" => input::read(),
            "output" => output::write(&text).map(|_| "written".into()),
            "env" => {
                Ok(std::env::var("SAT_TEST_SECRET").unwrap_or_else(|_| "not inherited".into()))
            }
            _ => Err("unknown operation".into()),
        }
    }
}
export!(Files);
