"""Background rocm-smi power sampler — produces J/req inputs for harnesses.

Public surface:
    PowerCapture(interval_s=0.5)  # context manager
        .start() / .stop()
        .snapshot() -> dict   # summary safe to serialize into a JSON manifest

Sampling strategy: subprocess to `rocm-smi -P --json` on a fixed cadence
in a background thread; per-call socket power is summed across all
detected cards. Energy is computed by rectangular integration over the
actual sample timestamps (not the requested cadence, which drifts under
load).

When rocm-smi is unavailable or every call fails, snapshot returns a
sentinel dict with `present: False` and NaN totals — harnesses can write
that into the manifest unconditionally.
"""

from __future__ import annotations

import json
import math
import shutil
import subprocess
import threading
import time
from dataclasses import dataclass, field

ROCM_SMI_BIN = "rocm-smi"
DEFAULT_INTERVAL_S = 0.5
POWER_KEY = "Current Socket Graphics Package Power (W)"


@dataclass
class _Sample:
    monotonic_s: float
    per_card_w: dict[str, float]

    @property
    def total_w(self) -> float:
        return sum(self.per_card_w.values())


@dataclass
class _State:
    samples: list[_Sample] = field(default_factory=list)
    failures: int = 0


def _one_sample() -> dict[str, float] | None:
    """Single rocm-smi call. Returns per-card watts dict, or None on
    any failure (missing binary, non-zero exit, JSON parse error,
    unexpected schema)."""
    try:
        proc = subprocess.run(
            [ROCM_SMI_BIN, "-P", "--json"],
            capture_output=True,
            text=True,
            timeout=2.0,
            check=False,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    try:
        parsed = json.loads(proc.stdout)
    except (json.JSONDecodeError, ValueError):
        return None
    out: dict[str, float] = {}
    for card_name, fields in parsed.items():
        if not isinstance(fields, dict):
            continue
        raw = fields.get(POWER_KEY)
        if raw is None:
            continue
        try:
            out[card_name] = float(raw)
        except (TypeError, ValueError):
            continue
    return out or None


class PowerCapture:
    """Context manager that polls rocm-smi in a background thread."""

    def __init__(self, interval_s: float = DEFAULT_INTERVAL_S):
        if interval_s <= 0:
            raise ValueError("interval_s must be positive")
        self.interval_s = interval_s
        self._state = _State()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._available = shutil.which(ROCM_SMI_BIN) is not None

    def __enter__(self) -> "PowerCapture":
        self.start()
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.stop()

    def start(self) -> None:
        if self._thread is not None:
            return
        if not self._available:
            return
        self._thread = threading.Thread(
            target=self._run, name="rocm-smi-power-sampler", daemon=True
        )
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=5.0)
            self._thread = None

    def _run(self) -> None:
        while not self._stop.is_set():
            sample = _one_sample()
            if sample is None:
                self._state.failures += 1
            else:
                self._state.samples.append(
                    _Sample(monotonic_s=time.monotonic(), per_card_w=sample)
                )
            self._stop.wait(self.interval_s)

    def snapshot(self) -> dict:
        """Summary safe to drop straight into a JSON manifest. Always
        returns a dict; never raises. When rocm-smi is unavailable or
        every sample failed, totals are NaN and `present` is False."""
        samples = self._state.samples
        if not self._available or not samples:
            return {
                "present": False,
                "rocm_smi_available": self._available,
                "interval_s_requested": self.interval_s,
                "sample_count": 0,
                "failure_count": self._state.failures,
                "duration_s": 0.0,
                "energy_j_total": float("nan"),
                "mean_power_w_total": float("nan"),
                "peak_power_w_total": float("nan"),
                "per_card_peak_w": {},
            }
        totals = [s.total_w for s in samples]
        peak = max(totals)
        mean = sum(totals) / len(totals)
        duration = samples[-1].monotonic_s - samples[0].monotonic_s
        energy = 0.0
        for prev, cur in zip(samples, samples[1:]):
            dt = cur.monotonic_s - prev.monotonic_s
            energy += 0.5 * (prev.total_w + cur.total_w) * dt
        per_card_peak: dict[str, float] = {}
        for s in samples:
            for card, w in s.per_card_w.items():
                if not math.isfinite(w):
                    continue
                if w > per_card_peak.get(card, float("-inf")):
                    per_card_peak[card] = w
        return {
            "present": True,
            "rocm_smi_available": True,
            "interval_s_requested": self.interval_s,
            "sample_count": len(samples),
            "failure_count": self._state.failures,
            "duration_s": duration,
            "energy_j_total": energy,
            "mean_power_w_total": mean,
            "peak_power_w_total": peak,
            "per_card_peak_w": per_card_peak,
        }

    def energy_per_request_j(self, request_count: int) -> float:
        """J/req convenience — NaN when no samples or request_count <= 0."""
        snap = self.snapshot()
        if not snap["present"] or request_count <= 0:
            return float("nan")
        return snap["energy_j_total"] / request_count


if __name__ == "__main__":
    import argparse

    p = argparse.ArgumentParser(description="rocm-smi power capture smoke")
    p.add_argument("--duration-s", type=float, default=3.0)
    p.add_argument("--interval-s", type=float, default=DEFAULT_INTERVAL_S)
    args = p.parse_args()

    with PowerCapture(interval_s=args.interval_s) as cap:
        time.sleep(args.duration_s)
    print(json.dumps(cap.snapshot(), indent=2))
