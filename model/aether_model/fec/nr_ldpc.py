"""3GPP TS 38.212 §5.3.2 LDPC codes (base graphs 1 and 2) with §5.4.2 rate matching.

This is the Aether FEC (ADR-0003). The base-graph shift tables are loaded from
``data/nr_ldpc_base_graphs.json``, extracted verbatim from the published specification by
``tools/extract_nr_ldpc_tables.py``. Nothing in this module is 5G-specific beyond the code
itself: it is a very good, public, rate-compatible QC-LDPC family whose circular-buffer rate
matching gives incremental-redundancy HARQ for free.

Conventions
-----------
* Bits are ``uint8`` arrays. LLRs are ``float64``, **positive = bit 0**.
* A *full codeword* has ``cols·Z`` bits: ``K = kb·Z`` systematic (the first ``2Z`` of which
  are never transmitted) followed by ``(cols − kb)·Z`` parity bits.
* The *circular buffer* ``d = c[2Z:]`` has ``N_cb = (cols − 2)·Z`` bits (66Z for BG1, 50Z for
  BG2). Filler bits (positions ``K′ ≤ k < K`` of the systematic part) are skipped by bit
  selection and given a large positive LLR at the receiver.
* The decoder is a batched layered normalised-min-sum working directly on the QC structure.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from functools import cache
from importlib import resources

import numpy as np
from numpy.typing import NDArray

BitArray = NDArray[np.uint8]
FloatArray = NDArray[np.float64]

LIFTING_SETS: tuple[tuple[int, ...], ...] = (
    (2, 4, 8, 16, 32, 64, 128, 256),
    (3, 6, 12, 24, 48, 96, 192, 384),
    (5, 10, 20, 40, 80, 160, 320),
    (7, 14, 28, 56, 112, 224),
    (9, 18, 36, 72, 144, 288),
    (11, 22, 44, 88, 176, 352),
    (13, 26, 52, 104, 208),
    (15, 30, 60, 120, 240),
)
"""Table 5.3.2-1: lifting sizes Z grouped by set index i_LS."""

ALL_LIFTING_SIZES: tuple[int, ...] = tuple(sorted(z for s in LIFTING_SETS for z in s))

FILLER_LLR = 1e3
"""LLR assigned to filler bits (known zeros) at the decoder input."""


def lifting_set_index(z: int) -> int:
    for i, zs in enumerate(LIFTING_SETS):
        if z in zs:
            return i
    raise ValueError(f"{z} is not a valid lifting size (Table 5.3.2-1)")


# ── Base graphs ───────────────────────────────────────────────────────


@dataclass(frozen=True)
class BaseGraph:
    number: int
    rows: int
    cols: int
    kb_max: int
    entries: tuple[tuple[int, int, tuple[int, ...]], ...]
    """(row i, column j, (V_ij for i_LS = 0 … 7))."""

    @property
    def n_cb_blocks(self) -> int:
        return self.cols - 2


@cache
def base_graph(number: int) -> BaseGraph:
    if number not in (1, 2):
        raise ValueError("base graph must be 1 or 2")
    text = resources.files("aether_model.fec").joinpath("data/nr_ldpc_base_graphs.json").read_text()
    raw = json.loads(text)[f"bg{number}"]
    entries = tuple((int(i), int(j), tuple(int(v) for v in vs)) for i, j, vs in raw["entries"])
    return BaseGraph(number, int(raw["rows"]), int(raw["cols"]), 22 if number == 1 else 10, entries)


def kb_for(bg: int, info_len: int) -> int:
    """K_b per TS 38.212 §5.2.2 for a code block of ``info_len`` bits (B, incl. CRC)."""
    if bg == 1:
        return 22
    if info_len > 640:
        return 10
    if info_len > 560:
        return 9
    if info_len > 192:
        return 8
    return 6


def select_lifting_size(bg: int, info_len: int) -> int:
    """Smallest Z with kb·Z ≥ info_len (§5.2.2)."""
    kb = kb_for(bg, info_len)
    for z in ALL_LIFTING_SIZES:
        if kb * z >= info_len:
            return z
    raise ValueError(f"{info_len} information bits exceed base graph {bg} (max {kb * 384})")


# ── The code ──────────────────────────────────────────────────────────


class NrLdpcCode:
    """One (base graph, Z) instance: systematic encoder, parity check, layered decoder."""

    def __init__(self, bg: int, z: int) -> None:
        self.bg = base_graph(bg)
        self.z = int(z)
        self.i_ls = lifting_set_index(self.z)
        self.kb = self.bg.kb_max
        self.k = self.kb * self.z
        self.n_full = self.bg.cols * self.z
        self.n_cb = self.bg.n_cb_blocks * self.z
        self.m = self.bg.rows * self.z
        # Per block row: arrays of (column, shift) with shift = V mod Z.
        rows: list[list[tuple[int, int]]] = [[] for _ in range(self.bg.rows)]
        for i, j, vs in self.bg.entries:
            rows[i].append((j, vs[self.i_ls] % self.z))
        self._rows = [(np.array([j for j, _ in r]), np.array([s for _, s in r])) for r in rows]
        self._prepare_encoder()

    # ── encoding ──────────────────────────────────────────────────────

    def _prepare_encoder(self) -> None:
        """Find the effective cyclic shift of parity column kb after summing block rows 0–3.

        The three entries of column kb in rows 0–3 have shifts (s_a, s_b, s_c) where two are
        equal and cancel over GF(2), leaving a single P^s. We compute it numerically rather
        than hard-coding which row carries the odd shift, so both base graphs and all lifting
        sets are handled identically."""
        kb = self.kb
        acc = np.zeros(self.z, dtype=np.uint8)
        e = np.zeros(self.z, dtype=np.uint8)
        e[0] = 1
        for i in range(4):
            cols, shifts = self._rows[i]
            for j, s in zip(cols, shifts, strict=True):
                if j == kb:
                    acc ^= np.roll(e, -s)  # P^s applied to the unit vector
        nz = np.flatnonzero(acc)
        if len(nz) != 1:
            raise RuntimeError("unexpected core structure: summed column kb is not a single shift")
        # acc = roll(e, -s) has its 1 at index (-s) mod Z
        self._p1_shift = int((-nz[0]) % self.z)

    def encode(self, info: NDArray[np.integer]) -> BitArray:
        """Systematic encoding of ``K = kb·Z`` bits (fillers already zero) → full codeword."""
        c = np.asarray(info, dtype=np.uint8)
        if c.shape != (self.k,):
            raise ValueError(f"expected {self.k} information bits, got {c.shape}")
        z, kb = self.z, self.kb
        cw = np.zeros(self.n_full, dtype=np.uint8)
        cw[: self.k] = c
        seg = cw.reshape(-1, z)  # block view, writes through

        def row_sum_over(i: int, columns: set[int]) -> BitArray:
            """XOR of P^s · c_j over the entries (j, s) of block row i with j in ``columns``."""
            acc = np.zeros(z, dtype=np.uint8)
            cols, shifts = self._rows[i]
            for j, s in zip(cols, shifts, strict=True):
                if j in columns:
                    acc ^= np.roll(seg[j], -s)
            return acc

        info_cols = set(range(kb))
        # p1: sum of rows 0..3 restricted to information columns, then undo the single shift.
        s0123 = np.zeros(z, dtype=np.uint8)
        for i in range(4):
            s0123 ^= row_sum_over(i, info_cols)
        seg[kb] = np.roll(s0123, self._p1_shift)  # P^{-s} · (Σ A_i c)  → p1
        # Remaining core parities p2..p4 by substitution: each of rows 0..3 introduces one
        # new unknown (shift 0) once the previous ones are known.
        known = set(range(kb + 1))
        for _ in range(3):
            for i in range(4):
                cols, _ = self._rows[i]
                unknown = [j for j in cols if j not in known and j < kb + 4]
                if len(unknown) == 1:
                    j_new = unknown[0]
                    seg[j_new] = row_sum_over(i, known)  # shift of the new unknown is 0
                    known.add(j_new)
                    break
        # Extension parities: block row i ≥ 4 contains exactly one identity entry at column kb+i.
        for i in range(4, self.bg.rows):
            seg[kb + i] = row_sum_over(i, known)
            known.add(kb + i)
        return cw

    def syndrome_ok(self, codeword: NDArray[np.integer]) -> bool:
        cw = np.asarray(codeword, dtype=np.uint8)
        if cw.shape != (self.n_full,):
            raise ValueError(f"expected {self.n_full} bits, got {cw.shape}")
        seg = cw.reshape(-1, self.z)
        for cols, shifts in self._rows:
            acc = np.zeros(self.z, dtype=np.uint8)
            for j, s in zip(cols, shifts, strict=True):
                acc ^= np.roll(seg[j], -s)
            if acc.any():
                return False
        return True

    # ── decoding ──────────────────────────────────────────────────────

    def decode(
        self,
        llr: FloatArray,
        max_iter: int = 25,
        alpha: float = 0.8,
        early_stop: bool = True,
    ) -> tuple[BitArray, NDArray[np.bool_], NDArray[np.int64]]:
        """Layered normalised min-sum on full-codeword LLRs.

        ``llr`` is ``(n_full,)`` or ``(batch, n_full)``: 0 for punctured bits, ``FILLER_LLR``
        for fillers. Returns ``(hard bits, converged flags, iterations used)`` with the batch
        axis preserved (squeezed for 1-D input).
        """
        x = np.asarray(llr, dtype=np.float64)
        single = x.ndim == 1
        if single:
            x = x[None, :]
        if x.shape[1] != self.n_full:
            raise ValueError(f"expected {self.n_full} LLRs per codeword, got {x.shape[1]}")
        b = x.shape[0]
        z = self.z
        post = x.copy().reshape(b, -1, z)  # (batch, block col, Z)
        # check-to-variable messages, one Z-vector per edge per codeword
        r_msgs = [np.zeros((b, len(cols), z)) for cols, _ in self._rows]
        converged = np.zeros(b, dtype=bool)
        iters = np.full(b, max_iter, dtype=np.int64)

        for it in range(1, max_iter + 1):
            for i, (cols, shifts) in enumerate(self._rows):
                # v2c: variable posteriors aligned to the checks of this block row, minus
                # this row's previous contribution.
                v = np.stack(
                    [np.roll(post[:, j, :], -s, axis=1) for j, s in zip(cols, shifts, strict=True)],
                    axis=1,
                )  # (b, deg, z)
                v -= r_msgs[i]
                mag = np.abs(v)
                sgn = np.sign(v)
                sgn[sgn == 0] = 1.0
                prod = np.prod(sgn, axis=1, keepdims=True)  # (b, 1, z)
                idx0 = np.argmin(mag, axis=1, keepdims=True)  # (b, 1, z)
                min1 = np.take_along_axis(mag, idx0, axis=1)
                is_min = np.arange(len(cols))[None, :, None] == idx0  # (b, deg, z)
                min2 = np.where(is_min, np.inf, mag).min(axis=1, keepdims=True)
                c2v = np.where(is_min, min2, min1)
                c2v = alpha * c2v * prod * sgn
                r_msgs[i] = c2v
                new = v + c2v
                for e, (j, s) in enumerate(zip(cols, shifts, strict=True)):
                    post[:, j, :] = np.roll(new[:, e, :], s, axis=1)
            if early_stop:
                hard = (post < 0).astype(np.uint8)
                ok = self._syndromes_zero(hard)
                newly = ok & ~converged
                iters[newly] = it
                converged |= ok
                if converged.all():
                    break
        hard = (post < 0).astype(np.uint8).reshape(b, -1)
        if not early_stop:
            converged = self._syndromes_zero(hard.reshape(b, -1, z))
        if single:
            return hard[0], converged[:1], iters[:1]
        return hard, converged, iters

    def _syndromes_zero(self, hard: BitArray) -> NDArray[np.bool_]:
        """``hard``: (batch, block col, Z) → per-codeword flag that every check is satisfied."""
        ok = np.ones(hard.shape[0], dtype=bool)
        for cols, shifts in self._rows:
            acc = np.zeros((hard.shape[0], self.z), dtype=np.uint8)
            for j, s in zip(cols, shifts, strict=True):
                acc ^= np.roll(hard[:, j, :], -s, axis=1)
            ok &= ~acc.any(axis=1)
        return ok


@cache
def nr_ldpc_code(bg: int, z: int) -> NrLdpcCode:
    return NrLdpcCode(bg, z)


# ── Rate matching (§5.4.2) ────────────────────────────────────────────

_K0_NUMERATORS = {1: (0, 17, 33, 56), 2: (0, 13, 25, 43)}
"""Table 5.4.2.1-2 numerators: k0 = floor(num · N_cb / N) · Z with N_cb = N (no limited buffer)."""


class RateMatcher:
    """Circular-buffer bit selection with redundancy versions, and its soft inverse.

    ``info_len`` (K′) marks where filler bits start inside the systematic part; they are
    skipped on transmit and pinned to ``FILLER_LLR`` on receive.
    """

    def __init__(self, code: NrLdpcCode, info_len: int, e: int, rv: int = 0) -> None:
        if not 0 <= rv <= 3:
            raise ValueError("rv must be 0 … 3")
        if not 2 * code.z <= info_len <= code.k:
            raise ValueError(f"info_len must be within [{2 * code.z}, {code.k}]")
        self.code = code
        self.info_len = int(info_len)
        self.e = int(e)
        self.rv = int(rv)
        z = code.z
        self.k0 = (_K0_NUMERATORS[code.bg.number][rv] * code.n_cb // code.n_cb) * z  # N_cb == N
        # Positions in the circular buffer d (length n_cb) that are fillers: d_k = c_{k+2Z}.
        filler = np.zeros(code.n_cb, dtype=bool)
        filler[self.info_len - 2 * z : code.k - 2 * z] = True
        self._usable = np.flatnonzero(~filler)  # ordered circular-buffer positions to read
        # Bit selection order starting at k0, skipping fillers, wrapping around.
        start = int(np.searchsorted(self._usable, self.k0))
        order = np.concatenate((self._usable[start:], self._usable[:start]))
        reps = -(-self.e // len(order))
        self.positions = np.tile(order, reps)[: self.e]
        """Circular-buffer index transmitted at each output position."""

    def match(self, codeword: NDArray[np.integer]) -> BitArray:
        cw = np.asarray(codeword, dtype=np.uint8)
        d = cw[2 * self.code.z :]
        return d[self.positions]

    def recover(self, llr_e: FloatArray, buffer: FloatArray | None = None) -> FloatArray:
        """Soft rate recovery → full-codeword LLRs (punctured = 0, fillers = FILLER_LLR).

        Pass the previous transmission's *full-codeword* LLRs as ``buffer`` to combine
        redundancy versions (HARQ-IR); the result is a new array.
        """
        z = self.code.z
        if buffer is None:
            full = np.zeros(self.code.n_full, dtype=np.float64)
            full[self.info_len : self.code.k] = FILLER_LLR
        else:
            full = np.array(buffer, dtype=np.float64, copy=True)
        acc = np.zeros(self.code.n_cb, dtype=np.float64)
        np.add.at(acc, self.positions, np.asarray(llr_e, dtype=np.float64))
        full[2 * z :] += acc
        return full


def nr_bit_interleave(e: NDArray, q_m: int) -> NDArray:
    """§5.4.2.2 bit interleaver: ``f[i + j·Q_m] = e[i·E/Q_m + j]``."""
    e = np.asarray(e)
    if len(e) % q_m:
        raise ValueError("E must be a multiple of Q_m")
    return e.reshape(q_m, -1).T.reshape(-1)


def nr_bit_deinterleave(f: NDArray, q_m: int) -> NDArray:
    f = np.asarray(f)
    return f.reshape(-1, q_m).T.reshape(-1)
