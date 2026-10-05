#!/usr/bin/env python3
"""Build a local Linux/Python/RNS guest bundle without mounting or changing the host."""
import argparse
import gzip
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sysconfig
import tempfile


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def build(kernel, version, output):
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="sim-rns-build-") as temporary:
        root = Path(temporary) / "root"
        root.mkdir()
        copied = set()

        def copy_binary(source, destination=None):
            source = Path(source)
            dest = root / str(destination or source).lstrip("/")
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, dest)
            key = str(source.resolve())
            if key in copied:
                return
            copied.add(key)
            deps = subprocess.run(["ldd", str(source)], text=True, capture_output=True)
            for dependency in re.findall(r"(/[^\s()]+)", deps.stdout):
                if Path(dependency).is_file():
                    copy_binary(dependency)

        for directory in ["dev", "proc", "sys", "run", "tmp", "var/lib/sim-rns", "etc", "opt/sim-rns", "bin", "sbin"]:
            (root / directory).mkdir(parents=True, exist_ok=True)
        (root / "tmp").chmod(0o1777)
        copy_binary(shutil.which("busybox"), "/bin/busybox")
        for applet in ["sh", "mount", "mkdir", "sleep", "poweroff", "chmod"]:
            (root / "bin" / applet).symlink_to("busybox")
        copy_binary("/usr/bin/python3", "/usr/bin/python3")
        copy_binary("/bin/bash", "/bin/bash")
        copy_binary(shutil.which("ip"), "/usr/sbin/ip")
        copy_binary(shutil.which("kmod"), "/bin/kmod")
        (root / "sbin/modprobe").symlink_to("/bin/kmod")
        stdlib = Path(sysconfig.get_path("stdlib"))
        shutil.copytree(stdlib, root / str(stdlib).lstrip("/"), ignore=shutil.ignore_patterns("__pycache__", "test", "tests", "site-packages", "dist-packages"))
        # Include the installed packages and their licenses; this builder does not download code.
        for package in ["RNS", "serial", "cryptography", "_cffi_backend"]:
            spec = importlib.util.find_spec(package)
            if spec is None:
                raise RuntimeError(f"Install {package} for /usr/bin/python3 before building")
            source = Path(spec.origin)
            destination = root / "usr/lib/python3/dist-packages"
            destination.mkdir(parents=True, exist_ok=True)
            if source.name == "__init__.py":
                shutil.copytree(source.parent, destination / source.parent.name, ignore=shutil.ignore_patterns("__pycache__"))
            else:
                shutil.copy2(source, destination / source.name)
        for binary in list(root.rglob("*.so")):
            deps = subprocess.run(["ldd", str(binary)], text=True, capture_output=True)
            for dependency in re.findall(r"(/[^\s()]+)", deps.stdout):
                if Path(dependency).is_file():
                    copy_binary(dependency)
        # OpenSSL loads providers dynamically; ldd cannot discover these modules.
        for provider in Path("/usr/lib").glob("*/ossl-modules/*.so"):
            copy_binary(provider)
        ssl_config = Path("/etc/ssl/openssl.cnf")
        if ssl_config.exists():
            destination = root / "etc/ssl/openssl.cnf"
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ssl_config, destination)
        modules = ["virtio_pci", "virtio_blk", "ext4", "veth", "bridge"]
        for module in modules:
            deps = subprocess.check_output(["modprobe", "--show-depends", "-S", version, module], text=True)
            for line in deps.splitlines():
                if line.startswith("insmod "):
                    source = Path(line.split()[1])
                    dest = root / str(source).lstrip("/")
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, dest)
        module_root = root / "lib/modules" / version
        module_root.mkdir(parents=True, exist_ok=True)
        for name in ["modules.builtin", "modules.builtin.modinfo", "modules.order"]:
            source = Path("/lib/modules") / version / name
            if source.exists():
                shutil.copy2(source, module_root / name)
        run("depmod", "-b", str(root), version)
        here = Path(__file__).resolve().parent
        shutil.copy2(here / "agent.py", root / "opt/sim-rns/agent.py")
        shutil.copy2(here / "rns_node.py", root / "opt/sim-rns/rns_node.py")
        (root / "etc/passwd").write_text("root:x:0:0:root:/root:/bin/sh\n")
        (root / "etc/group").write_text("root:x:0:\n")
        (root / "init").write_text("""#!/bin/sh
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev
mount -t tmpfs tmpfs /run
mkdir -p /sys/fs/cgroup
mount -t cgroup2 cgroup2 /sys/fs/cgroup
printf '+cpu +memory' > /sys/fs/cgroup/cgroup.subtree_control
for module in virtio_pci virtio_blk ext4 veth bridge; do /sbin/modprobe "$module" || exit 1; done
for attempt in 1 2 3 4 5; do [ -b /dev/vda ] && break; sleep 1; done
mount -t ext4 /dev/vda /var/lib/sim-rns || exit 1
exec /usr/bin/python3 -u /opt/sim-rns/agent.py
""")
        (root / "init").chmod(0o755)
        # newc supports unprivileged creation; archive all entries as guest root.
        files = ["."] + sorted(str(p.relative_to(root)) for p in root.rglob("*"))
        archive = subprocess.run(["cpio", "--null", "-o", "--format=newc", "--owner=0:0"], cwd=root, input=("\0".join(files) + "\0").encode(), stdout=subprocess.PIPE, check=True).stdout
        with gzip.open(output / "initrd.gz", "wb", compresslevel=6) as compressed:
            compressed.write(archive)
        shutil.copy2(kernel, output / "vmlinuz")
        raw = Path(temporary) / "data.raw"
        with raw.open("wb") as disk:
            disk.truncate(256 * 1024 * 1024)
        run("mkfs.ext4", "-q", "-F", "-L", "sim-rns-data", str(raw))
        run("qemu-img", "convert", "-f", "raw", "-O", "qcow2", str(raw), str(output / "base.qcow2"))
        (output / "guest.json").write_text(json.dumps({"version": 1, "kernel": "vmlinuz", "initrd": "initrd.gz", "disk": "base.qcow2", "kernel_version": version}, indent=2))
    print(f"Guest bundle: {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", type=Path, required=True)
    parser.add_argument("--kernel-version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    build(args.kernel.resolve(), args.kernel_version, args.output.resolve())
