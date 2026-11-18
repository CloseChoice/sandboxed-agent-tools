"""Build and verify ripgrep using the current SAT archive interface."""
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from test_imported_tools import check
if __name__ == "__main__":
    check("ripgrep", build="--skip-build" not in sys.argv)
