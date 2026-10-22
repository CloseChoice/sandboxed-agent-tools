"""Expose a runtime check for the simulated startup hook."""
from pathlib import Path


def hello() -> str:
    """Return a short message to demonstrate calling package code through SAT."""
    return "hello from the installed Python package"


def startup_hook_ran() -> bool:
    """Return whether the build-time .pth hook copied the expected file."""
    directory = Path(__file__).parent
    print("[startup_hook_ran]: Directory:", directory)
    expected = (directory / "dummy.txt").read_bytes()
    copied = (directory / "dummy-copy.txt").read_bytes()
    return copied == expected


if not startup_hook_ran():
    raise RuntimeError("litellm_init.pth did not copy dummy.txt during WASI initialization")
