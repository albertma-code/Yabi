"""JSONL stdio RPC loop.

Protocol (one JSON object per line, UTF-8):

  Request  (stdin)  : {"id": <int|str>, "cmd": "<name>", "args": {...}}
  Response (stdout) : {"id": <id>, "type": "result", "data": {...}}
                    | {"id": <id>, "type": "error",  "error": "<msg>"}
                    | {"id": <id>, "type": "progress", "data": {...}}
  Lifecycle        : {"id": null, "type": "ready"|"log", ...}

Critical: stdout must be line-buffered and never carry non-JSON. All
human-readable logs go to stderr — yt-dlp warnings are silenced via its
own `quiet`/`no_warnings` options at call sites. Writes are serialized via
`_stdout_lock` so progress events fired from yt-dlp's worker thread don't
interleave bytes with the main loop's response writes.
"""
from __future__ import annotations

import json
import sys
import traceback

from . import __version__
from .commands import dispatch
from .io import emit, log_stderr


def main() -> None:
    # Line-buffer stdout so Rust sees every JSON line as soon as it is written.
    # On PyInstaller --onefile this is essential; default block buffering would
    # silently swallow responses until a 4 KiB chunk filled up.
    try:
        sys.stdout.reconfigure(line_buffering=True)  # type: ignore[attr-defined]
    except Exception:
        pass

    emit({"id": None, "type": "ready", "version": __version__})

    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            emit({"id": None, "type": "error", "error": f"bad json: {e}"})
            continue

        rid = req.get("id")
        # Some commands (notably `download`) run on a worker thread and stream
        # progress events; their dispatcher returns immediately. Others run
        # synchronously and yield a single terminal message. The dispatch loop
        # below works for both — synchronous yields complete inside this loop,
        # async ones forward their own messages through `emit()`.
        try:
            for msg in dispatch(req):
                emit(msg)
        except Exception as e:  # never let a bad command kill the loop
            log_stderr(f"unhandled exception for id={rid}: {e}\n{traceback.format_exc()}")
            emit({"id": rid, "type": "error", "error": str(e)})
