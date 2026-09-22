// Built only by test.py; the normal component contains none of these probes.
pub fn check() -> Result<(), String> {
    let preopens = super::bindings::wasi::filesystem::preopens::get_directories();
    if preopens.len() != 1 || preopens[0].1 != "/listing" {
        return Err("unexpected directory grants".into());
    }
    for path in ["/listing/../secret.txt", "/secret.txt", "/listing/escape"] {
        if std::fs::read(path).is_ok() {
            return Err(format!("read outside grant: {path}"));
        }
    }
    for path in ["/listing/hello.txt", "/listing/new.txt"] {
        if std::fs::write(path, "forbidden").is_ok() {
            return Err(format!("write succeeded: {path}"));
        }
    }
    if std::env::vars_os().next().is_some() {
        return Err("host environment inherited".into());
    }
    match std::net::TcpStream::connect("127.0.0.1:9") {
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
        _ => return Err("socket access was not denied by policy".into()),
    }
    Ok(())
}
