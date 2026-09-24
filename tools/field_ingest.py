"""Fold contributed sessions into the field log (roadmap P6-7).

    python tools/field_ingest.py <sidecar.json> [...] [--class awgn|good|moderate|poor]
                                [--band 40m] [--note "..."] [--log field/LOG.md]
                                [--paths field/paths.csv] [--ladders field/ladders.csv]

A volunteer's Test session (``docs/user/field-test.md`` §3a) arrives as a sidecar attached
to an issue. This reads each one and appends three things: a row to ``field/LOG.md`` — the
session as the log's table has it; a row per session to ``field/paths.csv`` — the path, the
probe, both transfers and the ladder in one cell; and a row per rung to ``field/ladders.csv``
— mode, frames, decoded, the SNR the other station measured — which is the material the
per-class penalties are fitted from. A sidecar already in the log is skipped, so running it
again over the same files changes nothing.

Two things the sidecar cannot know come from the command line: the channel class, which is
the operator's judgement (§4 of the guide), and the band when no rig control recorded the
frequency. A plain session (no Test session) is folded in too, from its frames and counters.
"""

from __future__ import annotations

import argparse
import csv
import json
import statistics
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FORMAT = "aether-hf-session/4"
"""The sidecar format this tool writes rows from: its mode numbers are rungs of the air's
ladder (ADR-0015). Older sidecars are read onto it by :func:`sidecar_rung`:
``aether-hf-session/3`` (the fast kinds, ADR-0014) numbered the 500 Hz ladder before its
middle kinds, ``aether-hf-session/2`` (the tone floor, ADR-0013) both ladders before their
middle rungs, and ``aether-hf-session/1`` — every sidecar before the tone floor — the OFDM
modes."""
LEGACY_FORMAT = "aether-hf-session/1"
FLOOR_FORMAT = "aether-hf-session/2"
FAST_FORMAT = "aether-hf-session/3"
FORMATS = (LEGACY_FORMAT, FLOOR_FORMAT, FAST_FORMAT, FORMAT)


def sidecar_rung(document: dict[str, object], mode: int) -> int | None:
    """The rung of the ladder a sidecar's mode number names. A ``/1`` sidecar numbered the
    OFDM modes: on the 500 Hz air modes 0 and 1 were the OFDM floor ADR-0013 retired, which
    is on no rung (``None``), and on the 2 300 Hz air every mode sat two rungs up on the
    ``/2`` ladder, above the tone floor's two. From there each air's rungs from 2 up moved
    once more: four on the 2 300 Hz air in ``/3``, above the fast kinds (ADR-0014), two on
    the 500 Hz air in ``/4``, above its middle kinds (ADR-0015)."""
    fmt = document.get("format")
    if fmt not in (LEGACY_FORMAT, FLOOR_FORMAT, FAST_FORMAT):
        return mode
    session = document.get("session") or {}
    bandwidth = session.get("bandwidth_hz") if isinstance(session, dict) else None
    if bandwidth == 500:
        if fmt == LEGACY_FORMAT and mode < 2:
            return None
        return mode + 2 if mode >= 2 else mode
    if fmt == FAST_FORMAT:
        return mode
    if fmt == LEGACY_FORMAT:
        mode += 2
    return mode + 4 if mode >= 2 else mode


BANDS: list[tuple[float, float, str]] = [
    (1.8e6, 2.0e6, "160 m"),
    (3.5e6, 4.0e6, "80 m"),
    (5.25e6, 5.45e6, "60 m"),
    (7.0e6, 7.3e6, "40 m"),
    (10.1e6, 10.15e6, "30 m"),
    (14.0e6, 14.35e6, "20 m"),
    (18.068e6, 18.168e6, "17 m"),
    (21.0e6, 21.45e6, "15 m"),
    (24.89e6, 24.99e6, "12 m"),
    (28.0e6, 29.7e6, "10 m"),
]


def band_of(frequency_hz: float | None) -> str | None:
    """The amateur band a frequency is in, as the log names it."""
    if frequency_hz is None:
        return None
    for low, high, name in BANDS:
        if low <= frequency_hz <= high:
            return name
    return f"{frequency_hz / 1e6:.3f} MHz"


@dataclass
class Rung:
    mode: int
    frames: int
    decoded: int
    snr_db: float | None


@dataclass
class Session:
    """What the log wants to know about one session, whichever kind it was."""

    recording: str
    date: str
    callsign: str
    remote: str
    bandwidth_hz: int | None
    frequency_hz: float | None
    my_grid: str
    their_grid: str
    km: float | None
    rig: str
    power_w: float | None
    antenna: str
    seconds: float
    frames: int
    decoded: int
    snr_db: float | None
    goodput_bps: float | None
    outcome: str
    probe_there_db: float | None = None
    probe_here_db: float | None = None
    message_bps: float | None = None
    file_bps: float | None = None
    rungs: list[Rung] = field(default_factory=list)
    notes: str = ""


def _number(value: object) -> float | None:
    return float(value) if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def summarise(document: dict[str, object], recording: str) -> Session:
    """The session a sidecar describes."""
    if document.get("format") not in FORMATS:
        raise ValueError(f"{recording}: not a session sidecar ({document.get('format')})")
    session = document.get("session") or {}
    assert isinstance(session, dict)
    operator = session.get("operator") or {}
    assert isinstance(operator, dict)
    test = session.get("test") or {}
    assert isinstance(test, dict)
    frames = document.get("frames") or []
    assert isinstance(frames, list)
    audio = document.get("audio") or {}
    assert isinstance(audio, dict)
    counters = document.get("counters") or {}
    assert isinstance(counters, dict)

    snrs = [s for f in frames if isinstance(f, dict) and (s := _number(f.get("snr_3k_db")))]
    data = [f for f in frames if isinstance(f, dict) and f.get("kind") == "data"]
    seconds = _number(audio.get("seconds")) or 0.0
    delivered = _number(counters.get("bytes_delivered")) or 0.0
    goodput = 8.0 * delivered / seconds if seconds > 0 and delivered > 0 else None

    path = test.get("path") or {}
    assert isinstance(path, dict)
    probe = test.get("probe") or {}
    assert isinstance(probe, dict)
    message = test.get("message") or {}
    file_ = test.get("file") or {}
    assert isinstance(message, dict) and isinstance(file_, dict)
    rungs = [
        Rung(rung, int(r["frames"]), int(r["decoded"]), _number(r.get("snr_db")))
        for r in test.get("ladder") or []
        if isinstance(r, dict) and (rung := sidecar_rung(document, int(r["mode"]))) is not None
    ]
    if test:
        # the transfers' goodput is the honest number: a Test session's audio also holds
        # the probe and the ladder, which no goodput should be charged for
        goodput = _number(file_.get("bps")) or _number(message.get("bps")) or goodput
        if snrs == [] and rungs:
            snrs = [r.snr_db for r in rungs if r.snr_db is not None]
    started = str(test.get("started") or document.get("started") or "")
    return Session(
        recording=recording,
        date=started[:10] or "?",
        callsign=str(session.get("callsign") or "?"),
        remote=str(test.get("remote") or session.get("remote") or "?"),
        bandwidth_hz=int(v) if (v := _number(session.get("bandwidth_hz"))) is not None else None,
        frequency_hz=_number(session.get("frequency_hz")),
        my_grid=str(path.get("my_grid") or operator.get("grid") or ""),
        their_grid=str(path.get("their_grid") or ""),
        km=_number(path.get("km")),
        rig=str(operator.get("rig") or ""),
        power_w=_number(operator.get("power_w")),
        antenna=str(operator.get("antenna") or ""),
        seconds=seconds,
        frames=len(data) if data else len(frames),
        decoded=sum(1 for f in (data or frames) if isinstance(f, dict) and f.get("decoded")),
        snr_db=statistics.mean(snrs) if snrs else None,
        goodput_bps=goodput,
        outcome=str(test.get("outcome") or ("session" if not test else "running")),
        probe_there_db=_number(probe.get("heard_there_db")),
        probe_here_db=_number(probe.get("heard_here_db")),
        message_bps=_number(message.get("bps")),
        file_bps=_number(file_.get("bps")),
        rungs=rungs,
        notes=str(session.get("notes") or ""),
    )


def ladder_cell(rungs: list[Rung]) -> str:
    return "; ".join(
        f"{r.mode}:{r.decoded}/{r.frames}@{'?' if r.snr_db is None else f'{r.snr_db:.0f}'}"
        for r in rungs
    )


def log_row(s: Session, klass: str, band: str | None, note: str) -> str:
    """A row of ``field/LOG.md``'s table."""
    distance = "?" if s.km is None else f"{s.km:.0f} km"
    if s.my_grid or s.their_grid:
        distance += f" ({s.my_grid or '?'} → {s.their_grid or '?'})"
    snr = "?" if s.snr_db is None else f"{s.snr_db:.1f}"
    goodput = "?" if s.goodput_bps is None else f"{s.goodput_bps:.0f}"
    parts = [note.strip()] if note.strip() else []
    if s.outcome != "session":
        parts.append(f"Test session: {s.outcome}")
    if s.probe_here_db is not None:
        there = "?" if s.probe_there_db is None else f"{s.probe_there_db:.0f}"
        parts.append(f"probe {there} dB there, {s.probe_here_db:.1f} dB here")
    if s.message_bps is not None or s.file_bps is not None:
        rates = [
            f"message {s.message_bps:.0f} b/s" if s.message_bps is not None else "",
            f"file {s.file_bps:.0f} b/s" if s.file_bps is not None else "",
        ]
        parts.append(", ".join(r for r in rates if r))
    if s.rungs:
        parts.append(f"ladder {ladder_cell(s.rungs)}")
    equipment = ", ".join(
        x for x in (s.rig, f"{s.power_w:.0f} W" if s.power_w is not None else "", s.antenna) if x
    )
    if equipment:
        parts.append(equipment)
    if s.notes:
        parts.append(s.notes)
    sentence = "; ".join(parts).replace("|", "/").replace("\n", " ")
    bandwidth = f", {s.bandwidth_hz} Hz" if s.bandwidth_hz else ""
    return (
        f"| {s.date} | {band or '?'}{bandwidth} | {distance} | {klass} | {s.callsign} → {s.remote} "
        f"| `{s.recording}` | {snr} | {goodput} / — b/s | {sentence} |"
    )


PATH_COLUMNS = [
    "date",
    "recording",
    "callsign",
    "remote",
    "my_grid",
    "their_grid",
    "km",
    "band",
    "frequency_hz",
    "bandwidth_hz",
    "class",
    "outcome",
    "snr_db",
    "probe_there_db",
    "probe_here_db",
    "message_bps",
    "file_bps",
    "goodput_bps",
    "rungs",
    "ladder",
    "rig",
    "power_w",
    "antenna",
]

LADDER_COLUMNS = [
    "date",
    "recording",
    "callsign",
    "remote",
    "km",
    "band",
    "bandwidth_hz",
    "class",
    "mode",
    "frames",
    "decoded",
    "fer",
    "snr_db",
]


def _cell(value: object) -> str:
    if value is None:
        return ""
    if isinstance(value, float):
        return f"{value:.1f}"
    return str(value)


def paths_row(s: Session, klass: str, band: str | None) -> dict[str, str]:
    return {
        "date": s.date,
        "recording": s.recording,
        "callsign": s.callsign,
        "remote": s.remote,
        "my_grid": s.my_grid,
        "their_grid": s.their_grid,
        "km": _cell(s.km),
        "band": band or "",
        "frequency_hz": _cell(s.frequency_hz),
        "bandwidth_hz": _cell(s.bandwidth_hz),
        "class": klass,
        "outcome": s.outcome,
        "snr_db": _cell(s.snr_db),
        "probe_there_db": _cell(s.probe_there_db),
        "probe_here_db": _cell(s.probe_here_db),
        "message_bps": _cell(s.message_bps),
        "file_bps": _cell(s.file_bps),
        "goodput_bps": _cell(s.goodput_bps),
        "rungs": str(len(s.rungs)),
        "ladder": ladder_cell(s.rungs),
        "rig": s.rig,
        "power_w": _cell(s.power_w),
        "antenna": s.antenna,
    }


def ladder_rows(s: Session, klass: str, band: str | None) -> list[dict[str, str]]:
    return [
        {
            "date": s.date,
            "recording": s.recording,
            "callsign": s.callsign,
            "remote": s.remote,
            "km": _cell(s.km),
            "band": band or "",
            "bandwidth_hz": _cell(s.bandwidth_hz),
            "class": klass,
            "mode": str(r.mode),
            "frames": str(r.frames),
            "decoded": str(r.decoded),
            "fer": f"{1 - r.decoded / r.frames:.3f}" if r.frames else "",
            "snr_db": _cell(r.snr_db),
        }
        for r in s.rungs
    ]


def _append_csv(path: Path, columns: list[str], rows: list[dict[str, str]]) -> None:
    if not rows:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    new = not path.exists() or path.stat().st_size == 0
    with path.open("a", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=columns)
        if new:
            writer.writeheader()
        writer.writerows(rows)


def already_logged(log: Path, recording: str) -> bool:
    return log.exists() and f"`{recording}`" in log.read_text(encoding="utf-8")


def ingest(
    sidecars: list[Path],
    *,
    log: Path,
    paths: Path,
    ladders: Path,
    klass: str,
    band: str | None,
    note: str,
) -> int:
    """Fold each sidecar in; returns how many were new."""
    added = 0
    for sidecar in sidecars:
        document = json.loads(sidecar.read_text(encoding="utf-8"))
        session = summarise(document, sidecar.stem)
        if already_logged(log, session.recording):
            print(f"{session.recording}: already in the log")
            continue
        session_band = band_of(session.frequency_hz) or band
        row = log_row(session, klass, session_band, note)
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("a", encoding="utf-8") as f:
            f.write(row + "\n")
        _append_csv(paths, PATH_COLUMNS, [paths_row(session, klass, session_band)])
        _append_csv(ladders, LADDER_COLUMNS, ladder_rows(session, klass, session_band))
        print(f"{session.recording}: {session.callsign} → {session.remote}, {session.outcome}")
        added += 1
    return added


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("sidecars", nargs="+", type=Path)
    ap.add_argument("--class", dest="klass", default="?", help="awgn, good, moderate or poor")
    ap.add_argument("--band", default=None, help="when no rig control recorded the frequency")
    ap.add_argument("--note", default="", help="the sentence for the log's last column")
    ap.add_argument("--log", type=Path, default=ROOT / "field" / "LOG.md")
    ap.add_argument("--paths", type=Path, default=ROOT / "field" / "paths.csv")
    ap.add_argument("--ladders", type=Path, default=ROOT / "field" / "ladders.csv")
    args = ap.parse_args()
    try:
        added = ingest(
            args.sidecars,
            log=args.log,
            paths=args.paths,
            ladders=args.ladders,
            klass=args.klass,
            band=args.band,
            note=args.note,
        )
    except (OSError, ValueError, KeyError) as error:
        print(error, file=sys.stderr)
        return 1
    print(f"{added} session(s) added")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
