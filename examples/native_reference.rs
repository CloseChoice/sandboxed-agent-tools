//! Native upstream-matcher results for checking the Wasm scalar backend.
use neo_frizbee::{Config, Matcher, Scoring, UnicodeMatching};

fn main() {
    let paths = [
        "src/main.rs", "docs/readme.md", "schema.rs", "src/schema.rs",
        "📁/café.rs", "src/Main.rs", "src/main.rs", "", "src\\main.rs",
    ];
    let queries = ["main", "smrs", "schexma", "café", "caf", "rs", "Main", "zzzzzz"];
    print!("[");
    let mut first = true;
    for query in queries {
        for typos in 0..=2 {
            if !first { print!(","); }
            first = false;
            let uppercase = query.chars().any(char::is_uppercase);
            let config = Config {
                max_typos: Some(typos),
                unicode: UnicodeMatching::Always,
                scoring: Scoring {
                    capitalization_bonus: if uppercase { 8 } else { 0 },
                    matching_case_bonus: if uppercase { 4 } else { 0 },
                    ..Scoring::default()
                },
                ..Config::default()
            };
            let matches = Matcher::new(query, &config).match_list(&paths);
            print!("[");
            for (i, hit) in matches.iter().enumerate() {
                if i > 0 { print!(","); }
                print!("[{},{}]", hit.index, hit.score);
            }
            print!("]");
        }
    }
    println!("]");
}
