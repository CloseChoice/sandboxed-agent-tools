wit_bindgen::generate!({ path: "wit", world: "uutils" });

struct Uutils;
impl Guest for Uutils {
    fn run(args: Vec<String>, _input: Option<String>) -> i32 {
        let mut args = args.into_iter();
        let Some(command) = args.next() else {
            eprintln!("Expected a utility name");
            return 2;
        };
        uucore::set_utility_is_second_arg();
        let _ = uucore::locale::setup_localization(&command);
        let argv = std::iter::once(command.clone())
            .chain(args)
            .map(std::ffi::OsString::from);
        match command.as_str() {
            "head" => uu_head::uumain(argv),
            "tail" => uu_tail::uumain(argv),
            "cat" => uu_cat::uumain(argv),
            "wc" => uu_wc::uumain(argv),
            "cut" => uu_cut::uumain(argv),
            "sort" => uu_sort::uumain(argv),
            "uniq" => uu_uniq::uumain(argv),
            "nl" => uu_nl::uumain(argv),
            "ls" => uu_ls::uumain(argv),
            "basename" => uu_basename::uumain(argv),
            "dirname" => uu_dirname::uumain(argv),
            _ => {
                eprintln!(
                    "Unsupported utility {command:?}. Available: head, tail, cat, wc, cut, sort, uniq, nl, ls, basename, dirname. top is not a coreutils command and requires host process access."
                );
                2
            }
        }
    }
}
export!(Uutils);
