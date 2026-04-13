#!/usr/bin/env python3
"""Cross-platform live smoke checks for CLI, Python SDK, server, and MCP."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import queue
import shutil
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any


REPO = pathlib.Path(__file__).resolve().parents[1]
HIST_SYMBOL = "AAPL"
HIST_START = "20260401"
HIST_END = "20260402"
AT_TIME = "09:30"
AT_TIME_LEGACY = "34200000"
CALENDAR_DATE = "20260406"


def _bin_path(name: str) -> pathlib.Path:
    suffix = ".exe" if os.name == "nt" else ""
    candidates = [
        REPO / "target" / "release" / f"{name}{suffix}",
        REPO / "tools" / "server" / "target" / "release" / f"{name}{suffix}",
        REPO / "tools" / "mcp" / "target" / "release" / f"{name}{suffix}",
    ]
    for path in candidates:
        if path.exists():
            return path
    raise RuntimeError(
        "missing required binary; looked in:\n" + "\n".join(str(path) for path in candidates)
    )


def _columnar_len(value: Any) -> int:
    if isinstance(value, dict):
        for column in value.values():
            if hasattr(column, "__len__"):
                return len(column)
        return 0
    if hasattr(value, "__len__"):
        return len(value)
    raise TypeError(f"unsupported result shape: {type(value)!r}")


def _get_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        sock.listen(1)
        return int(sock.getsockname()[1])


def _run(cmd: list[str], *, env: dict[str, str] | None = None, timeout: int = 60) -> str:
    proc = subprocess.run(
        cmd,
        cwd=REPO,
        env={**os.environ, **(env or {})},
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=timeout,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"command failed ({proc.returncode}): {' '.join(cmd)}\n{proc.stdout.strip()}"
        )
    return proc.stdout


def _wait_http_json(url: str, *, timeout: float = 30.0) -> Any:
    deadline = time.time() + timeout
    last_error: Exception | None = None
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=5) as response:
                body = response.read().decode("utf-8")
                return json.loads(body)
        except (urllib.error.URLError, json.JSONDecodeError) as exc:
            last_error = exc
            time.sleep(0.5)
    raise RuntimeError(f"http check timed out for {url}: {last_error}")


class _LinePump:
    """Background stdout pump for subprocess-driven smoke tests."""

    def __init__(self, proc: subprocess.Popen[str]) -> None:
        self.proc = proc
        self.lines: list[str] = []
        self.queue: "queue.Queue[str]" = queue.Queue()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _run(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            line = line.rstrip("\n")
            self.lines.append(line)
            self.queue.put(line)

    def wait_for_jsonrpc(self, expected_id: int, *, timeout: float = 20.0) -> dict[str, Any]:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                line = self.queue.get(timeout=0.5)
            except queue.Empty:
                if self.proc.poll() is not None:
                    raise RuntimeError(
                        f"process exited before JSON-RPC response {expected_id}: "
                        + "\n".join(self.lines[-40:])
                    )
                continue
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(payload, dict) and payload.get("id") == expected_id:
                return payload
        raise RuntimeError(
            f"timed out waiting for JSON-RPC response {expected_id}:\n"
            + "\n".join(self.lines[-40:])
        )

    def tail(self, n: int = 40) -> str:
        return "\n".join(self.lines[-n:])


def _terminate_process(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=10)


def _smoke_cli(creds: pathlib.Path) -> None:
    tdx = str(_bin_path("tdx"))
    base = [tdx, "--creds", str(creds)]

    out = _run(base + ["calendar", "open_today", "--format", "json"])
    calendar = json.loads(out)
    if not calendar:
        raise RuntimeError("cli calendar_open_today returned no rows")

    out = _run(base + ["stock", "history_eod", HIST_SYMBOL, HIST_START, HIST_END, "--format", "json"])
    eod = json.loads(out)
    if not eod:
        raise RuntimeError("cli stock_history_eod returned no rows")

    out = _run(
        base
        + [
            "stock",
            "at_time_trade",
            HIST_SYMBOL,
            HIST_START,
            HIST_END,
            AT_TIME,
            "--format",
            "json",
        ]
    )
    at_time = json.loads(out)
    if not at_time:
        raise RuntimeError("cli stock_at_time_trade formatted-time returned no rows")

    out = _run(
        base
        + [
            "stock",
            "at_time_trade",
            HIST_SYMBOL,
            HIST_START,
            HIST_END,
            AT_TIME_LEGACY,
            "--format",
            "json",
        ]
    )
    at_time_legacy = json.loads(out)
    if not at_time_legacy:
        raise RuntimeError("cli stock_at_time_trade legacy-ms returned no rows")

    print("cli smoke: ok")


def _smoke_python_sdk(creds: pathlib.Path) -> None:
    try:
        from thetadatadx import Config, Credentials, ThetaDataDx  # type: ignore
    except ImportError as exc:  # pragma: no cover - workflow installs this explicitly
        raise RuntimeError("python SDK is not installed for live smoke") from exc

    client = ThetaDataDx(Credentials.from_file(str(creds)), Config.production())
    try:
        calendar = client.calendar_open_today()
        if _columnar_len(calendar) == 0:
            raise RuntimeError("python calendar_open_today returned no rows")

        eod = client.stock_history_eod(HIST_SYMBOL, HIST_START, HIST_END)
        if _columnar_len(eod) == 0:
            raise RuntimeError("python stock_history_eod returned no rows")

        at_time = client.stock_at_time_trade(HIST_SYMBOL, HIST_START, HIST_END, AT_TIME)
        if _columnar_len(at_time) == 0:
            raise RuntimeError("python stock_at_time_trade formatted-time returned no rows")

        at_time_legacy = client.stock_at_time_trade(
            HIST_SYMBOL, HIST_START, HIST_END, AT_TIME_LEGACY
        )
        if _columnar_len(at_time_legacy) == 0:
            raise RuntimeError("python stock_at_time_trade legacy-ms returned no rows")
    finally:
        client.shutdown()

    print("python sdk smoke: ok")


def _smoke_server(creds: pathlib.Path) -> None:
    server = str(_bin_path("thetadatadx-server"))
    http_port = _get_free_port()
    ws_port = _get_free_port()
    proc = subprocess.Popen(
        [
            server,
            "--creds",
            str(creds),
            "--no-fpss",
            "--http-port",
            str(http_port),
            "--ws-port",
            str(ws_port),
            "--log-level",
            "warn",
        ],
        cwd=REPO,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    pump = _LinePump(proc)
    try:
        status = _wait_http_json(f"http://127.0.0.1:{http_port}/v3/system/status")
        if status.get("status") != "CONNECTED":
            raise RuntimeError(f"unexpected server status payload: {status!r}")

        open_today = _wait_http_json(f"http://127.0.0.1:{http_port}/v3/calendar/open_today")
        if not open_today.get("response"):
            raise RuntimeError("server calendar_open_today returned no rows")

        history_params = urllib.parse.urlencode(
            {"symbol": HIST_SYMBOL, "start_date": HIST_START, "end_date": HIST_END}
        )
        history = _wait_http_json(
            f"http://127.0.0.1:{http_port}/v3/stock/history/eod?{history_params}"
        )
        if not history.get("response"):
            raise RuntimeError("server stock/history/eod returned no rows")

        at_time_params = urllib.parse.urlencode(
            {
                "symbol": HIST_SYMBOL,
                "start_date": HIST_START,
                "end_date": HIST_END,
                "time_of_day": AT_TIME,
            }
        )
        at_time = _wait_http_json(
            f"http://127.0.0.1:{http_port}/v3/stock/at_time/trade?{at_time_params}"
        )
        if not at_time.get("response"):
            raise RuntimeError("server stock/at_time/trade formatted-time returned no rows")

        at_time_legacy_params = urllib.parse.urlencode(
            {
                "symbol": HIST_SYMBOL,
                "start_date": HIST_START,
                "end_date": HIST_END,
                "time_of_day": AT_TIME_LEGACY,
            }
        )
        at_time_legacy = _wait_http_json(
            f"http://127.0.0.1:{http_port}/v3/stock/at_time/trade?{at_time_legacy_params}"
        )
        if not at_time_legacy.get("response"):
            raise RuntimeError("server stock/at_time/trade legacy-ms returned no rows")
    finally:
        _terminate_process(proc)
        if proc.returncode not in (0, -15, 1, None):
            raise RuntimeError(f"server exited unexpectedly:\n{pump.tail()}")

    print("server smoke: ok")


def _smoke_mcp(creds: pathlib.Path) -> None:
    mcp = str(_bin_path("thetadatadx-mcp"))
    proc = subprocess.Popen(
        [mcp, "--creds", str(creds)],
        cwd=REPO,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    pump = _LinePump(proc)
    try:
        assert proc.stdin is not None

        def send(payload: dict[str, Any]) -> None:
            proc.stdin.write(json.dumps(payload) + "\n")
            proc.stdin.flush()

        send(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "live-smoke", "version": "1.0"},
                },
            }
        )
        init = pump.wait_for_jsonrpc(1)
        if init.get("result", {}).get("serverInfo", {}).get("name") != "thetadatadx-mcp":
            raise RuntimeError(f"unexpected MCP initialize response: {init!r}")

        send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        tools = pump.wait_for_jsonrpc(2)
        names = {tool["name"] for tool in tools.get("result", {}).get("tools", [])}
        if "ping" not in names or "calendar_on_date" not in names:
            raise RuntimeError(f"unexpected MCP tools/list response: {tools!r}")

        ping_payload: dict[str, Any] | None = None
        for request_id in range(3, 11):
            send(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "tools/call",
                    "params": {"name": "ping", "arguments": {}},
                }
            )
            ping = pump.wait_for_jsonrpc(request_id)
            ping_payload = json.loads(ping["result"]["content"][0]["text"])
            if ping_payload.get("connected"):
                break
            time.sleep(0.5)
        if not ping_payload or not ping_payload.get("connected"):
            raise RuntimeError(f"mcp never reached connected=true:\n{pump.tail()}")

        send(
            {
                "jsonrpc": "2.0",
                "id": 11,
                "method": "tools/call",
                "params": {
                    "name": "calendar_on_date",
                    "arguments": {"date": CALENDAR_DATE},
                },
            }
        )
        day = pump.wait_for_jsonrpc(11)
        calendar = json.loads(day["result"]["content"][0]["text"])
        if calendar.get("count", 0) <= 0:
            raise RuntimeError(f"unexpected MCP calendar_on_date response: {day!r}")
    finally:
        _terminate_process(proc)
        if proc.returncode not in (0, -15, 1, None):
            raise RuntimeError(f"mcp exited unexpectedly:\n{pump.tail()}")

    print("mcp smoke: ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("creds", help="Path to creds.txt")
    args = parser.parse_args()

    creds = pathlib.Path(args.creds).resolve()
    if not creds.exists():
        raise SystemExit(f"credentials file not found: {creds}")

    _smoke_cli(creds)
    _smoke_python_sdk(creds)
    _smoke_server(creds)
    _smoke_mcp(creds)
    print("live smoke: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
