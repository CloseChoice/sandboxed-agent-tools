// The upstream CLI, evaluator, format handling, and help text are used directly.
include!("../upstream/main.rs");

wit_bindgen::generate!({ world: "jaq" });
struct Jaq;
impl Guest for Jaq {
    fn run(args: Vec<String>) -> i32 {
        drop(args); // The host supplies these same arguments through WASI argv.
        if let Ok(directory) = std::env::var("PWD") {
            if std::env::set_current_dir(directory).is_err() {
                eprintln!("Error: cannot select the granted working directory");
                return 2;
            }
        }
        let status = main();
        // ExitCode's numeric accessor is unstable; equality is portable.
        (0..=255).find(|code| status == ExitCode::from(*code as u8)).unwrap_or(1)
    }
}
export!(Jaq);
