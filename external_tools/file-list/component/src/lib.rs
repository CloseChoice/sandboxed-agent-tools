mod bindings {
    use super::FileList;
    #[cfg(not(feature = "sat"))]
    wit_bindgen::generate!({ path: "../wit", world: "file-list", generate_all });
    #[cfg(feature = "sat")]
    wit_bindgen::generate!({ path: "../sat-wit", world: "file-list" });
    export!(FileList with_types_in self);
}

struct FileList;

#[cfg(not(feature = "sat"))]
impl bindings::Guest for FileList {
    fn run(file_name: String) -> Result<String, String> {
        #[cfg(feature = "security-probe")]
        probe::check()?;
        listing("/listing")
            .map_err(|_| "could not list the granted directory".into())
            .and_then(|mut output| {
                let text = bindings::read_file()?;
                output.push_str(&format!("\ncontent of file {file_name}:\n{text}"));
                Ok(output)
            })
    }
}

#[cfg(feature = "sat")]
impl bindings::Guest for FileList {
    fn run(directory: String, file: String) -> Result<String, String> {
        let mut output = listing(&directory).map_err(|_| "could not list the granted directory")?;
        let text = std::fs::read_to_string(&file).map_err(|_| "could not read the granted file")?;
        let name = std::path::Path::new(&file).file_name().unwrap_or_default().to_string_lossy();
        output.push_str(&format!("\ncontent of file {name}:\n{text}"));
        Ok(output)
    }
}

fn listing(directory: &str) -> std::io::Result<String> {
    let mut entries = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
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
