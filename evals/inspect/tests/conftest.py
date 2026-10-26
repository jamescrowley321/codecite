import sys
from pathlib import Path

# task.py and report.py are scripts, not a package: import them by name.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
