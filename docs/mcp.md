# MCP server

Build with `cargo build --workspace --locked`. Point a stdio MCP client at the absolute path of `target/debug/sim-rns-mcp` (or the release binary). No HTTP listener or port is involved. The server uses the official Rust MCP SDK for initialization, tool discovery, argument handling and stdio transport. Standard output contains only MCP messages; diagnostics go to standard error.

```json
{
  "mcpServers": {
    "sim-rns": {
      "command": "/absolute/path/to/sim-rns/target/debug/sim-rns-mcp"
    }
  }
}
```

The MCP process and GTK application are separate. `launch` starts `sim-rns-app` alongside the MCP executable, inheriting its desktop environment, or attaches if the app is already running. You can also start `sim-rns-app` yourself. Closing an MCP connection leaves the app and simulation running. Use `runtime.shutdown` before `app.quit` when you want to stop both; quitting or closing a project does not shut down its VM.

## Tools

All tools except `launch` take an `operation` discriminator. `tools/list` supplies the exact schemas and required fields. In this document, `runtime.status` means the `runtime` tool with `{"operation":"status"}`.

| Tool | Operations |
| --- | --- |
| `launch` | Start or attach to the app; no arguments |
| `app` | `state`, `quit` |
| `project` | `create`, `create_demo`, `open`, `close`, `inspect`, `add_node`, `add_script`, `read_file`, `write_file` |
| `runtime` | `status`, `prepare`, `boot`, `shutdown`, `pause`, `resume`, `start_node`, `stop_node`, `restart_node`, snapshot and topology operations |
| `ui` | `tree`, `screenshot`, `command`, `activate`, `context_menu`, `dismiss_popup`, `drag`, `focus`, `set_text`, `set_active`, `set_value`, `select`, `scroll`, `present`, `resize`, `maximize`, `close_window` |

Project and runtime tools target the active project in the app. `project.inspect` returns the root project file and derived recipe, including include paths, templates, elements and topology. `runtime.status` includes node states, recent events, log tails and VM assets. Runtime mutations return after completion, with the command outcome and resulting status. `boot` prepares the VM disk if needed.

To create a runnable example after building a guest bundle:

```json
{"operation":"create_demo","path":"/tmp/my-simulation","bundle":"/absolute/path/to/guest-bundle"}
```

Then call `runtime` with:

```json
{"operation":"boot"}
{"operation":"status"}
{"operation":"stop_node","element_id":"phone-a"}
{"operation":"start_node","element_id":"phone-a"}
{"operation":"pause"}
{"operation":"resume"}
{"operation":"shutdown"}
```

`project.create` makes the generic scaffold, with the same current template limitations as the GUI. `create_demo` produces the supported Python Reticulum configuration.

## UI control

Call `ui.tree` first. It returns every in-process GTK top-level window, including dialogs, plus its widget hierarchy. Widgets include IDs, type, text/label where applicable, visibility, sensitivity, focus, CSS classes, bounds relative to their parent and controller types. Windows expose installed action names, readable command IDs for app menus/toolbars, and enabled state. `ui.command` accepts either an installed action name or a canonical command ID such as `shell.settings`. Dropdowns expose string options, toggles expose their state, and numeric controls expose values.

Use the returned IDs and exact command names:

```json
{"operation":"activate","widget_id":42}
{"operation":"set_text","widget_id":91,"text":"hello from MCP"}
{"operation":"command","window_id":1,"name":"shell.settings"}
{"operation":"screenshot","window_id":1}
```

These IDs are examples, not constants. IDs are tied to widget lifetime; refresh the tree after opening/closing projects, views or dialogs. A stale ID returns an error. Hidden or insensitive widgets reject interactions. Tab activation invokes the shell's selection callback, keeping visible pages, tab styling and persisted selection consistent. Tab context menus expose shell layout actions, and `drag` drives the widget's drag controller for tab rearrangement. Coordinates are widget-local start points plus pixel offsets; inspect bounds before dragging.

`set_text` supports editable entries and text views, `set_active` supports checks/toggles/switches, `set_value` supports spin buttons and ranges, and `select` supports dropdowns and list boxes. `scroll` takes absolute horizontal/vertical adjustment values. `screenshot` returns an MCP PNG image block, not a host file path. UI commands return after dispatch; use a later tree or runtime status call to observe asynchronous effects.

## Local connection and behavior

The app listens on `$XDG_RUNTIME_DIR/sim-rns/control.sock`, falling back to `$HOME/.cache/sim-rns/control.sock`. Override it with `SIM_RNS_CONTROL_SOCKET` on both processes, or pass `--socket /path/to/control.sock` to the MCP server. `launch` passes the selected socket to the child app. The parent directory must be owned by the current user and have mode `0700`; the socket has mode `0600`. A file lock prevents a second app from replacing a live endpoint, and a new owner cleans stale sockets after a crash. This endpoint grants full app control to local processes running as that user.

GTK work runs on the main thread. Project I/O and QEMU operations run on workers using the same busy state as the toolbar. Concurrent UI inspection remains available during VM boot. Project switching and conflicting runtime operations are rejected while work is in progress. Calls time out after about three minutes; a timeout or client cancellation does not undo work already dispatched. Inspect state before retrying mutations.

`write_file` replaces an existing UTF-8 source file of at most 1 MiB. It rejects absolute paths, parent traversal, symlinks escaping the project, and private metadata such as `.sim-rns` and `.git`. It requires a stopped VM, holds the project lock, writes atomically, validates the recipe and restores the original on validation failure. Recipe/template views refresh after successful edits. It is not a multi-file transaction. Files in a prepared guest retain the existing guest initialization semantics; editing sources does not hot-reload guest configuration.

Control messages and responses are limited to 4 MiB; exceptionally large widget trees or screenshots return errors. Text-view contents in the tree are limited to 16,000 characters. Native OS file choosers hosted outside the process are not part of the GTK tree; use `project.open`/`create` for project selection. There is no general desktop keyboard/mouse automation API.

Snapshots and live topology changes remain unsupported by the QEMU backend. Their runtime operations return explicit errors. The MCP layer does not claim to implement these missing simulator features.

## Verification

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
xvfb-run -a dbus-run-session -- python3 scripts/test-mcp.py
xvfb-run -a dbus-run-session -- python3 scripts/test-mcp.py \
  --guest-bundle /absolute/path/to/guest-bundle
```

The integration test uses real stdio MCP messages, an isolated configuration directory and a private GTK display/session. It checks discovery, invalid arguments, launch/attach, project operations, file rollback, UI tabs, text editing, screenshots and error propagation. With a guest bundle it also checks real VM/node control, Reticulum packet exchange and UI responsiveness during boot.
