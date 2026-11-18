This will be the project readme.

This outlines how I imagine the new and minimal architecture for SAT. 
Agents will then implement that.

## sandboxed-agent-tools

Sandbox your agent's tools. This enables agents to execute code that is not trusted with broad permissions.

```
sat fuzzy-search --pattern '<somepattern>' -- --read "$PWD/file1.md" --read "$PWD/file2.md" --read "$PWD/src"
```
allows the fuzzy-search tool to only read the specified files or directories, no more. Even if the fuzzy-search would try to send the file contents via http to an attacker the web assembly system inferface's security model will catch and block that. Registration validates the binary’s actual imports and exports. The host enforces filesystem and HTTP permissions at runtime; WIT alone cannot describe allowed paths or destinations.

### Installation
Install using the script:

```bash
todo: this needs to be written, but has lower priority currently
curl ... sat-install.sh [optional: provide SAT_HOME here]h | sh
```

Or build from source:
```bash
cargo commands, etc.
```

append to path:
```
# Add this line to your shell startup file (for example ~/.bashrc):
export PATH="${SAT_HOME:-$HOME/.sat}:$PATH"
source .bashrc
```

`SAT_HOME` defaults to `~/.sat`; the host binary lives directly in that directory. Registered tools live under `$SAT_HOME/tools/<toolname>/`, without another nested `.sat` directory.

### Cli Options
- sat help
- sat init (generates `$PWD/.sat/config.overrides.toml`, if this is present, fails. Existing file can be overwritten with sat init --force)
- sat list 
    list toolnames
- sat register <local-path-or-http(s)-url>
    register a tool from a complete local or remote `.tar` or `.zip` archive
- sat remove/unregister <toolname>
    delete the installed tool directory and its settings from `$SAT_HOME/config.toml` and the current project’s `$PWD/.sat/config.overrides.toml`; other projects are not scanned or modified
- sat <toolname>
    execute a tool
    Tools might need more parameters, the syntax works as
    sat <toolname> <toolarg1> <toolarg2> --<toolkey1> <toolvalue1> -- --read PATH --write PATH --allow-http HOSTNAME
- sat config [<toolname>]
    Show the current config of sat + tools, if toolname is provided, just show the config of that.
- `sat python -c`/`sat python <filename>.py`

all options allow `--help`

### Permission Model
Global settings live in `$SAT_HOME/config.toml`. Project overrides live in
`$PWD/.sat/config.overrides.toml`. Both files use `[tools.<toolname>]` tables
for tool-specific settings, with permission tables such as
`[tools.brave.http_permissions]` and `[tools.brave.file_permissions]`.

Tool archives supply suggested permissions in `defaults.toml`. Registration
prompts the user before saving them in project configuration.

There are no cross-tool defaults. Each configured tool must specify all four
filesystem lists (`read`, `write`, `deny-read`, `deny-write`), including explicit
empty lists, and exactly one HTTP list (`allow-http` or `deny-http`). An absent
tool configuration grants no filesystem or HTTP access. A present but incomplete
tool configuration is an error.

The current project's complete tool settings replace that tool's global settings.
CLI flags replace the corresponding configured list for that invocation; lists
are never appended or merged across sources. Repeating a flag supplies multiple
values for its replacement list. Lists without a CLI override retain their
configured values. Selecting an HTTP list on the CLI replaces the entire HTTP
mode and list, including an inherited opposite mode. Supplying both HTTP modes
on the CLI is an error.

A tool may have an optional `alias = "..."` directly under `[tools.<toolname>]`.
The alias is an alternative command name, not a guest filesystem path mapping.
Aliases must be unique across effective tool settings and must not collide with
registered tool names or SAT's built-in commands. Reject conflicting aliases.

#### HTTP

HTTP permissions use either an allow-list or a deny-list of hostnames. Only one
list may be set for a policy. Setting both is a configuration error, even if
one or both lists are empty. A present tool settings block must specify one list.

| Configuration | Behavior |
| --- | --- |
| No tool configuration exists | Deny all HTTP requests |
| Tool configuration exists but neither list is set | Reject the configuration |
| `allow-http = []` | Deny all HTTP requests |
| `deny-http = []` | Allow all HTTP(S) requests |
| Populated allow-list | Allow only matching hostnames |
| Populated deny-list | Allow all hostnames except matching ones |
| Both lists are set | Reject the configuration |

Matching uses the parsed URL hostname, not a substring of the URL:

- `api.brave.com` matches only that hostname.
- `*.brave.com` matches subdomains at any depth, including `api.brave.com` and
  `foo.api.brave.com`, but does not match the root hostname `brave.com`.
- To include the root and all subdomains, list both `brave.com` and `*.brave.com`.
- Neither rule matches `evilbrave.com` or `brave.com.evil.com`.

These are hostname rules, not origin rules: they cover HTTP and HTTPS on any
port. All methods, paths, and query strings are permitted for an allowed
hostname. Method and path restrictions are outside this MVP.

The host checks every outgoing request before sending it. If redirects are
followed, each redirect destination must pass the same policy check before
another request is sent. WIT import validation cannot replace these runtime
checks because an HTTP interface does not specify the allowed destinations.

#### File Permissions
Use repeatable `--read PATH` and `--write PATH` flags for files or directories.
Write includes read access. Grant paths must exist. Directory grants are
recursive, and writable directories permit creating new files inside them.

Use a quoted filename pattern such as `--read "$PWD/src/*.py"` to grant individual
regular files. The host expands `*` and `?` in the last path component once per
invocation. Write grants and configured grants use literal paths.

Set `deny-read` and `deny-write` in the tool configuration to exclude paths
from grants. Per-invocation flags preserve those exclusions. Denials
always win and never grant access: deny-read blocks reads even through a write
grant, while deny-write blocks writes. To protect a file from both operations,
include it in both deny lists.

Deny patterns use glob matching against paths relative to each granted directory
(or the basename for an individual file grant). `*` matches within one path
segment, `?` matches one character within a segment, and `**` matches across
segments. `**/.env` matches `.env` at any depth, including the grant root.
Use `/` as the pattern separator on every platform. The host must enforce exclusions on
resolved targets as well as requested paths so symlinks and alternate paths
cannot bypass them.

Grant paths must be absolute or start with `$PWD` or `$SAT_HOME`; bare relative
paths are rejected. In configuration, SAT resolves only those two leading tokens:
`$PWD` is the invocation working directory and `$SAT_HOME` is the resolved SAT
home directory. This is path substitution, not general environment expansion or
guest environment inheritance. A shell may expand variables in CLI arguments,
for example `--read "$HOME/data"`, which supplies an absolute path. Deny globs
remain relative patterns as described above.

File denials apply to all filesystem operations, not only opening file contents.
Read-denied entries are omitted from directory listings; write denials also cover
creation, deletion, and rename. Rename and link operations must not bypass denied
paths or expose denied contents under another name. If an operation cannot be
enforced safely, reject it.

### Building your own extensions
#### wit files
Here we explain how we expect wit files to look like.
#### tool metadata

#### tool interface

Adopt the SAT runner from branch `feat/sat-component-mvp`: its component interface, argument types, generated
help, result formatting, and error exit behavior. Resource limits are deferred;
the new implementation currently imposes no memory, fuel, or execution-time limits.


#### example config files
The default config file lives in SAT_HOME/config.toml
but can also be overwritten in the current directory under .sat/config.overrides.toml

config.overrides.toml

```toml
[tools.brave]
alias = "web-search"

[tools.brave.file_permissions]
read = []
write = []
deny-read = []
deny-write = []

[tools.brave.http_permissions]
allow-http = ["api.brave.com", "*.brave.com"]
# deny-http = ["example.com", "*.example.com"]
```

tool metadata (should go into `tool.toml`)
```toml
name = "python"
version = "0.1.0"
description = "Run Python code inside a WASI sandbox."
# this entrypoint must be provided by the wit-world
entrypoint = "run"
# allow only one of the options below
exactly_one_of = ["code", "script"]

[[arguments]]
name = "script"
help = "Python file to execute (requires a read grant)."

[options.code]
short = "c"
help = "Python code to execute."

[[examples]]
command = 'sat python -c "print(1 + 2)"'

[[examples]]
command = 'sat python "$PWD/script.py" -- --read "$PWD/script.py"'
```

### Registering Tools
when calling
```bash
sat register <local-path-or-http(s)-url>
```
For a local path, verify that the archive exists and is a regular file. For an
HTTP(S) URL, download the complete archive to temporary storage and check that
the download succeeded before processing it. Process the archive in an isolated
temporary directory; clean it up after success, failure, or cancellation.

Archives may be `.tar` or `.zip` and must contain these three regular files at
root level: `tool.wasm`, `tool.toml`, and `defaults.toml`. Registration copies
only these three files into `$SAT_HOME/tools/<toolname>/`; other entries are
ignored, never extracted or copied. Reject missing, duplicate, or linked required
entries, and never extract archive paths outside the tool directory.

If the tool already exists, fail with an error explaining that `--force` or `-f`
is required to overwrite it. Forced registration repeats the permission prompts
and replaces both the installed files and the corresponding tool section in
`$PWD/.sat/config.overrides.toml`, preserving other tools' sections.

Registration interactively asks the user which permission defaults to save.
Before prompting, show all six suggested entries from the archive's
`defaults.toml`: `read`, `write`, `deny-read`, `deny-write`, `allow-http`, and `deny-http`. Show empty
lists explicitly and distinguish an unset HTTP list from `[]`, since an empty
deny-list allows all HTTP(S).

Then prompt for each entry separately, in this order:

1. `read`
2. `write`
3. `deny-read`
4. `deny-write`
5. `allow-http`
6. `deny-http`

At each prompt, display the suggested value. The user can type `Y` to accept it
or enter a replacement TOML array of strings, such as `["$PWD/src", "$PWD/README.md"]`.
Entering `[]` explicitly selects an empty list. For HTTP entries, the user can
also enter `unset` to leave that list unspecified. Blank input does not accept
permissions; ask again. Invalid input is explained and the same entry is prompted
again.

HTTP allow and deny remain mutually exclusive. If both are selected, explain
the conflict and let the user revise the HTTP entries before saving. The saved settings must contain exactly one HTTP list; use `allow-http = []`
to deny all HTTP. Missing filesystem suggestions are displayed as `[]`, and all
four filesystem lists must be saved explicitly.
Cancelling or failing registration leaves existing installed files and
configuration untouched and does not activate suggested permissions. Validate
the archive, component, metadata, and chosen settings before publishing changes.

After all entries are valid, save the user's chosen values as a tool-specific
entry in `$PWD/.sat/config.overrides.toml`. Create the project `.sat` directory
and configuration file if absent; preserve unrelated settings.
For example, registering a tool named `brave` adds settings under
`[tools.brave]`:

```toml
[tools.brave.file_permissions]
read = []
write = []
deny-read = []
deny-write = []

[tools.brave.http_permissions]
allow-http = ["api.search.brave.com"]
# Alternatively use deny-http; never set both lists.
```
The archive's `defaults.toml` uses unprefixed permission tables without a tool
name wrapper, for example:

```toml
[file_permissions]
read = []
write = []
deny-read = ["**/.env"]
deny-write = ["**/.env"]

[http_permissions]
allow-http = ["api.search.brave.com"]
```

The archive's `defaults.toml` supplies suggestions; the values written to
`$PWD/.sat/config.overrides.toml` are those explicitly accepted or entered by the
user. The original defaults file is also copied to
`$SAT_HOME/tools/<toolname>/defaults.toml` for reference, but it is not loaded
during execution. The resulting settings in `config.overrides.toml` are used
as the project's tool overrides.

#### Test concept
- how to test this properly? We could add some things so that this might work for specific things, e.g. create some dummy wasm binaries (malicious) and test that these are rejected, also create dummy toml files and make sure that these are actually working/rejected

### Deferred functionality

Search, inspection, compilation caching, scoped credential injection, and archive
hashes are deferred. Guest filesystem path aliases are also deferred; the alias
supported above is a unique tool command alias. Clocks and randomness use WASI's
normal services; raw sockets and inherited host environment remain unavailable.
