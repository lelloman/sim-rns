#!/usr/bin/env python3
"""Real stdio MCP -> live GTK integration. Run under xvfb-run and dbus-run-session."""
import argparse
import base64
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--guest-bundle", type=Path)
    parser.add_argument("--filtered-launch", action="store_true")
    parser.add_argument("--screenshot", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="sim-rns-mcp-") as directory:
        work = Path(directory)
        runtime = work / "runtime"
        runtime.mkdir(mode=0o700)
        env = dict(os.environ, XDG_CONFIG_HOME=str(work / "config"),
                   XDG_CACHE_HOME=str(work / "cache"), XDG_RUNTIME_DIR=str(runtime),
                   SIM_RNS_CONTROL_SOCKET=str(runtime / "control.sock"),
                   GSK_RENDERER="cairo", GTK_A11Y="none")
        bundle = args.guest_bundle.resolve() if args.guest_bundle else work / "fixture-bundle"
        if not args.guest_bundle:
            bundle.mkdir()
            (bundle / "guest.json").write_text(json.dumps(dict(version=1, kernel="vmlinuz", initrd="initrd.gz", disk="base.qcow2")))
            (bundle / "vmlinuz").write_text("fixture kernel")
            (bundle / "initrd.gz").write_text("fixture initramfs")
            subprocess.run(["qemu-img", "create", "-f", "qcow2", bundle / "base.qcow2", "16M"], check=True, stdout=subprocess.DEVNULL)
        with (work / "stderr.log").open("w+") as log:
            mcp_env = env.copy()
            if args.filtered_launch:
                for key in ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR"]:
                    mcp_env.pop(key, None)
            server = subprocess.Popen([root / "target/debug/sim-rns-mcp"], env=mcp_env,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                      stderr=log, text=True, bufsize=1)
            sequence = 0

            def send(message):
                server.stdin.write(json.dumps(dict(jsonrpc="2.0", **message)) + "\n")
                server.stdin.flush()

            def rpc(method, params):
                nonlocal sequence
                sequence += 1
                send(dict(id=sequence, method=method, params=params))
                deadline = time.monotonic() + 180
                while time.monotonic() < deadline:
                    assert select.select([server.stdout], [], [], 1)[0] or server.poll() is None, "MCP exited"
                    if not select.select([server.stdout], [], [], 0)[0]:
                        continue
                    line = server.stdout.readline()
                    assert line, "MCP closed stdout"
                    response = json.loads(line)
                    if response.get("id") == sequence:
                        return response
                raise AssertionError(f"timeout: {method}")

            def tool(tool_name, operation=None, error=False, **kwargs):
                arguments = kwargs if operation is None else dict(operation=operation, **kwargs)
                response = rpc("tools/call", dict(name=tool_name, arguments=arguments))
                assert "error" not in response, response
                result = response["result"]
                assert bool(result.get("isError")) == error, result
                content = result["content"][0]
                if error or content["type"] != "text":
                    return content
                return json.loads(content["text"])

            def widgets():
                def walk(item):
                    yield item
                    for child in item.get("children", []):
                        yield from walk(child)
                return [item for window in tool("ui", "tree")["windows"] for item in walk(window)]

            def close_in_ui():
                tool("project", "close")
                time.sleep(.1)
                switch = next((item for item in widgets() if item.get("label") == "Switch" and item["mapped"]), None)
                if switch:
                    tool("ui", "activate", widget_id=switch["id"])

            def create_in_ui(destination, name, check_invalid=False):
                close_in_ui()
                deadline = time.monotonic() + 5
                while True:
                    button = next((item for item in widgets() if item.get("label") == "Create New Project" and item["mapped"]), None)
                    if button:
                        break
                    assert time.monotonic() < deadline, "launcher did not appear"
                    time.sleep(.1)
                tool("ui", "activate", widget_id=button["id"])
                fields = {item["name"]: item["id"] for item in widgets() if item["name"].startswith("new-project-")}
                tool("ui", "set_text", widget_id=fields["new-project-name"], text=name)
                tool("ui", "set_text", widget_id=fields["new-project-path"], text=str(destination))
                create = next(item for item in widgets() if item.get("label") == "Create Simulation" and item["mapped"])
                if check_invalid:
                    tool("ui", "set_text", widget_id=fields["new-project-bundle"], text=str(work / "missing-bundle"))
                    tool("ui", "activate", widget_id=create["id"])
                    deadline = time.monotonic() + 10
                    while tool("app", "state")["runtime_busy"]:
                        assert time.monotonic() < deadline
                        time.sleep(.1)
                    assert not destination.exists(), "invalid bundle created project files"
                    assert any("Cannot open guest bundle" in item.get("text", "") and item["mapped"] for item in widgets())
                tool("ui", "set_text", widget_id=fields["new-project-bundle"], text=str(bundle))
                tool("ui", "activate", widget_id=create["id"])
                deadline = time.monotonic() + 10
                while True:
                    state = tool("app", "state")
                    if state["project"] and state["project"]["path"] == str(destination):
                        break
                    assert time.monotonic() < deadline, state
                    time.sleep(.1)

            try:
                response = rpc("initialize", dict(protocolVersion="2025-11-25", capabilities={}, clientInfo=dict(name="integration-test", version="1")))
                assert response["result"]["capabilities"]["tools"] is not None
                send(dict(method="notifications/initialized"))
                tools = rpc("tools/list", {})["result"]["tools"]
                assert {item["name"] for item in tools} == {"launch", "app", "project", "runtime", "ui"}
                invalid = rpc("tools/call", dict(name="ui", arguments={"operation": "invalid"}))
                assert "error" in invalid or invalid.get("result", {}).get("isError"), invalid
                tool("app", "state", error=True)  # No app yet: a tool error, not a server crash.
                state = tool("launch")
                assert state["project"] is None
                assert tool("launch")["pid"] == state["pid"]
                time.sleep(0.3)
                project = work / "project"
                tool("project", "create", path=str(project), name="MCP Integration", bundle=str(bundle))
                project = work / "gui-project"
                create_in_ui(project, "MCP Integration", check_invalid=True)
                assert tool("app", "state")["project"]["path"] == str(project)
                assert tool("project", "inspect")["recipe"]["metadata"]["name"] == "MCP Integration"
                time.sleep(0.3)
                tree = widgets()
                window = next(item for item in tree if "commands" in item and item["mapped"])
                assert not any(item.get("text") in ["Workspace", "Activity"] and "tab-label" in item["css_classes"] for item in tree)
                # Invoke the actual tab-header gesture and verify both shell style and page.
                header = next(item for item in tree if "tab-header" in item["css_classes"] and any(c.get("text") == "Recipe" for c in item.get("children", [])))
                tool("ui", "activate", widget_id=header["id"])
                tree = widgets()
                assert "active" in next(item for item in tree if item["id"] == header["id"])["css_classes"]
                assert any(item.get("visible_child") == "custom-workbench-page:recipe" for item in tree)
                content_scrolls = [item for item in tree if item["type"] == "GtkScrolledWindow" and item["mapped"] and item["bounds"]["height"] > 40]
                assert len(content_scrolls) == 1 and content_scrolls[0]["bounds"]["height"] > 300, content_scrolls
                shot = tool("ui", "screenshot", window_id=window["id"])
                assert shot["type"] == "image" and base64.b64decode(shot["data"]).startswith(b"\x89PNG")
                if args.screenshot:
                    args.screenshot.write_bytes(base64.b64decode(shot["data"]))
                # Reorder tabs through the shell's drag controller and exercise its context menu.
                first = next(item for item in tree if "tab-header" in item["css_classes"] and any(c.get("text") == "Overview" for c in item.get("children", [])))
                offset = first["bounds"]["x"] + first["bounds"]["width"] / 4 - header["bounds"]["x"] - 5
                tool("ui", "drag", widget_id=header["id"], start_x=5, start_y=5, offset_x=offset, offset_y=0)
                header = next(item for item in widgets() if "tab-header" in item["css_classes"] and any(c.get("text") in ["Overview", "Recipe", "Templates"] for c in item.get("children", [])))
                assert any(c.get("text") == "Recipe" for c in header.get("children", [])), header
                tool("ui", "context_menu", widget_id=header["id"])
                popup = next(item for item in widgets() if item["type"] in ["GtkPopover", "GtkPopoverMenu"] and item["mapped"])
                tool("ui", "dismiss_popup", widget_id=popup["id"])
                # Open the real shell Settings view and operate its visible controls.
                assert any(item.get("command_id") == "shell.settings" for item in window["commands"]), {key: window.get(key) for key in ["id", "name", "title", "mapped", "commands"]}
                tool("ui", "command", window_id=window["id"], name="shell.settings")
                time.sleep(0.2)
                controls = widgets()
                assert any(item.get("text") == "Settings" and item["mapped"] for item in controls), "Settings view not opened"
                toggle = next((item for item in controls if item["type"] in ["GtkCheckButton", "GtkSwitch"] and item["mapped"] and item["sensitive"]), None)
                if toggle:
                    tool("ui", "set_active", widget_id=toggle["id"], active=not toggle["active"])
                    assert next(item for item in widgets() if item["id"] == toggle["id"])["active"] != toggle["active"]
                # Create a shell buffer and edit its actual GTK text widget.
                command = next(item["name"] for item in window["commands"] if item.get("command_id") == "shell.new_buffer")
                tool("ui", "command", window_id=window["id"], name=command)
                time.sleep(0.2)
                editor = next(item for item in widgets() if item["type"] == "GtkTextView" and item["mapped"] and item["sensitive"])
                tool("ui", "set_text", widget_id=editor["id"], text="hello from MCP")
                assert next(item for item in widgets() if item["id"] == editor["id"])["text"] == "hello from MCP"
                tool("ui", "activate", widget_id=999999, error=True)
                tool("ui", "command", window_id=window["id"], name="missing", error=True)
                tool("project", "add_node")
                tool("project", "add_script")
                source = tool("project", "read_file", path="sim-rns.project.json")
                tool("project", "write_file", path=source["path"], contents="invalid JSON", error=True)
                assert tool("project", "read_file", path=source["path"]) == source
                data = json.loads(source["contents"])
                data["name"] = "Edited through MCP"
                tool("project", "write_file", path=source["path"], contents=json.dumps(data))
                assert tool("project", "inspect")["recipe"]["metadata"]["name"] == "Edited through MCP"
                tool("project", "read_file", path="../outside", error=True)
                tool("project", "read_file", path=".sim-rns/runtime-state.json", error=True)
                assert tool("runtime", "status")["vm_state"] == "stopped"
                tool("runtime", "create_snapshot", name="unsupported", error=True)
                close_in_ui()
                assert tool("app", "state")["project"] is None
                tool("runtime", "status", error=True)
                tool("project", "open", path=str(project))
                if args.guest_bundle:
                    demo = work / "demo"
                    create_in_ui(demo, "Ordinary runnable project")
                    # Submit a long-running boot and a UI request concurrently over MCP.
                    sequence += 1
                    boot_id = sequence
                    send(dict(id=boot_id, method="tools/call", params=dict(name="runtime", arguments=dict(operation="boot"))))
                    time.sleep(0.15)
                    sequence += 1
                    tree_id = sequence
                    send(dict(id=tree_id, method="tools/call", params=dict(name="ui", arguments=dict(operation="tree"))))
                    replies = {}
                    deadline = time.monotonic() + 180
                    while len(replies) < 2:
                        assert time.monotonic() < deadline, "concurrent boot timeout"
                        if not select.select([server.stdout], [], [], 1)[0]:
                            continue
                        response = json.loads(server.stdout.readline())
                        if response.get("id") in (boot_id, tree_id):
                            replies[response["id"]] = response["result"]
                    assert list(replies)[0] == tree_id, "UI blocked behind VM boot"
                    assert not replies[tree_id].get("isError"), replies[tree_id]
                    assert json.loads(replies[boot_id]["content"][0]["text"])["status"]["vm_state"] == "running"
                    source = tool("project", "read_file", path="sim-rns.project.json")
                    tool("project", "write_file", path=source["path"], contents=source["contents"], error=True)
                    deadline = time.monotonic() + 60
                    while True:
                        status = tool("runtime", "status")
                        if all(any("RECEIVED hello" in line for line in status["node_logs"].get(node, [])) for node in ["backbone-a", "phone-a"]):
                            break
                        assert time.monotonic() < deadline, "Reticulum packets not received"
                        time.sleep(1)
                    tool("runtime", "stop_node", element_id="phone-a")
                    assert next(n for n in tool("runtime", "status")["nodes"] if n["element_id"] == "phone-a")["state"] == "stopped"
                    tool("runtime", "start_node", element_id="phone-a")
                    tool("runtime", "restart_node", element_id="phone-a")
                    tool("runtime", "pause")
                    assert tool("runtime", "status")["vm_state"] == "paused"
                    tool("runtime", "resume")
                    tool("runtime", "shutdown")
                    assert tool("runtime", "status")["vm_state"] == "stopped"
                tool("project", "close")
                tool("app", "quit")
                print("PASS: MCP handshake, discovery, errors, launch/attach, GTK tabs/editor/screenshot, project edits/rollback, runtime" + (" and real QEMU guest" if args.guest_bundle else ""))
            except BaseException:
                log.flush()
                log.seek(0)
                print(log.read())
                raise
            finally:
                # Best effort cleanup even when a test fails midway through a VM operation.
                for domain, operation in [("runtime", "shutdown"), ("app", "quit")]:
                    try:
                        with socket.socket(socket.AF_UNIX) as client:
                            client.settimeout(60)
                            client.connect(env["SIM_RNS_CONTROL_SOCKET"])
                            client.sendall((json.dumps(dict(domain=domain, request=dict(operation=operation))) + "\n").encode())
                            client.recv(1024 * 1024)
                    except OSError:
                        pass
                server.stdin.close()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == "__main__":
    main()
