mod bindings {
    use super::FileList;
    wit_bindgen::generate!({ path: "../wit", world: "file-list", generate_all });
    export!(FileList with_types_in self);
}

struct FileList;

impl bindings::Guest for FileList {
    fn run(file_name: String) -> Result<String, String> {
        #[cfg(feature = "security-probe")]
        probe::check()?;
        listing()
            .map_err(|_| "could not list the granted directory".into())
            .and_then(|mut output| {
                let text = bindings::read_file()?;
                output.push_str(&format!("\ncontent of file {file_name}:\n{text}"));
                Ok(output)
            })
    }
}

fn listing() -> std::io::Result<String> {
    let mut entries = std::fs::read_dir("/listing")?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut output = String::from("type        bytes  modified (Unix seconds)  name\n");
    for entry in entries {
        let meta = std::fs::symlink_metadata(entry.path())?;
        let kind = if meta.is_dir() {
            "dir"
        } else if meta.is_symlink() {
            "link"
        } else {
            "file"
        };
        let modified = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_secs().to_string())
            .unwrap_or_else(|_| "before epoch".into());
        output.push_str(&format!(
            "{kind:<5} {:>11}  {modified:>23}  {}\n",
            meta.len(),
            entry.file_name().to_string_lossy()
        ));
    }
    Ok(output)
}

#[cfg(feature = "security-probe")]
mod probe;
