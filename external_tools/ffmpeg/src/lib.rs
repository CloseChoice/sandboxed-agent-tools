use std::ffi::{CString, c_char, c_int};

wit_bindgen::generate!({ world: "media-cli" });

unsafe extern "C" {
    fn sat_media_main(argc: c_int, argv: *mut *mut c_char) -> c_int;
}

struct Media;
impl Guest for Media {
    fn run(args: Vec<String>) -> i32 {
        let program = if cfg!(feature = "probe") { "ffprobe" } else { "ffmpeg" };
        let strings = std::iter::once(program.to_string()).chain(args)
            .map(CString::new).collect::<Result<Vec<_>, _>>();
        let Ok(strings) = strings else {
            eprintln!("{program}: arguments cannot contain NUL bytes");
            return 1;
        };
        let mut argv = strings.iter().map(|s| s.as_ptr().cast_mut()).collect::<Vec<_>>();
        let argc = argv.len();
        argv.push(std::ptr::null_mut());
        // A fresh component instance owns one upstream CLI invocation. FFmpeg's
        // own parser, processing, stdout/stderr and exit handling remain intact.
        unsafe { sat_media_main(argc as c_int, argv.as_mut_ptr()) }
    }
}
export!(Media);
