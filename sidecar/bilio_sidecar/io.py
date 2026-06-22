"""Thread-safe stdio helpers.

Split out from rpc.py so command handlers can write progress events from
worker threads without creating a circular import (rpc imports commands;
commands needs emit).
"""
from __future__ import annotations

import json
import sys
import threading
from typing import Any

_stdout_lock = threading.Lock()


def emit(obj: dict[str, Any]) -> None:
    """Write one JSON line to stdout. Thread-safe — usable from yt-dlp
    progress hooks running on the download worker thread."""
    line = json.dumps(obj, ensure_ascii=False) + "\n"
    with _stdout_lock:
        sys.stdout.write(line)
        sys.stdout.flush()


def log_stderr(msg: str) -> None:
    sys.stderr.write(f"[bilio-sidecar] {msg}\n")
    sys.stderr.flush()
