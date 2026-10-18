"""Test the installer's hooks (`app/src-tauri/windows/hooks.nsh`) under Wine.

The hooks run before Tauri's own running-app check, in the installer and the uninstaller,
and make sure nothing holds the shell's or the modem's file: they wait, ask the shell to
close, ask the modem to stop over its control port, set aside what is still held and name
who held it. Wine's Restart Manager is a stub that reports nothing, so a fake one
(`fake_rstrtmgr.c`) takes its place in the prefix's system folder — NSIS loads plugins' DLLs
from there only — with a file's holders, and whether they let go when asked, set by marker
files beside it. A fake `curl.exe` logs what the modem was asked.

The hook scenarios drive `harness.nsi`, which inserts the hooks with waits cut to a tenth.
With `--installer` the real installer is built as well, by the Tauri CLI that `release.yml`
pins, from placeholder binaries (a copy of `app/` in the work folder: nothing in the tree is
touched), and run silently through a fresh install, two updates and an uninstall.

What this cannot show: Windows' own Restart Manager with real processes, the shell answering
its request (`stop_on_exit` in `main.rs`) and the real curl.exe. Those need Windows.

    python tools/installer_hooks/run.py [--installer] [--work DIR]

Needs wine with 32-bit support, makensis, i686-w64-mingw32-gcc and xvfb-run; `--installer`
needs npx and cargo too (the CLI reads the shell's Cargo metadata). The README beside this
says which packages.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Callable
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
HOOKS = ROOT / "app" / "src-tauri" / "windows" / "hooks.nsh"


def cli_version() -> str:
    """The Tauri CLI that `release.yml` builds the installers with (`TAURI_CLI`)."""
    text = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    found = re.search(r'^\s*TAURI_CLI:\s*"([^"]+)"', text, re.MULTILINE)
    if not found:
        sys.exit("release.yml pins no TAURI_CLI")
    return found.group(1)


def need(*tools: str) -> None:
    missing = [tool for tool in tools if not shutil.which(tool)]
    if missing:
        sys.exit(
            f"missing: {', '.join(missing)} (tools/installer_hooks/README.md says which packages)"
        )


def sh(*command: str | Path, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    subprocess.run([str(part) for part in command], cwd=cwd, env=env, check=True)


class Bench:
    """A Wine prefix with the fakes in place, and the folder the scenarios install into."""

    def __init__(self, work: Path) -> None:
        self.work = work
        self.prefix = work / "prefix"
        self.t = self.prefix / "drive_c" / "t"
        self.inst = self.t / "inst"
        self.env = dict(
            os.environ,
            WINEPREFIX=str(self.prefix),
            WINEDEBUG="-all",
            WINEDLLOVERRIDES="rstrtmgr=n,b;mscoree,mshtml=",
            FAKE_RM_LOG=r"C:\t\rm.log",
            FAKE_CURL_LOG=r"C:\t\curl.log",
        )
        self.failures: list[str] = []

    # ── building ──────────────────────────────────────────────────────

    def build(self) -> None:
        gcc = "i686-w64-mingw32-gcc"
        sh(
            gcc,
            "-O2",
            "-shared",
            "-o",
            self.work / "rstrtmgr.dll",
            HERE / "fake_rstrtmgr.c",
            "-Wl,--kill-at",
            "-static-libgcc",
        )
        sh(gcc, "-O2", "-o", self.work / "curl.exe", HERE / "fake_curl.c", "-static-libgcc")
        for variant in ([], ["-DINTERACTIVE"]):
            # written to the work folder: -NOCD keeps makensis there
            sh(
                "makensis",
                "-NOCD",
                "-V2",
                "-INPUTCHARSET",
                "UTF8",
                f"-DHOOKS={HOOKS}",
                *variant,
                HERE / "harness.nsi",
                cwd=self.work,
            )
        if not (self.prefix / "drive_c" / "windows").is_dir():
            sh("xvfb-run", "-a", "wineboot", "-i", env=self.env)
        windows = self.prefix / "drive_c" / "windows"
        for folder in ("system32", "syswow64"):
            shutil.copy(self.work / "curl.exe", windows / folder / "curl.exe")
        # NSIS loads plugins' DLLs from the system folder only (SetDefaultDllDirectories): the
        # fake takes the 32-bit one's place, as the native DLL the override prefers
        shutil.copy(self.work / "rstrtmgr.dll", windows / "syswow64" / "rstrtmgr.dll")

    # ── running ───────────────────────────────────────────────────────

    def check(self, what: str, ok: bool) -> None:
        print(("  ok    " if ok else "  FAIL  ") + what)
        if not ok:
            self.failures.append(what)

    def fresh(self, files: list[str], markers: dict[str, str] | None = None) -> None:
        shutil.rmtree(self.t, ignore_errors=True)
        self.inst.mkdir(parents=True)
        for name in ("harness.exe", "harness-ui.exe"):
            shutil.copy(self.work / name, self.t)
        for name in files:
            (self.inst / name).write_bytes(b"MZ old " + name.encode())
        for name, text in (markers or {}).items():
            (self.inst / name).write_text(text)

    def harness(
        self,
        scenario: str,
        *flags: str,
        env: dict[str, str] | None = None,
        after: Callable[[], None] | None = None,
        ui: bool = False,
    ) -> dict[str, str]:
        if after:
            threading.Thread(target=after, daemon=True).start()
        command = ["wine", str(self.t / "harness.exe"), "/S"]
        if ui:
            # a wineserver left from the silent runs has no display to give a window
            subprocess.run(["wineserver", "-w"], env=self.env, check=False, timeout=60)
            command = ["xvfb-run", "-a", "wine", str(self.t / "harness-ui.exe")]
        subprocess.run(
            [*command, f"/SCENARIO={scenario}", *flags, r"/D=C:\t\inst"],
            env=dict(self.env, **(env or {})),
            check=False,
            timeout=180,
        )
        result = self.t / "result.txt"
        text = result.read_text(encoding="utf-8", errors="replace") if result.exists() else ""
        return dict(line.split("=", 1) for line in text.splitlines() if "=" in line)

    def log(self) -> list[str]:
        path = self.t / "rm.log"
        return path.read_text().splitlines() if path.exists() else []

    def ticks(self, word: str) -> list[int]:
        """When the fake was called with `word`, in milliseconds."""
        return [int(line.split()[0]) for line in self.log() if word in line]

    def asked_to_close(self) -> int:
        return len(self.ticks("shutdown"))

    def unchanged(self, result: dict[str, str]) -> None:
        self.check(
            "registers and stack come back as they were",
            result.get("registers") == "s0,s1,s2,s3,s4,s5,s6,s7,s8,s9"
            and result.get("stack") == "sentinel",
        )


def hook_scenarios(bench: Bench) -> None:
    inst = bench.inst
    both = ["aether-hf.exe", "aetherd.exe"]

    print("1. nothing holds the files (an update)")
    bench.fresh(both)
    bench.unchanged(bench.harness("letgo", "/UPDATE", "/P"))
    bench.check(
        "one look at each file, nothing asked",
        len(bench.ticks("getlist")) == 2 and bench.asked_to_close() == 0,
    )

    print("2. the shell lets go by itself 0.8 s in (an update's shell exiting)")
    bench.fresh(both, {"aether-hf.exe.held": "1"})

    def lets_go() -> None:
        while not bench.ticks("getlist"):
            time.sleep(0.02)
        time.sleep(0.8)
        (inst / "aether-hf.exe.held").unlink()

    bench.unchanged(bench.harness("letgo", "/UPDATE", "/P", after=lets_go))
    bench.check(
        "not asked to close, not set aside",
        bench.asked_to_close() == 0
        and (inst / "aether-hf.exe").exists()
        and not (inst / "aether-hf.exe.old").exists(),
    )

    print("3. the shell holds on until asked, then closes in 0.3 s")
    bench.fresh(both, {"aether-hf.exe.held": "1", "aether-hf.exe.closes": "300"})
    bench.unchanged(bench.harness("letgo", "/UPDATE", "/P"))
    looked, asked = bench.ticks("getlist"), bench.ticks("shutdown flags=1 ")
    bench.check("asked once, with RmForceShutdown", len(asked) == 1 and bench.asked_to_close() == 1)
    if looked and asked:
        waited = (asked[0] - looked[0]) / 1000
        bench.check(f"asked after the unattended wait (1.5 s): {waited:.1f} s", 1.4 <= waited <= 3)
    bench.check(
        "not set aside",
        (inst / "aether-hf.exe").exists() and not (inst / "aether-hf.exe.old").exists(),
    )

    print("4. two processes hold the shell and never let go")
    bench.fresh(both, {"aether-hf.exe.held": "2"})
    bench.unchanged(bench.harness("letgo", "/UPDATE", "/P"))
    looked = bench.ticks("getlist")
    span = (looked[-1] - looked[0]) / 1000 if looked else 0
    bench.check(f"waited 1.5 s, asked, waited 2 s: {span:.1f} s", 3.3 <= span <= 7)
    bench.check(
        "set aside as aether-hf.exe.old",
        (inst / "aether-hf.exe.old").read_bytes() == b"MZ old aether-hf.exe",
    )
    bench.check("aether-hf.exe free for the new one", not (inst / "aether-hf.exe").exists())
    bench.check("the report read the list into a buffer", bool(bench.ticks("buffer=yes")))
    bench.check("the modem's file untouched", (inst / "aetherd.exe").exists())

    print("5. an earlier aether-hf.exe.old still in use: the next name is .old2")
    bench.fresh(both, {"aether-hf.exe.held": "1"})
    (inst / "aether-hf.exe.old").mkdir()  # cannot be deleted, as one still running cannot
    bench.harness("letgo", "/UPDATE", "/P")
    bench.check(
        "set aside as aether-hf.exe.old2",
        (inst / "aether-hf.exe.old2").read_bytes() == b"MZ old aether-hf.exe",
    )

    print("6. the modem holds on, and stops when asked over its port")
    bench.fresh(both, {"aetherd.exe.held": "1"})
    bench.unchanged(
        bench.harness(
            "letgo", "/UPDATE", "/P", env={"FAKE_CURL_FREES": r"C:\t\inst\aetherd.exe.held"}
        )
    )
    curl_log = bench.t / "curl.log"
    curl = curl_log.read_text() if curl_log.exists() else ""
    print("    curl ran as:", curl.strip())
    bench.check(
        "asked with POST /v1/shutdown on 127.0.0.1:8515",
        "--request POST" in curl
        and '--data "{}"' in curl
        and "http://127.0.0.1:8515/v1/shutdown" in curl,
    )
    bench.check("never closed by force", bench.asked_to_close() == 0)
    bench.check(
        "not set aside", (inst / "aetherd.exe").exists() and not (inst / "aetherd.exe.old").exists()
    )

    print("7. the modem never stops")
    bench.fresh(both, {"aetherd.exe.held": "1"})
    bench.unchanged(bench.harness("letgo", "/UPDATE", "/P"))
    bench.check("never closed by force", bench.asked_to_close() == 0)
    bench.check(
        "set aside as aetherd.exe.old",
        (inst / "aetherd.exe.old").exists() and not (inst / "aetherd.exe").exists(),
    )

    print("8. the report names each holder and its process")
    bench.fresh(["probe.exe"], {"probe.exe.held": "3"})
    said = bench.harness("say", "/UPDATE").get("say")
    print("    the last holder read:", said)
    bench.check("the last of three: process 4243, Fake Holder 3", said == "4243|Fake Holder 3")

    print("9. a fresh install: no files at all")
    bench.fresh([])
    bench.unchanged(bench.harness("letgo"))
    bench.check("the Restart Manager not asked", not bench.log())

    print("10. the uninstaller does the same, and what it set aside goes after")
    bench.fresh(both, {"aether-hf.exe.held": "1"})
    bench.unchanged(bench.harness("unletgo", "/P"))
    bench.check("set aside", (inst / "aether-hf.exe.old").exists())
    for name in ("aether-hf.exe", "aetherd.exe", "aether-hf.exe.held"):
        (inst / name).unlink(missing_ok=True)
    (inst / "aetherd.exe.old3").write_bytes(b"x")
    bench.harness("post", "/P")
    bench.check("POSTUNINSTALL deleted what was set aside, and the folder", not inst.exists())

    print("11. with someone there: a glance, not the unattended wait")
    bench.fresh(both, {"aetherd.exe.held": "1"})
    result = bench.harness("letgo", env={"FAKE_CURL_FREES": r"C:\t\inst\aetherd.exe.held"}, ui=True)
    bench.unchanged(result)
    bench.check("neither silent nor passive", result.get("modes") == "update 0 passive 0")
    looked = bench.ticks("getlist")
    span = (looked[-1] - looked[0]) / 1000 if looked else 99
    bench.check(f"the modem asked after the 0.3 s glance: {span:.1f} s of looking", span < 1.2)
    bench.check("not set aside", (inst / "aetherd.exe").exists())


def bundle(bench: Bench) -> Path:
    """The real installer, by the pinned Tauri CLI, from placeholder binaries."""
    app = bench.work / "app"
    shutil.rmtree(app, ignore_errors=True)
    shutil.copytree(ROOT / "app" / "ui", app / "ui")
    shutil.copytree(
        ROOT / "app" / "src-tauri",
        app / "src-tauri",
        ignore=shutil.ignore_patterns("target", "binaries"),
    )
    target = "x86_64-pc-windows-msvc"
    release = app / "src-tauri" / "target" / target / "release"
    release.mkdir(parents=True)
    (app / "src-tauri" / "binaries").mkdir()
    placeholder = bench.work / "curl.exe"  # any Windows program does
    for path in (
        release / "aether-hf.exe",
        release / "aether-app.exe",
        app / "src-tauri" / "binaries" / f"aetherd-{target}.exe",
    ):
        shutil.copy(placeholder, path)
    sh(
        "npx",
        "--yes",
        f"@tauri-apps/cli@{cli_version()}",
        "bundle",
        "--bundles",
        "nsis",
        "--target",
        target,
        "--no-binary-patching",
        "--no-sign",
        "--ci",
        cwd=app / "src-tauri",
    )
    built = list((release / "bundle" / "nsis").glob("*-setup.exe"))
    if len(built) != 1:
        sys.exit(f"expected one installer, found {built}")
    return built[0]


def installer_scenarios(bench: Bench, setup: Path) -> None:
    real = bench.t / "real"
    shutil.rmtree(bench.t, ignore_errors=True)
    bench.t.mkdir(parents=True)
    shutil.copy(setup, bench.t / "setup.exe")
    # WebView2 present, so the installer's own section does not try to download it
    subprocess.run(
        [
            "wine",
            "reg",
            "add",
            r"HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
            "/v",
            "pv",
            "/d",
            "130.0.0.0",
            "/f",
        ],
        env=bench.env,
        check=True,
        capture_output=True,
    )

    def install(*flags: str) -> int:
        (bench.t / "rm.log").unlink(missing_ok=True)
        done = subprocess.run(
            ["wine", str(bench.t / "setup.exe"), "/S", *flags, r"/D=C:\t\real"],
            env=bench.env,
            check=False,
            timeout=600,
        )
        return done.returncode

    print(f"R1. a fresh silent install ({setup.name})")
    bench.check("exit 0", install() == 0)
    bench.check(
        "aether-hf.exe, aetherd.exe, ui and the uninstaller are there",
        all(
            (real / name).exists()
            for name in ("aether-hf.exe", "aetherd.exe", "uninstall.exe", "ui")
        ),
    )

    print("R2. an update while the shell holds on until asked, then closes in 0.3 s")
    (real / "aether-hf.exe").write_bytes(b"OLD")
    (real / "aether-hf.exe.held").write_text("1")
    (real / "aether-hf.exe.closes").write_text("300")
    bench.check("exit 0", install("/UPDATE", "/P") == 0)
    looked, asked = bench.ticks("getlist"), bench.ticks("shutdown")
    if looked and asked:
        waited = (asked[0] - looked[0]) / 1000
        bench.check(
            f"asked once, after the 20 s wait: {waited:.1f} s",
            len(asked) == 1 and 19.5 <= waited <= 25,
        )
    else:
        bench.check("asked to close", False)
    bench.check(
        "the new aether-hf.exe written, nothing set aside",
        (real / "aether-hf.exe").read_bytes()[:2] == b"MZ"
        and not (real / "aether-hf.exe.old").exists(),
    )
    (real / "aether-hf.exe.closes").unlink()

    print("R3. an update while something holds aether-hf.exe and never lets go")
    (real / "aether-hf.exe").write_bytes(b"OLD")
    (real / "aether-hf.exe.held").write_text("1")
    bench.check("exit 0", install("/UPDATE", "/P") == 0)
    bench.check(
        "the old one set aside as aether-hf.exe.old",
        (real / "aether-hf.exe.old").read_bytes() == b"OLD",
    )
    bench.check("the new aether-hf.exe written", (real / "aether-hf.exe").read_bytes()[:2] == b"MZ")
    log = bench.log()
    bench.check(
        "Tauri's own check, after the hooks, found the file free",
        bool(log) and "holders=0" in [line for line in log if "getlist" in line][-1],
    )

    print("R4. uninstall while aether-hf.exe is held")
    (real / "aether-hf.exe.held").write_text("1")
    subprocess.run(
        ["wine", str(real / "uninstall.exe"), "/S", "/P"], env=bench.env, check=False, timeout=600
    )
    # an NSIS uninstaller runs from a copy of itself and returns at once; the hooks then wait
    # 20 s, ask, and wait 30 s more
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline and (real / "aetherd.exe").exists():
        time.sleep(1)
    time.sleep(3)
    left = sorted(path.name for path in real.iterdir()) if real.exists() else []
    print("    left:", left)
    bench.check(
        "the programs are gone, and what was set aside",
        not {"aether-hf.exe", "aetherd.exe", "aether-hf.exe.old"} & set(left),
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--installer",
        action="store_true",
        help="also build the real installer with the pinned CLI and run it",
    )
    parser.add_argument("--work", type=Path, help="work folder (default: a new temporary one)")
    args = parser.parse_args()
    need("wine", "wineserver", "makensis", "i686-w64-mingw32-gcc", "xvfb-run")
    if args.installer:
        need("npx", "cargo")
    work = args.work or Path(tempfile.mkdtemp(prefix="aether-installer-hooks-"))
    work.mkdir(parents=True, exist_ok=True)
    bench = Bench(work.resolve())
    bench.build()
    hook_scenarios(bench)
    if args.installer:
        installer_scenarios(bench, bundle(bench))
    subprocess.run(["wineserver", "-k"], env=bench.env, check=False)
    print()
    if bench.failures:
        print("FAILED: " + "; ".join(bench.failures))
        sys.exit(1)
    print("all passed")


if __name__ == "__main__":
    main()
