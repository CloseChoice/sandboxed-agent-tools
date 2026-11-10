"""Explicit Pi tool allowlists shared by the local and container runners."""

NATIVE_TOOLS = ("read", "bash", "edit", "write", "grep", "find", "ls")
SAT_TOOLS = ("sat_search", "sat_outline", "sat_read_symbol", "sat_ripgrep")


def active_tool_names(variant):
    if variant not in ("baseline", "sat", "rtk", "sat_rtk"):
        raise ValueError(f"Unknown experiment variant: {variant}")
    return list(NATIVE_TOOLS + (SAT_TOOLS if variant in ("sat", "sat_rtk") else ()))
