# Sim RNS UI Shell Spec

## Purpose

This document defines the top-level application UX for `sim-rns` as hosted inside `maruzzella`.

The key product rule is that the app has two primary modes:

1. no project open
2. project open

These are not just view states. They are two distinct shell modes with different responsibilities and different UX expectations.

## Shell Model

`sim-rns` must run inside `maruzzella`, but it must not always start in the normal full workspace shell.

The application should use the two-mode shell capability now available in `maruzzella`:

- `launcher` mode
- `workspace` mode

The app starts in launcher mode. When a project is opened, the app switches to workspace mode. When the project is closed, the app returns to launcher mode.

## Mode 1: No Project Open

### Summary

When no project is open, the app should present a compact launcher-style shell rather than the normal project workspace.

This should feel similar to a JetBrains-style "welcome / no project open" window:

- compact
- purpose-built
- centered on opening or creating a project
- not visually framed as an already-open workspace

### Required Actions

Launcher mode should contain exactly these primary actions:

- recent projects
- open local project
- open remote project over SSH
- creation wizard

These are sufficient for v1.

### Explicit Non-Goals

Launcher mode does not need these as first-class features in v1:

- connection manager
- known hosts management UI
- general shell workbench panels
- fake empty editor/workbench tabs

### UX Expectations

Launcher mode should:

- use a compact, non-maximized window by default
- avoid the full project workspace chrome
- foreground the project-opening actions immediately
- make recent items easy to reopen

## Launcher Content

### Recent Projects

The launcher should show a recent-projects list.

This list should contain both local and remote project handles.

Each item should show enough information for the user to understand what they are reopening:

- project name if known
- local path, or remote `user@host:path`
- whether the entry is local or remote

The recent list is an action list, not just a history display.

### Open Local Project

This flow opens a project available on the local filesystem.

The user should be able to:

- choose the local project path
- validate that it is a compatible project
- open it into workspace mode

### Open Remote Project Over SSH

This flow opens a project hosted on another machine.

The user should be able to provide:

- SSH target
- remote project path

Once opened:

- the simulator continues to run on the remote host
- only the UI runs locally
- the local app acts as a control and visualization client

This is not a local import. It is attachment to a remote project host through SSH.

### Creation Wizard

This flow creates a new project from a recipe and then opens it.

The creation wizard should eventually support both local and remote targets, though a local-first implementation is acceptable at the beginning.

The wizard is part of launcher mode because it exists before a live project is open.

## Project Handle

The UI and shell transition should use a common project-handle concept.

A `ProjectHandle` represents an opened project independently of whether it is local or remote.

At minimum, a project handle should carry:

- transport type
- location
- project identity if known
- enough metadata for the workspace to reconnect or refresh state

### Transport Types

The first two transport types are:

- `local`
- `ssh`

### Local Project Handle

A local project handle should represent:

- local filesystem location
- local runtime context

### Remote SSH Project Handle

A remote project handle should represent:

- SSH target
- remote project path
- remote runtime context

In remote mode, the simulator runtime stays on the remote machine.

## Mode 2: Project Open

### Summary

Once a project is open, the app switches to the full workspace shell.

This is the real simulator workspace, where the user operates on the live project.

### Workspace Scope

Workspace mode is where the app should expose:

- project status
- VM state
- snapshots
- elements
- topology
- logs and events
- controls and commands

The detailed workspace layout is still to be designed, but it must be distinct from launcher mode.

### Transition Into Workspace

When the user opens a project from launcher mode:

1. the launcher flow resolves a valid project handle
2. the app constructs the project workspace session
3. `maruzzella` switches from launcher mode to workspace mode

The workspace should now be bound to that project handle.

### Closing A Project

When the user closes the current project:

1. the workspace detaches from the project handle
2. the app switches back to launcher mode
3. the app remains running

Closing a project is not the same thing as quitting the application.

## Maruzzella Integration

`sim-rns` depends on `maruzzella`'s launcher/workspace mode support.

The intended model is:

- startup in `ShellMode::Launcher`
- launcher content hosted inside `maruzzella`
- switch to `WorkspaceSession` when a project opens
- switch back to launcher when the project closes

This is the correct integration path for `sim-rns`.

## Design Principles

### 1. Project-First UX

The app is organized around a live project. The UI should not pretend that an empty workspace is meaningful when no project is attached.

### 2. Launcher Is A Real App Mode

The no-project-open state is not a popup and not a startup placeholder tab. It is a real shell mode.

### 3. Local And Remote Should Share The Same UX Model

Local and remote projects should feel like two transport modes of the same concept, not like two different products.

### 4. The Workspace Only Appears When It Matters

The full workspace shell should appear only once there is a project to operate on.

## Open Questions

These still need to be defined:

1. What exact widget/layout composition should launcher mode use?
2. What exact fields should `ProjectHandle` contain?
3. How should recent projects be persisted?
4. How should failed SSH connection attempts be surfaced in launcher mode?
5. What is the first concrete workspace layout once a project opens?
