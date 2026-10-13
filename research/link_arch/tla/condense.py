"""Condense a TLC counterexample to one line a step: the action, the clock, what changed.

python research/link_arch/tla/condense.py research/link_arch/tla/out/ImplicitTurn_requests.txt
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

STATE = re.compile(r"^State (\d+): <(\w+)(?:\(\"?(\w+)\"?\))?")
VAR = re.compile(r"^/\\ (\w+) = (.*)$")
NONE = "99999"


def states(text: str) -> list[tuple[str, dict[str, str]]]:
    out: list[tuple[str, dict[str, str]]] = []
    cur: dict[str, str] | None = None
    last_var = ""
    for line in text.splitlines():
        m = STATE.match(line)
        if m:
            action = m.group(2) + (f"({m.group(3)})" if m.group(3) else "")
            cur = {}
            out.append((action, cur))
            continue
        if cur is None:
            continue
        v = VAR.match(line)
        if v:
            last_var = v.group(1)
            cur[last_var] = v.group(2)
        elif line.startswith("  ") and last_var:
            cur[last_var] += " " + line.strip()
        elif not line.strip():
            last_var = ""
    return out


def tidy(value: str) -> str:
    value = re.sub(
        r"\[start \|-> 99999, end \|-> 99999, kind \|-> \"none\"(, sensed \|-> \w+)?\]", "-", value
    )
    value = re.sub(r"\[kind \|-> \"none\", at \|-> 99999\]", "-", value)
    value = re.sub(r"\[at \|-> 99999, kind \|-> \"none\"\]", "-", value)
    value = re.sub(r"\[start \|-> 99999, kind \|-> \"none\"\]", "-", value)
    value = value.replace(NONE, "-").replace(" |-> ", ":").replace('"', "")
    return re.sub(r"\s+", " ", value)


def condense(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="replace")
    head = next((ln for ln in text.splitlines() if "violated" in ln or "No error" in ln), "")
    seq = states(text)
    lines = [f"{path.stem}: {head.replace('Error: ', '')}"]
    prev: dict[str, str] = {}
    for i, (action, vars_) in enumerate(seq, 1):
        changed = {k: v for k, v in vars_.items() if prev.get(k) != v and k != "now"}
        if action == "Tick" and set(changed) <= {"now"}:
            prev = vars_
            continue
        now = vars_.get("now", "?")
        body = "; ".join(f"{k}={tidy(v)}" for k, v in changed.items()) if i > 1 else "initial"
        lines.append(f"  {i:>2} t={now:>3} {action:<22} {body}")
        prev = vars_
    return "\n".join(lines)


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        print(condense(Path(arg)))
        print()
