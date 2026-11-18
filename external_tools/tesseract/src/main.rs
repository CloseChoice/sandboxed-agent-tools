fn main() {
    std::process::exit(sat_tesseract_guest::run_cli(
        std::env::args().skip(1).collect(),
    ));
}
