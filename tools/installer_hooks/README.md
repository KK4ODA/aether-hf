# The installer's hooks, under Wine

`app/src-tauri/windows/hooks.nsh` runs before Tauri's own running-app check, in the Windows
installer and uninstaller. It makes sure nothing holds the shell's or the modem's file:
it waits, asks the shell to close (the modem stops first), asks the modem to stop over its
control port, sets aside a file still held as `<name>.old`, and names the program that held
it. The header of `hooks.nsh` says why: Tauri's check gives up at the first refusal, and
from bundler 2.10 that stopped updates with *Failed to kill Aether HF*.

`run.py` tests all of that on Linux, under Wine:

- **The hook scenarios** drive `harness.nsi`, which inserts the hooks with every wait cut to a
  tenth. Covered: nothing held; a shell that lets go by itself; one that closes when asked;
  two holders that never let go (set aside, and the report reads the list); an earlier
  `.old` still in use (`.old2`); a modem that stops when asked, and one that does not (never
  closed by force); the report's names and process ids; a fresh install; the uninstaller and
  its cleanup; the short wait when someone is there. It also checks that the hooks give back
  the registers and the stack as they found them.
- **`--installer`** builds the real installer, with the Tauri CLI `release.yml` pins
  (`TAURI_CLI`), from placeholder binaries in a copy of `app/` in the work folder, and runs it
  silently: a fresh install, an update while the shell holds on until asked, an update while
  something never lets go, and an uninstall.

Wine's Restart Manager reports nothing, so `fake_rstrtmgr.c` takes its place. NSIS loads
plugins' DLLs from the system folder only, so the fake goes into the prefix's `syswow64`. A
file `P` is held while `P.held` exists beside it (its text is how many hold it), and lets go
when asked if `P.closes` exists (its text is the delay in milliseconds). `fake_curl.c` logs
what the modem was asked, and deletes the file `FAKE_CURL_FREES` names, as a modem stopping.

It cannot show Windows' own Restart Manager with real processes, the shell answering it
(`stop_on_exit` in `app/src-tauri/src/main.rs`), or the real `curl.exe`. Those need Windows:
when the pinned CLI is bumped, also try an update across the bump on a Windows machine.

## Running it

```bash
sudo dpkg --add-architecture i386 && sudo apt-get update
sudo apt-get install wine wine32:i386 wine64 nsis gcc-mingw-w64-i686 xvfb
python3 tools/installer_hooks/run.py              # the hook scenarios, about a minute
python3 tools/installer_hooks/run.py --installer  # and the real installer, about three
```

`--installer` also needs `npx` and `cargo`. `--work DIR` keeps the Wine prefix and the logs
(`prefix/drive_c/t/rm.log` is every call the fake took). CI runs it with `--installer`
whenever the hooks, this folder or `release.yml` change (`.github/workflows/installer-hooks.yml`).
