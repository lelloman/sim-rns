#!/usr/bin/env python3
"""Check MCP startup failures and deadlines without a desktop or QEMU."""
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix="sim-rns-launch-") as directory:
    work = Path(directory)
    binary = work / "sim-rns-mcp"
    shutil.copyfile(root / "target/debug/sim-rns-mcp", binary)
    binary.chmod(0o700)
    for body, expected, limit in [
        ("import sys\nprint('test display connection failed', file=sys.stderr)\nsys.exit(27)\n", "test display connection failed", 5),
        ("import time\ntime.sleep(60)\n", "startup timed out", 20),
    ]:
        app = work / "sim-rns-app"
        app.write_text("#!/usr/bin/python3\n" + body)
        app.chmod(0o700)
        env = dict(os.environ, DISPLAY=":test", SIM_RNS_CONTROL_SOCKET=str(work / "control.sock"))
        server = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, env=env)
        try:
            def send(message):
                server.stdin.write(json.dumps(dict(jsonrpc="2.0", **message)) + "\n")
                server.stdin.flush()

            def receive(expected_id, timeout):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    if select.select([server.stdout], [], [], .1)[0]:
                        response = json.loads(server.stdout.readline())
                        if response.get("id") == expected_id:
                            return response
                raise AssertionError("MCP response deadline exceeded")

            send(dict(id=1, method="initialize", params=dict(protocolVersion="2025-11-25", capabilities={}, clientInfo=dict(name="launch-test", version="1"))))
            receive(1, 5)
            send(dict(method="notifications/initialized"))
            start = time.monotonic()
            send(dict(id=2, method="tools/call", params=dict(name="launch", arguments={})))
            result = receive(2, limit)["result"]
            assert result["isError"] and expected in result["content"][0]["text"], result
            assert time.monotonic() - start < limit
        finally:
            server.stdin.close()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
print("PASS: immediate startup diagnostics and bounded launch timeout")
