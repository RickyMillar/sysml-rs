#!/usr/bin/env python3
"""Deterministic generator for the espresso-pump-hybrid check-valve characteristics.

This emits two CSV lookup tables consumed by the model's ``@DataSource``-backed
``SampledFunction`` attributes:

  * ``data/generated_pump_opening.csv`` — effective flow area on the *opening*
    (advancing) branch, as a function of the normalized valve command ``u``.
  * ``data/generated_pump_closing.csv`` — effective flow area on the *closing*
    (retracting) branch. It lags the opening branch by a bounded hysteresis
    separation that vanishes at both command endpoints.

Design is derived entirely from generic first principles (a smoothstep opening
characteristic plus a bounded, endpoint-vanishing hysteresis lag). No measured
device curve, product lookup table, or calibration value is used.

Conventions (DATA-03, the fixture-provenance specification):
  * seedless / analytic-closed-form -> output depends only on CONFIG below;
  * fixed decimal quantization (DECIMALS), LF newlines, no trailing whitespace,
    no locale formatting, deterministic row order;
  * self-check of every declared invariant BEFORE writing (aborts non-zero on
    failure, never emits a partially-valid file);
  * after writing, the SHA-256 of each file is recorded into
    ``fixture-provenance.toml`` alongside row count, units, and equation id.

Run (from the fixture directory ``examples/espresso-pump-hybrid``):

    python3 scripts/generate_characteristics.py

Re-running on any host must reproduce byte-identical CSVs; the provenance gate
(``cargo test -p sysml-spec-tests --test fixture_provenance``) recomputes each
SHA-256 and fails on drift.
"""

from __future__ import annotations

import hashlib
import pathlib
import sys

# --- generator identity -----------------------------------------------------
GENERATOR_VERSION = "1.0.0"

# --- checked configuration --------------------------------------------------
# Every number the output depends on lives here, with a first-principles
# rationale (numerical conditioning / visible branch separation / short CI run).
N = 64            # command-grid resolution: u_i = i/(N-1), i in [0, N-1].
A_MIN = 0.02      # floor effective area at zero command (a nearly-shut valve,
                  # kept > 0 so the orifice term stays well-conditioned).
A_MAX = 1.0       # normalized fully-open effective area.
H = 0.64          # hysteresis amplitude. Bounded by monotonicity: the closing
                  # branch stays non-decreasing as long as H <= 3*(A_MAX-A_MIN)
                  # (= 2.94 here); 0.64 gives a clearly-visible peak separation
                  # of H/16 = 0.04 at u = 0.5 while remaining well inside that
                  # bound.
DECIMALS = 6      # fixed fractional digits for byte-stable text.

# --- closed forms (independently derived) -----------------------------------
#   S(u)       = u^2 * (3 - 2u)                 smoothstep, monotone on [0,1]
#   opening(u) = A_MIN + (A_MAX - A_MIN) * S(u)
#   sep(u)     = H * u^2 * (1 - u)^2            endpoint-vanishing hysteresis lag
#   closing(u) = opening(u) - sep(u)            closing branch trails opening
OPENING_EQUATION_ID = "opening(u) = A_min + (A_max-A_min)*u^2*(3-2u)"
CLOSING_EQUATION_ID = "closing(u) = opening(u) - H*u^2*(1-u)^2"


def smoothstep(u: float) -> float:
    return u * u * (3.0 - 2.0 * u)


def opening(u: float) -> float:
    return A_MIN + (A_MAX - A_MIN) * smoothstep(u)


def separation(u: float) -> float:
    return H * (u * u) * ((1.0 - u) ** 2)


def closing(u: float) -> float:
    return opening(u) - separation(u)


def command_grid() -> list[float]:
    return [i / (N - 1) for i in range(N)]


def fmt(x: float) -> str:
    # Fixed-decimal, locale-independent. Normalize -0.000000 to 0.000000.
    s = f"{x:.{DECIMALS}f}"
    if s == "-" + "0." + "0" * DECIMALS:
        s = "0." + "0" * DECIMALS
    return s


def build_rows(fn) -> list[tuple[float, float]]:
    return [(u, fn(u)) for u in command_grid()]


def render_csv(rows: list[tuple[float, float]]) -> str:
    lines = ["u,A_eff"]
    lines.extend(f"{fmt(u)},{fmt(a)}" for u, a in rows)
    # Trailing newline, LF only, no trailing whitespace on any line.
    return "\n".join(lines) + "\n"


# --- self-checks (DATA-03 §3) -----------------------------------------------
def assert_invariants(open_rows, close_rows) -> None:
    us = [u for u, _ in open_rows]

    # Grid endpoints and strict monotonic domain.
    assert us[0] == 0.0, "command grid must start at u=0"
    assert us[-1] == 1.0, "command grid must end at u=1"
    assert all(us[i] < us[i + 1] for i in range(len(us) - 1)), "domain not strictly increasing"

    # Finite, in range.
    for label, rows, hi in (("opening", open_rows, A_MAX), ("closing", close_rows, A_MAX)):
        for u, a in rows:
            assert a == a and abs(a) != float("inf"), f"{label}({u}) not finite"
            assert A_MIN - 1e-12 <= a <= hi + 1e-12, f"{label}({u})={a} out of [{A_MIN},{hi}]"

    # Endpoint values pinned on both branches (separation vanishes at endpoints).
    for rows, label in ((open_rows, "opening"), (close_rows, "closing")):
        assert abs(rows[0][1] - A_MIN) < 1e-12, f"{label}(0) must equal A_MIN"
        assert abs(rows[-1][1] - A_MAX) < 1e-12, f"{label}(1) must equal A_MAX"

    # Monotonic non-decreasing range on BOTH branches.
    for rows, label in ((open_rows, "opening"), (close_rows, "closing")):
        for i in range(len(rows) - 1):
            assert rows[i + 1][1] >= rows[i][1] - 1e-12, f"{label} not monotone at i={i}"

    # Branch separation: opening >= closing everywhere, strictly positive in the
    # interior, exactly zero at both endpoints.
    seps = [o[1] - c[1] for o, c in zip(open_rows, close_rows)]
    assert abs(seps[0]) < 1e-12 and abs(seps[-1]) < 1e-12, "separation must vanish at endpoints"
    assert all(s >= -1e-12 for s in seps), "opening must dominate closing"
    assert max(seps) > 1e-3, "branch separation must be observable"


# --- provenance manifest ----------------------------------------------------
def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def render_manifest(open_bytes: bytes, close_bytes: bytes) -> str:
    cfg = f"N={N}; A_min={A_MIN}; A_max={A_MAX}; H={H}; decimals={DECIMALS}; family=smoothstep"
    gen = f"scripts/generate_characteristics.py@{GENERATOR_VERSION}"
    regen = "python3 scripts/generate_characteristics.py"
    return f"""# fixture-provenance.toml — espresso-pump-hybrid (DATA-01)
#
# Generated by scripts/generate_characteristics.py. Do NOT hand-edit the
# [[data]] sha256/row_count fields; re-run the generator to refresh them.
# Validated by `cargo test -p sysml-spec-tests --test fixture_provenance`.

[fixture]
name          = "espresso-pump-hybrid"
version       = "1.0.0"
created       = "2026-08-02"
design_source = "capability-contract"

[[data]]
path                 = "data/generated_pump_opening.csv"
origin               = "synthetic-equation"
generator            = "{gen}"
generator_config     = "{cfg}"
equation_id          = "{OPENING_EQUATION_ID}"
units                = "u:dimensionless, A_eff:normalized-area"
value_range          = "u in [0,1], A_eff in [{A_MIN}, {A_MAX}]"
row_count            = {N}
sha256               = "{sha256_hex(open_bytes)}"
regeneration_command = "{regen}"

[[data]]
path                 = "data/generated_pump_closing.csv"
origin               = "synthetic-equation"
generator            = "{gen}"
generator_config     = "{cfg}"
equation_id          = "{CLOSING_EQUATION_ID}"
units                = "u:dimensionless, A_eff:normalized-area"
value_range          = "u in [0,1], A_eff in [{A_MIN}, {A_MAX}]"
row_count            = {N}
sha256               = "{sha256_hex(close_bytes)}"
regeneration_command = "{regen}"
"""


def main() -> int:
    fixture_dir = pathlib.Path(__file__).resolve().parent.parent
    data_dir = fixture_dir / "data"
    data_dir.mkdir(exist_ok=True)

    open_rows = build_rows(opening)
    close_rows = build_rows(closing)
    assert_invariants(open_rows, close_rows)

    open_bytes = render_csv(open_rows).encode("utf-8")
    close_bytes = render_csv(close_rows).encode("utf-8")

    (data_dir / "generated_pump_opening.csv").write_bytes(open_bytes)
    (data_dir / "generated_pump_closing.csv").write_bytes(close_bytes)
    (fixture_dir / "fixture-provenance.toml").write_text(
        render_manifest(open_bytes, close_bytes), encoding="utf-8"
    )

    print(f"wrote {len(open_rows)} opening + {len(close_rows)} closing rows")
    print(f"  opening sha256 {sha256_hex(open_bytes)}")
    print(f"  closing sha256 {sha256_hex(close_bytes)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
