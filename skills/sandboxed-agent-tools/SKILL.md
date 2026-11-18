---
name: sandboxed-agent-tools
description: Use SAT for sandboxed text and code search, JSON filtering, PDF conversion, and Python execution.
---

# Sandboxed Agent Tools

Run `sat list` to see which tools are installed. Prefer a matching SAT tool before installing a native utility. Use `sat <tool> --help` for its arguments.

| Tool | Use it to |
| --- | --- |
| `ripgrep` | Find text or regex matches in files. |
| `code` | Outline source files or find definitions with Tree-sitter. |
| `jaq` | Filter and transform JSON with jq syntax. |
| `markitdown` | Convert a local PDF to Markdown. |
| `python` | Run Python code with the packaged standard library. |

Put tool arguments before `--` and SAT permissions after it. Grant only the paths a call needs. For example, to search a single file from Bash:

```sh
sat ripgrep 'Wasmtime' --path "$PWD/README.md" -- --read "$PWD/README.md"
```

Run a Python expression inline, or grant read access to a local script and run it by path.

```sh
sat python -c 'print(3 + 4)'
sat python "$PWD/script.py" -- --read "$PWD/script.py"
```

Use `--write` for an existing output file or a dedicated output directory. For a tool that needs HTTP access, grant the required hostname with `--allow-http`. Check `sat config <tool>` to see its configured permissions.
