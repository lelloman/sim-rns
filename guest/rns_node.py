"""Minimal real Reticulum node, with a plaintext diagnostic packet exchange."""
import os
from pathlib import Path
import time
print(f"ISOLATION uid={os.getuid()} netns={os.readlink('/proc/self/ns/net')}", flush=True)
import RNS

home = Path.home()
config = home / "reticulum"
config.mkdir(exist_ok=True)
interfaces = []
for index, address in enumerate(os.environ.get("SIM_ADDRESSES", "").split()):
    broadcast = address.rsplit(".", 1)[0] + ".255"
    interfaces.append(f"""  [[lan-{index}]]
    type = UDPInterface
    enabled = yes
    listen_ip = 0.0.0.0
    listen_port = {4242 + index}
    forward_ip = {broadcast}
    forward_port = {4242 + index}
""")
(config / "config").write_text("[reticulum]\n  enable_transport = yes\n  share_instance = no\n[logging]\n  loglevel = 4\n[interfaces]\n" + "".join(interfaces))
reticulum = RNS.Reticulum(configdir=str(config))
identity_path = home / "identity"
identity = RNS.Identity.from_file(str(identity_path)) if identity_path.exists() else RNS.Identity()
if not identity_path.exists():
    identity.to_file(str(identity_path))
print("BOOT " + Path("/proc/sys/kernel/random/boot_id").read_text().strip(), flush=True)
print(f"IDENTITY {identity.hash.hex()}", flush=True)
incoming = RNS.Destination(None, RNS.Destination.IN, RNS.Destination.PLAIN, "simrns", "diagnostic")
incoming.set_packet_callback(lambda data, packet: print("RECEIVED " + data.decode(errors="replace"), flush=True))
outgoing = RNS.Destination(None, RNS.Destination.OUT, RNS.Destination.PLAIN, "simrns", "diagnostic")
while True:
    RNS.Packet(outgoing, f"hello from {os.environ['SIM_ELEMENT_ID']} at {time.time_ns()}".encode()).send()
    time.sleep(2)
