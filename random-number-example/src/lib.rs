wit_bindgen::generate!({ world: "random-number", generate_all });

struct RandomNumber;

impl Guest for RandomNumber {
    fn run() -> u64 {
        wasi::random::random::get_random_u64()
    }
}

export!(RandomNumber);
