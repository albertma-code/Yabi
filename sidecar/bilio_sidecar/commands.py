"""Command dispatch table.

A dispatcher is a generator that yields 0..N messages per request.
Commands that run on a worker thread (`download`) spawn the thread and yield
a single terminal result; the worker thread pushes progress via `emit()`.
"""
from __future__ import annotations

import os
import threading
import time
from typing import Any, Iterator

from .io import emit, log_stderr
from .ytdlp import CancelRequested, check_cookies, download_video, parse_url

# Active downloads keyed by request id.
_active_jobs: dict[Any, threading.Event] = {}


def dispatch(req: dict[str, Any]) -> Iterator[dict[str, Any]]:
    cmd = req.get("cmd")
    rid = req.get("id")
    args = req.get("args") or {}

    if cmd == "ping":
        yield {"id": rid, "type": "pong", "ts": time.time()}
        return

    if cmd == "check_cookies":
        cookies_from_browser = args.get("cookies_from_browser")
        status = check_cookies(cookies_from_browser)
        yield {"id": rid, "type": "result", "data": status}
        return

    if cmd == "parse":
        url = args.get("url")
        cookies_from_browser = args.get("cookies_from_browser")
        if not url or not isinstance(url, str):
            yield {"id": rid, "type": "error", "error": "missing or invalid 'url'"}
            return

        # Acknowledge the request synchronously so the frontend can subscribe
        # to `parse_progress` events before the sidecar starts the HTTP work
        # that produces them. Without this, the listener might miss the first
        # few entries on a fast-resolving small playlist.
        emit({"id": rid, "type": "parse_started"})

        # Per-entry callback: emit a `parse_progress` event so the UI can
        # update the entry table incrementally. Uses the same id as the
        # eventual `result`; the Rust shell forwards non-terminal types
        # (anything that isn't `result`/`error`) to the frontend event bus
        # without consuming the request's pending oneshot.
        def _on_entry(index: int, total: int, entry: dict[str, Any]) -> None:
            emit({
                "id": rid,
                "type": "parse_progress",
                "data": {
                    "index": index,
                    "total": total,
                    "entry": entry,
                },
            })

        info = parse_url(
            url,
            cookies_from_browser=cookies_from_browser,
            on_entry=_on_entry,
        )
        yield {"id": rid, "type": "result", "data": info}
        return

    if cmd == "download":
        url = args.get("url")
        format_id = args.get("format_id")
        output_dir = os.path.expanduser(args.get("output_dir") or "~/Downloads/Bilio")
        cookies_from_browser = args.get("cookies_from_browser")
        if not url or not isinstance(url, str):
            yield {"id": rid, "type": "error", "error": "missing or invalid 'url'"}
            return
        if not format_id or not isinstance(format_id, str):
            yield {"id": rid, "type": "error", "error": "missing 'format_id'"}
            return

        cancel_ev = threading.Event()
        _active_jobs[rid] = cancel_ev

        # Yield a started acknowledgment immediately so Rust knows the job
        # was accepted; actual progress is streamed via emit() from the thread.
        yield {"id": rid, "type": "result", "data": {"status": "started"}}

        def _worker():
            try:
                final_path = download_video(
                    url=url,
                    format_id=format_id,
                    output_dir=output_dir,
                    progress_callback=_make_progress_cb(rid),
                    cancel_event=cancel_ev,
                    cookies_from_browser=cookies_from_browser,
                )
                emit({
                    "id": rid,
                    "type": "result",
                    "data": {"status": "completed", "filepath": final_path},
                })
            except CancelRequested:
                emit({"id": rid, "type": "result", "data": {"status": "cancelled"}})
            except Exception as e:
                log_stderr(f"download error id={rid}: {e}")
                emit({"id": rid, "type": "error", "error": str(e)})
            finally:
                _active_jobs.pop(rid, None)

        threading.Thread(target=_worker, daemon=True).start()
        return

    if cmd == "cancel":
        rid_to_cancel = args.get("job_id")
        if rid_to_cancel and rid_to_cancel in _active_jobs:
            _active_jobs[rid_to_cancel].set()
            yield {"id": rid, "type": "result", "data": {"cancelled": rid_to_cancel}}
        else:
            yield {"id": rid, "type": "error", "error": f"no active job {rid_to_cancel!r}"}
        return

    yield {"id": rid, "type": "error", "error": f"unknown cmd: {cmd!r}"}


def _make_progress_cb(rid: Any):
    """Return a callable suitable as a yt-dlp progress_hook."""

    def cb(d: dict[str, Any]) -> None:
        emit({
            "id": rid,
            "type": "progress",
            "data": {
                "status": d.get("status"),
                "downloaded_bytes": d.get("downloaded_bytes"),
                "total_bytes": d.get("total_bytes") or d.get("total_bytes_estimate"),
                "speed": d.get("speed"),
                "eta": d.get("eta"),
                "filename": os.path.basename(d.get("filename", "") or ""),
                "percent": d.get("_percent_str"),
            },
        })

    return cb