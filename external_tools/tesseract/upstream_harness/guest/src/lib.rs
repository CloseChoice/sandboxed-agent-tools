use std::ffi::{CString, c_char};
wit_bindgen::generate!({path:"wit",world:"upstream-test"});
unsafe extern "C" {
    fn sat_upstream_tests(argc: i32, argv: *mut *mut c_char) -> i32;
}
struct Tests;
impl Guest for Tests {
    fn run(args: Vec<String>) -> i32 {
        let args: Vec<_> = std::iter::once("upstream-test".to_string())
            .chain(args)
            .map(|s| CString::new(s).unwrap())
            .collect();
        let mut pointers: Vec<_> = args.iter().map(|s| s.as_ptr() as *mut c_char).collect();
        pointers.push(std::ptr::null_mut());
        unsafe { sat_upstream_tests(args.len() as i32, pointers.as_mut_ptr()) }
    }
}
export!(Tests);
