# List a directory, then read one selected file

A standalone P2 component and a small Wasmtime host. Select the directory and
text file in [`standalone.toml`](standalone.toml):

```toml
component = "target/wasm32-wasip2/debug/file_list_component.wasm"

[permissions]
directory = "sample/directory"
file = "sample/message.txt"
```

All relative paths resolve from the TOML file's directory, independently of the
current working directory. Absolute paths also work. Both grants are read-only.
This manual example treats the supplied TOML as operator-selected configuration;
SAT must check tool-declared requirements against approved grants before execution.

Start with [`wit/world.wit`](wit/world.wit):

```wit
world file-list {
    import wasi:filesystem/preopens@0.2.12;
    import read-file: func() -> result<string, string>;
    export run: func(file-name: string) -> result<string, string>;
}
```

The host opens exactly the selected file read-only and exposes `read-file()`.
That import accepts **no path**, so the component cannot use it to read a sibling
file. This is a small custom WIT capability, not a standard WASI single-file preopen.
The host also preopens the selected directory read-only as `/listing` using WASI.
That directory grant includes its descendants; it is not restricted to metadata.
Choose a directory separate from the input file's parent if you want the file's
siblings to remain inaccessible.

The component implements a portable `ll`-style listing: sorted names, entry type,
byte size, and modification time in Unix seconds. It does not invoke a shell or
the host's `ls`. It then appends `content of file <name>:` and the UTF-8 file text.
Unlike Unix `ls -l`, it does not show owner/group or Unix permission bits.

Results return through WIT as a string. Only the host prints them. Neither host
environment nor guest stdio is inherited; raw sockets are disabled, and no HTTP
implementation is linked. The host resolves `run` at runtime and checks its
signature; it does not generate bindings from this component's WIT.

From this directory:

```sh
cargo build --locked -p file-list-component --target wasm32-wasip2
cargo run --locked -p file-list-host -- standalone.toml
```

The listing appears first, followed by:

```text
content of file message.txt:
This text comes from the separately granted file.
```

The filesystem permissions are in [`host/src/main.rs`](host/src/main.rs).
The listing and append logic are in [`component/src/lib.rs`](component/src/lib.rs).
The vendored standard WIT dependencies live in `wit/deps`.

Run `python3 test.py` for offline checks of the real component: read-only access,
parent/symlink escapes, environment isolation, network denial, and final output.
The script temporarily builds an additional security-probe feature, then restores
the normal component. This minimal example does not add execution or output-size
limits and is intended for the supplied local component.
