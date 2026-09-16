# Maintaining Aether HF

How the maintainer handles what arrives: issues, forks, pull requests, releases. Written
for one maintainer who reviews with Claude Code beside them, so it says which commands to
type and what to look for, not just what the policy is. `CONTRIBUTING.md` is the
contributor's side of the same rules.

## 1. What a fork means

A fork is a copy someone made under their own account. It costs nothing, asks nothing of
you and needs no review: people fork to read the code offline, to try a change, or to send
a pull request later. **Forks are not work for you.** The list is at
`https://github.com/KK4ODA/aether-hf/forks`, or

```bash
gh api repos/KK4ODA/aether-hf/forks --paginate --jq '.[] | "\(.full_name)\t\(.pushed_at)"'
```

A fork that has been pushed to recently may hold a change worth asking for; a fork that
diverged for good is its own project under the same licence, which the licence permits.
Nothing in a fork can reach this repository except through a pull request.

## 2. Triage: issues

Read each new issue once and give it one of three answers within a few days:

* **A bug with a way to reproduce it** — label `bug`, and either fix it or say when.
  A diagnostic bundle or a session recording (`.wav` + sidecar) is the evidence to ask
  for; `docs/user/field-test.md` says how to make one.
* **An on-air report** — label `field`, thank them, and fold the recording into
  `field/sessions/` as a regression test when it decodes (or into the field log when it
  does not).
* **A feature request** — say whether it is on the roadmap (`docs/ROADMAP.md` §14 is the
  queue) and where. Things this project has decided *not* to do (a VARA-compatible air
  interface, encryption, chat features that belong in the host program) get a polite
  no with the reason, once, and the issue closed.

Issues asking for support with a host program (Winlink Express, Pat, VarAC) are real and
frequent; answer them, and turn the answer into a line in `docs/user/` so the next one is
a link.

## 3. Reviewing a pull request

### 3.1 Look before you fetch

A pull request from a stranger is code from a stranger. Read the description and the diff
on GitHub first (`gh pr view <n>`, `gh pr diff <n>`) before running anything from it, and
be suspicious of a change that touches `.github/workflows/`, `tools/release.py`, the
updater, or anything that runs on your machine or in CI with secrets. CI runs a fork's
workflow changes only with your approval (the "Approve and run" button on the PR) — leave
that alone until you have read the diff.

### 3.2 Bring it in

```bash
gh pr list                       # what is open
gh pr view 42 --comments         # the description and the conversation
gh pr diff 42                    # the whole diff
gh pr checkout 42                # a local branch tracking the contributor's
```

Then ask Claude Code to review it with you. The useful prompt names what matters:

> Review PR 42 against docs/MAINTAINING.md §3.3. It is checked out. Run the suites the
> change touches, tell me what the diff does in plain words, and list what does not meet
> the rules with the file and line.

`/code-review ultra 42` runs the multi-agent review of the pull request; ask for it when
the change is large or in the modem.

### 3.3 The rules a change is held to

Every one of these has cost the project a day at some point; none is decoration.

1. **CI is green** on the PR (model on Windows and Ubuntu, core, app). A red CI is the
   contributor's to fix, not yours.
2. **Model first.** A change to the modem or the link engine goes into the Python model
   (`model/aether_model/`) *and* the Rust port (`core/`), model first, with the port's
   `tests/model_vectors.rs` still passing. A PR that changes only the port of something the
   model also implements is not mergeable; ask for the model half.
3. **No threshold relaxed.** A test's numeric target moves only with an ADR in
   `docs/adr/` saying why. An `xfail` marker is added only with an audit or ADR reference
   and removed only in the PR that fixes the defect it names.
4. **Vectors regenerate only for a deliberate model change.** A PR that regenerates
   `vectors/`, `core/*/tests/data/*_vectors.json` or `preamble_tables.json` must say which
   model change made the old ones wrong. Otherwise a mismatch was a bug in the core, and
   regenerating hid it.
5. **The air interface is an ADR.** Anything that changes what goes over the air — a
   frame layout, a preamble, a mode table, the handshake — needs an ADR and a note in
   `docs/spec/air-interface.md` (regenerate its tables with `python tools/make_spec.py`),
   and breaks compatibility with every installed station, so it lands in a release whose
   notes say so.
6. **A benchmark delta for DSP and protocol changes.** The PR template asks for it. "No
   change expected" is acceptable for a refactor when the vectors and the suites say so;
   a claimed improvement needs the curve (`tools/bench_phy.py`, `tools/bench_link.py`,
   `tools/bench_floor.py`) in `bench/baselines/` and the table in `bench/README.md`.
7. **Public sources only.** Nothing derived from VARA's internals, from decompiling, or
   from "I captured VARA's audio and matched it". Ask where a design came from when it is
   not obvious; a reference to a standard, a paper or an open implementation is the
   answer you want. Compatibility work targets VARA's *published* host command set.
8. **Licence.** Contributions are MIT OR Apache-2.0 (`CONTRIBUTING.md` §Ground rules). A
   PR that brings in code under another licence (GPL from another modem, for instance)
   cannot be merged whatever its quality.
9. **Conventional Commits**, because the release notes are generated from the commit
   messages by git-cliff. `feat(scope):`, `fix(scope):`, `docs:`, `bench:`, `chore:`.
   A PR whose commits do not follow the form gets rebased by the contributor or, for a
   one-commit PR, reworded by you before merging.
10. **Style.** `ruff`, `mypy`, `cargo fmt --all`, `cargo clippy --all-targets
    --all-features -- -D warnings` — all of which CI runs, so this is item 1 again.

### 3.4 Run it

```bash
python -m uv run --project model --no-sync python -m pytest model/tests -m "not slow" -p no:cacheprovider
cd core && cargo test --release --workspace && cargo clippy --all-targets --all-features -- -D warnings
cd ../app/src-tauri && cargo clippy --all-targets -- -D warnings
```

For a modem change, also the benchmark the PR claims a delta on, at least at one SNR, so
the number in the PR is one you have seen.

### 3.5 Decide

* **Approve and merge** when every rule holds. Merging is a rebase (no squash, no merge
  commit): the commits land on `master` as they are, so their messages must already be
  right.

  ```bash
  gh pr review 42 --approve --body "Reviewed against MAINTAINING.md §3.3; suites green here."
  gh pr merge 42 --rebase --delete-branch
  ```

* **Request changes** with the rule that is not met and the file and line, in plain
  words. One round of review is normal; three means the change is the wrong shape and it
  is kinder to say so.

  ```bash
  gh pr review 42 --request-changes --body "..."
  ```

* **Close** a PR that cannot be merged on principle (a proprietary source, an air-interface
  change without an ADR that the contributor does not want to write, a feature the project
  has declined) with a sentence that says which, and thanks. Do not leave it open to be
  polite; an open PR is a promise.

`gh pr checkout` leaves the branch behind; `git switch master && git branch -D <branch>`
when done. Never rebase or force-push a contributor's branch yourself.

## 4. Releases

`CLAUDE.md` § "Cutting a release" is the procedure (`tools/release.py bump`, the tag, the
Release workflow). Two things a contributor's change can need from you: an air-interface
change means a note in the release that says every station must update, and a change to
the configuration schema means a migration step in `core/aetherd/src/config.rs` and a
fixture under `core/aetherd/tests/data/config/`.

## 5. Repository settings (set 2026-09-16)

What is set, so a change to it is a decision and not an accident:

* **Merge methods**: rebase merging only, so a squash or a merge commit cannot happen by
  a stray click. Head branches are deleted on merge.
* **A ruleset on `master`** ("master: CI green before merge", Settings → Rules): no
  deletion, no force-push, changes arrive by pull request, and the eight CI jobs (model on
  Windows and Ubuntu at two Python versions, core and app on both) must pass before a
  merge. Repository admins bypass it, which is what lets the maintainer keep pushing
  straight to `master`; a contributor cannot.
* **Discussions on**: questions go to Q&A and on-air stories to Show and tell instead of
  into issues; the issue templates' `config.yml` points there.
* **Fork workflow approval** stays at GitHub's default ("require approval for first-time
  contributors"), which is what keeps a PR from running its own workflow with the
  repository's secrets before you have read it.
