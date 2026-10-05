"""Simulator guest control plane. Runs as guest root; node processes do not."""
import json
import os
from pathlib import Path
import resource
import selectors
import signal
import subprocess
import sys
import termios
import time
import tty
import venv

DATA = Path("/var/lib/sim-rns")
SUPPORTED = {"network.lan", "reticulum.python.backbone", "script.python", "script.bash"}


def run(*argv):
    subprocess.run(argv, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def node_exec(uid, home, command, environment, memory_mb):
    os.setgroups([])
    os.setgid(uid)
    os.setuid(uid)
    os.chdir(home)
    if memory_mb:
        limit = memory_mb * 1024 * 1024
        resource.setrlimit(resource.RLIMIT_AS, (limit, limit))
    os.execvpe(command[0], command, environment)


class Backend:
    def __init__(self):
        self.recipe = None
        self.nodes = {}
        self.children = {}
        self.stopped = set()
        self.addresses = {}
        self.restart_after = {}
        self.failures = {}
        if (DATA / "recipe.json").exists():
            self.initialize(json.loads((DATA / "recipe.json").read_text()), {}, restoring=True)

    def initialize(self, recipe, assets, restoring=False):
        if self.recipe is not None:
            if self.recipe != recipe:
                raise ValueError("guest already initialized; recipe edits require a fresh project")
            return
        for element in recipe["elements"]:
            if any(asset["mode"] != "copy" for asset in element["assets"]):
                raise ValueError("guest asset template rendering is not implemented; use copy seeds")
            if element["template_id"] not in SUPPORTED:
                raise ValueError(f"unsupported guest template: {element['template_id']}")
            if not element["id"] or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_." for c in element["id"]) or element["id"] in {".", ".."}:
                raise ValueError("invalid element ID")
        if len(recipe["elements"]) > 200:
            raise ValueError("this guest supports at most 200 elements")
        if len({e["id"] for e in recipe["elements"]}) != len(recipe["elements"]):
            raise ValueError("duplicate element IDs")
        templates = {t["id"]: t for t in recipe["templates"]}
        for index, element in enumerate(recipe["elements"]):
            element = dict(element)
            element["index"] = index
            element["uid"] = 1000 + index
            self.nodes[element["id"]] = element
            home = DATA / "nodes" / element["id"]
            home.mkdir(parents=True, exist_ok=True)
            home.chmod(0o700)
            if not restoring:
                for asset in element["assets"]:
                    destination = Path(asset["destination"])
                    if destination.is_absolute() or ".." in destination.parts:
                        raise ValueError("invalid asset destination")
                    target = home / destination
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(bytes(assets[f"{element['id']}/{asset['destination']}"]))
            if not (home / "venv").exists():
                venv.EnvBuilder(with_pip=False, system_site_packages=True).create(home / "venv")
            for path in [home, *home.rglob("*")]:
                if not path.is_symlink():
                    os.chown(path, element["uid"], element["uid"])
            defaults = templates[element["template_id"]]["defaults"]
            element["command"] = element["command_override"] or defaults["command"]
            if element["template_id"] == "reticulum.python.backbone" and not element["command_override"]:
                element["command"] = [str(home / "venv/bin/python3"), "/opt/sim-rns/rns_node.py"]
            element["environment"] = dict(defaults["env"], **element["env"])
            element["policy"] = element["restart_policy"] or defaults["restart_policy"]
            limits = element["resources"] or defaults["resources"]
            element["memory"] = limits["memory_mb"]
            cgroup = Path(f"/sys/fs/cgroup/sim-rns-{element['uid']}")
            cgroup.mkdir(exist_ok=True)
            (cgroup / "memory.max").write_text(str(element["memory"] * 1024 * 1024) if element["memory"] else "max")
            (cgroup / "cpu.weight").write_text(str(limits["cpu_weight"] or 100))
            self.addresses[element["id"]] = []
            if element["enabled"] and element["template_id"] != "network.lan":
                run("ip", "netns", "add", f"sr{index}")
                run("ip", "-n", f"sr{index}", "link", "set", "lo", "up")
        networks = [e for e in self.nodes.values() if e["template_id"] == "network.lan" and e["enabled"]]
        for number, network in enumerate(networks, 1):
            bridge = f"br{network['index']}"
            run("ip", "link", "add", bridge, "type", "bridge")
            run("ip", "link", "set", bridge, "up")
            for link in recipe["topology"]["attachments"]:
                if link["network_id"] != network["id"]:
                    continue
                element = self.nodes[link["element_id"]]
                if not element["enabled"]:
                    continue
                index = element["index"]
                host, guest = f"v{index}n{number}", f"eth{number}"
                ns = f"sr{index}"
                run("ip", "link", "add", host, "type", "veth", "peer", "name", guest, "netns", ns)
                run("ip", "link", "set", host, "master", bridge)
                run("ip", "link", "set", host, "up")
                address = f"10.77.{number}.{index + 2}"
                run("ip", "-n", ns, "addr", "add", address + "/24", "broadcast", "+", "dev", guest)
                run("ip", "-n", ns, "link", "set", guest, "up")
                self.addresses[element["id"]].append(address)
        self.recipe = recipe
        if not restoring:
            temporary = DATA / "recipe.pending"
            with temporary.open("w") as file:
                json.dump(recipe, file)
                file.flush()
                os.fsync(file.fileno())
            temporary.replace(DATA / "recipe.json")
        for element_id in recipe["startup"]["order"]:
            if self.nodes[element_id]["enabled"]:
                self.start(element_id)

    def start(self, element_id):
        element = self.nodes[element_id]
        if not element["enabled"]:
            raise ValueError("node is disabled")
        if element["template_id"] == "network.lan":
            return
        if element_id in self.children and self.children[element_id].poll() is None:
            return
        self.stopped.discard(element_id)
        self.restart_after.pop(element_id, None)
        home = DATA / "nodes" / element_id
        environment = dict(element["environment"], HOME=str(home), PATH=f"{home}/venv/bin:/usr/bin:/bin", SIM_ELEMENT_ID=element_id, SIM_ADDRESSES=" ".join(self.addresses[element_id]), PYTHONUNBUFFERED="1")
        with (home / "node.log").open("ab") as log:
            self.children[element_id] = subprocess.Popen([
                "ip", "netns", "exec", f"sr{element['index']}", "/usr/bin/python3", __file__,
                "--node", str(element["uid"]), str(home), json.dumps(element["command"]), json.dumps(environment), str(element["memory"])
            ], stdout=log, stderr=log, stdin=subprocess.DEVNULL, start_new_session=True,
                preexec_fn=lambda: Path(f"/sys/fs/cgroup/sim-rns-{element['uid']}/cgroup.procs").write_text(str(os.getpid())))

    def stop(self, element_id):
        if self.nodes[element_id]["template_id"] == "network.lan":
            raise ValueError("network lifecycle control is not implemented")
        self.stopped.add(element_id)
        child = self.children.get(element_id)
        if child and child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
            try:
                child.wait(timeout=1)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()

    def supervise(self):
        for element_id, child in list(self.children.items()):
            code = child.poll()
            policy = self.nodes[element_id]["policy"]
            if code is not None and element_id not in self.stopped and (policy == "always" or (policy == "on_failure" and code != 0)):
                if element_id not in self.restart_after:
                    count = self.failures.get(element_id, 0) + 1
                    self.failures[element_id] = count
                    self.restart_after[element_id] = time.monotonic() + min(30, 2 ** min(count, 5))
                elif time.monotonic() >= self.restart_after[element_id]:
                    self.start(element_id)

    def status(self):
        nodes, logs = [], {}
        for element_id, element in self.nodes.items():
            child = self.children.get(element_id)
            running = element["template_id"] == "network.lan" or (child is not None and child.poll() is None)
            nodes.append(dict(element_id=element_id, template_id=element["template_id"], enabled=element["enabled"], state="disabled" if not element["enabled"] else "running" if running else "failed" if child is not None and child.returncode and element_id not in self.stopped else "stopped"))
            log = DATA / "nodes" / element_id / "node.log"
            if log.exists():
                with log.open("rb") as file:
                    file.seek(max(0, log.stat().st_size - 4096))
                    logs[element_id] = file.read(4096).decode(errors="replace").splitlines()[-12:]
        return dict(nodes=nodes, logs=logs, topology=self.recipe["topology"]["attachments"] if self.recipe else [])

    def dispatch(self, command):
        op = command["op"]
        if op == "initialize":
            self.initialize(command["recipe"], command["assets"])
        elif op == "status":
            pass
        elif op == "shutdown":
            for element_id, element in self.nodes.items():
                if element["template_id"] != "network.lan":
                    self.stop(element_id)
            os.sync()
        elif op == "start_node":
            self.start(command["element_id"])
        elif op == "stop_node":
            self.stop(command["element_id"])
        elif op == "restart_node":
            self.stop(command["element_id"])
            self.start(command["element_id"])
        else:
            raise ValueError(f"unsupported guest command: {op}")
        return self.status()


def main():
    backend = Backend()
    fd = os.open("/dev/ttyS1", os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    tty.setraw(fd)
    selector = selectors.DefaultSelector()
    selector.register(fd, selectors.EVENT_READ)
    pending = bytearray()
    print("sim-rns guest backend ready", flush=True)
    while True:
        backend.supervise()
        if not selector.select(timeout=0.5):
            continue
        pending.extend(os.read(fd, 65536))
        if len(pending) > 16 * 1024 * 1024:
            pending.clear()
            continue
        while b"\n" in pending:
            line, _, pending = pending.partition(b"\n")
            request = {}
            try:
                request = json.loads(line)
                if request["version"] != 1:
                    raise ValueError("unsupported protocol version")
                response = dict(version=1, id=request.get("id"), status=backend.dispatch(request["command"]))
            except Exception as error:
                response = dict(version=1, id=request.get("id"), error=str(error))
                print(f"guest command failed: {error}", flush=True)
            payload = (json.dumps(response) + "\n").encode()
            # Serial output can be partial; keep the complete response framed.
            os.set_blocking(fd, True)
            while payload:
                payload = payload[os.write(fd, payload):]
            os.set_blocking(fd, False)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--node":
        node_exec(int(sys.argv[2]), sys.argv[3], json.loads(sys.argv[4]), json.loads(sys.argv[5]), int(sys.argv[6]))
    else:
        main()
