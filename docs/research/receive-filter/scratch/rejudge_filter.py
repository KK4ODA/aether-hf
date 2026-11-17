"""H1's filter_warnings, read again over every run of a harness out dir."""
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(sys.argv[2]) / "tools"))
import session_matrix as sm  # noqa: E402

warned = 0
runs = sorted(p for p in Path(sys.argv[1]).iterdir() if (p / "scenario.toml").exists())
for run_dir in runs:
    run = sm.Run(tomllib.loads((run_dir / "scenario.toml").read_text()), run_dir, run_dir / "channel.progress")
    found = sm.filter_warnings(run)
    warned += bool(found)
    print(f"{run_dir.name:44s} {'; '.join(found) or '-'}")
print(f"{warned} of {len(runs)} runs with a warning on an uncut radio")
