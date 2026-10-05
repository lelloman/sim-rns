# Sim RNS

A local Reticulum network simulator hosted in the Maruzzella GTK shell. Each project has one QEMU VM. The current guest implementation supports Python Reticulum nodes and Python/Bash scripts in separate Linux network namespaces, with separate users, homes, virtual environments, and cgroup CPU/memory limits.

## Build and checks

The workspace currently uses a sibling `../maruzzella` checkout (tested with 0.1.2), GTK 4.10 or newer, a current Rust toolchain, and Linux. Install QEMU (`qemu-system-x86_64`, `qemu-img`) for runtime use.

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt -p sim-rns-app -p sim-rns-core -p sim-rns-plugin -p sim-rns-control -p sim-rns-mcp -- --check
```

Tests use local Unix sockets. The explicit QEMU lifecycle test needs QEMU but no OS image:

```sh
cargo test -p sim-rns-core real_qemu_lifecycle -- --ignored --nocapture
```

## MCP control

The `sim-rns-mcp` binary provides a stdio MCP server for the running GTK app. Its tools can launch the app, manage projects and source files, control the VM and individual nodes, inspect logs, operate the UI, and capture window screenshots. See [MCP setup and tool reference](docs/mcp.md) for client configuration, examples, limitations, and integration tests.

```json
{
  "mcpServers": {
    "sim-rns": {
      "command": "/absolute/path/to/sim-rns/target/debug/sim-rns-mcp"
    }
  }
}
```

Build the workspace first. From the MCP client, call `launch` to open the app (or attach to an existing instance), then use `app` with `{"operation":"state"}` and `ui` with `{"operation":"tree"}`. Launch can recover the desktop environment from a running session owned by the same user.

## Build a guest bundle

`guest/build.py` builds an initramfs and a persistent ext4 data disk without mounting anything or changing host configuration. It copies an installed Python environment and the selected kernel's modules. It currently targets an x86-64 Debian/Ubuntu host.

Requirements: `/usr/bin/python3` with `RNS`, `serial`, `cryptography`, and `_cffi_backend` importable; `busybox` (static), `bash`, `ip`, `kmod`, `depmod`, `cpio`, `mkfs.ext4`, `ldd`, and `qemu-img`. Supply a **readable kernel and exactly matching installed modules**. If `/boot` is restricted, a matching distribution kernel package can be extracted into a user-owned temporary directory instead of changing `/boot` permissions.

```sh
python3 guest/build.py \
  --kernel /path/to/readable/vmlinuz \
  --kernel-version YOUR_INSTALLED_KERNEL_VERSION \
  --output /path/to/new/guest-bundle
```

The output directory must not already exist. The bundle includes `guest.json`, `vmlinuz`, `initrd.gz`, and `base.qcow2`. Rebuild after changing guest code. This is a development image builder, not a reproducible distribution-image pipeline.

## Create and run a simulation

```sh
cargo run -p sim-rns-core --bin sim-rns-ctl -- \
  create /tmp/my-sim /path/to/guest-bundle "My simulation"
cargo run -p sim-rns-core --bin sim-rns-ctl -- boot /tmp/my-sim
cargo run -p sim-rns-core --bin sim-rns-ctl -- status /tmp/my-sim
```

Every new project contains a virtual LAN, two Python Reticulum nodes (`backbone-a` and `phone-a`), and a seeded Python script. The second node keeps the scaffold's existing name; it runs Reticulum, not LXMF. Each Reticulum node creates a distinct persistent identity and sends plaintext diagnostic Reticulum packets over the guest-only LAN. Logs show `IDENTITY` and `RECEIVED hello from ...` entries. QEMU has no host-facing network interface.

Open `/tmp/my-sim` from the GTK app to see live VM/node status and log tails:

```sh
cargo run -p sim-rns-app
```

The CLI also supports `pause`, `resume`, `stop`, `start-node PROJECT ELEMENT_ID`, `stop-node PROJECT ELEMENT_ID`, and `restart-node PROJECT ELEMENT_ID`. The toolbar supports Run (including resume), Pause, and Stop. Closing the UI detaches; Stop shuts down the project. Guest shutdown stops node processes and syncs the data volume before QEMU exits.

In the launcher, choose **Create New Project**, enter a name and destination, and select your guest-bundle directory. The last selected bundle is remembered. Creation checks the manifest, readable nonempty assets, contained paths, and QEMU disk metadata before writing project files; this cannot guarantee that an arbitrary kernel will boot. New projects use 512 MiB RAM and one CPU, with supported Python Reticulum nodes. Click **Run** to boot. `create-demo` remains available as a compatibility shortcut.

Existing projects are unchanged. Their `vm.base_image` may be a guest-bundle directory or bootable image path; relative paths resolve against the project root. A blank image is not silently created.

## Edit a simulation

Open the **Nodes** tab while the VM is stopped. **Add Node / LAN** offers Python Reticulum, virtual LAN, Python script, and Bash script templates. Use **Edit** to change enabled state, LAN connections, environment variables, memory limits, CPU weight, restart policy, and command arguments. Script nodes accept Python/Bash source directly; existing project scripts have their own editor below the node list. Existing node IDs are fixed; create a new node to use a different ID.

Saving checks the whole recipe and rejects stale forms if another operation changed the project. Removing a LAN disconnects its nodes; removed node source files are retained. Running and paused simulations cannot be edited.

For a previously prepared VM, select **Start a fresh guest on next Run** before saving a configuration change. The old VM directory is preserved under `.sim-rns/previous-vm-*`, and the next Run prepares a new guest from the project bundle. The new guest has new identities and guest files. Backups consume disk space and are not removed automatically. Source edits do not live-update an initialized guest. This editor does not rename nodes or edit asset mappings and VM settings.

## End-to-end verification

```sh
SIM_RNS_TEST_BUNDLE=/path/to/guest-bundle \
  cargo test -p sim-rns-core --test guest_e2e -- --ignored --nocapture
```

This boots a temporary project, verifies packet reception by both Reticulum nodes, distinct identities, node stop/start, VM pause/resume, and identity persistence across reboot, then stops the VM. Failure leaves project artifacts in `/tmp/sr-e2e-*` for diagnosis. QEMU/guest boot diagnostics are in `.sim-rns/logs/qemu.log`.

## Current boundaries

- QMP responses and process identity are verified; an unresponsive control channel returns an error and retains process tracking. The app never kills a process using an unverified numeric PID. Legacy numeric PID files require manually stopping the old VM before removing the stale tracking file.
- Runtime commands are serialized by a per-project filesystem lock; metadata is replaced atomically. Status polling does not rewrite runtime metadata. Readers of externally edited project files still require valid JSON.
- Guest initialization is persistent. Editing an initialized guest requires a fresh guest disk; the Nodes editor can archive the old VM with explicit consent. Normal reboot preserves its files and identities and reruns the startup sequence. Resume preserves the paused execution state.
- Guest-supported templates are `network.lan`, `reticulum.python.backbone`, `script.python`, and `script.bash`. Asset seeds are copied; template rendering, Rust/LXMF runtimes, and arbitrary custom templates are not implemented.
- VM snapshots, live topology mutation, SSH projects, slowdown, and full recipe/asset editing remain unimplemented. Unsupported QEMU commands return errors instead of updating pretend state. `FileBackedRuntime` is a metadata-only test model.
- The serial control protocol is versioned and request-correlated. It is project-local, with no network listener. Logs are currently local files with bounded UI tails; disk log rotation is not implemented.
