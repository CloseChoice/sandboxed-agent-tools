use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=dashboard/assets");
    fn collect(root: &Path, path: &Path, output: &mut String) {
        let mut entries: Vec<_> = fs::read_dir(path)
            .expect("Build the dashboard first: cd sat/dashboard && npm ci && npm run build")
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                collect(root, &entry, output);
            } else {
                let route = format!(
                    "/{}",
                    entry
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                );
                output.push_str(&format!(
                    "({route:?}, include_bytes!({:?})),\n",
                    entry.canonicalize().unwrap()
                ));
            }
        }
    }
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("dashboard/assets");
    let mut assets = String::from("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    collect(&root, &root, &mut assets);
    assets.push_str("];\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("dashboard_assets.rs"),
        assets,
    )
    .unwrap();
}
