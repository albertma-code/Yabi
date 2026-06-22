"""Bilio Python sidecar package.

Stdio JSONL RPC layer wrapping yt-dlp. Spawned once by the Tauri Rust shell
and kept alive for the application lifetime; each line on stdin is a request,
each line on stdout is a response or event. See sidecar/bilio_sidecar/rpc.py.
"""

__version__ = "0.1.0"
