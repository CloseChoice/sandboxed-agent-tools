# Upstream source

Source: jaq 3.1.2 crate from crates.io, commit
f7fea9219d8d5364c91c6a0278765eaa0e6fa95e.
https://github.com/01mf02/jaq/tree/f7fea9219d8d5364c91c6a0278765eaa0e6fa95e/jaq/src

The CLI parser, evaluator, data formats, help, and tests are upstream source.
WASI adapters:
- The optional native mimalloc allocator is disabled for WASI.
- Error::Halt returns its u8 status instead of calling std::process::exit, so the
  status travels back through WIT.
- In-place replacement skips std::fs::set_permissions on WASI, which has no
  chmod operation. Host filesystem modes on replacement files follow the host's
  file-creation defaults. Native builds retain upstream permission restoration.
The WASI build disables rustyline file-history (fd-lock does not support WASI).
It uses rustyline's upstream wasm terminal backend; no host terminal or history
files are inherited. The native parity binary retains file-history support.
