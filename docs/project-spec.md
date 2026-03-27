# Reticulum Network Simulator

## Purpose

Reticulum Network Simulator (`sim-rns`) is a local-first environment for building, running, debugging, and experimenting with Reticulum networks on a single computer.

Its goal is to let a user create a synthetic network, run it as a living system, and study emergent behavior without needing a fleet of physical devices.

The simulator must support:

- rapid experimentation with topology and node configuration
- repeatable project initialization
- debugging of routing and service behavior
- checkpointing and resuming live simulations
- observation of emergent behavior in larger networks

## Product Shape

The product has two main parts:

1. A UI plugin for the `maruzzella` shell running on the host.
2. A simulation backend running inside a dedicated QEMU VM.

One project always runs inside exactly one dedicated VM.

The host-side plugin is the operator interface. The guest-side backend manages the running simulation inside the VM.

## Core Model

The simulator must distinguish between the parts of the system that can be described structurally and the parts that cannot.

### Project

A **project** is a living simulation world.

Operationally, a project is:

- one dedicated VM
- the full guest filesystem and process state inside that VM
- the running nodes and their evolving state
- zero or more snapshots of that VM state

A project is not fully representable as a clean structured document once it starts running. Arbitrary processes may send messages, mutate files, allocate memory, open sockets, maintain timers, and evolve in ways that cannot be completely anticipated or normalized into a declarative schema.

### Recipe

A **recipe** is the structured artifact used to initialize a project.

The recipe is the project's DNA. It is the part that can be described, versioned, diffed, edited, shared, and used to create a fresh project instance.

A recipe defines only things that can be declared ahead of time, such as:

- VM baseline configuration
- initial node set
- node personas and initial roles
- initial topology
- initial configuration files and templates
- provisioning actions
- startup actions
- initial simulator policies

The recipe is not the project. It is the blueprint from which a project can be created.

The recipe should be centered around two primary concerns:

- VM setup
- elements to run inside that VM

### Snapshot

A **snapshot** is a restorable checkpoint of a project.

Snapshots are the only faithful persisted representation of the full state of a running or paused project after arbitrary execution has taken place.

A snapshot may preserve:

- process memory
- open sockets and kernel-visible runtime state, as supported by the VM mechanism
- in-flight execution point
- guest filesystem state
- timers and execution context as captured by the VM

Snapshots are the continuity mechanism for a living project.

### Commands

A **command** is a structured operator action applied to a project.

Commands are protocolable even when the project itself is not.

Examples:

- create project from recipe
- boot project
- pause project
- restore snapshot
- inject topology change
- run action in a node
- collect logs

Commands are not the project state. They are structured interactions with the project.

## High-Level Requirements

### 1. One Project, One VM

Each project must execute inside exactly one dedicated QEMU VM. This is a hard architectural rule.

### 2. Recipe-Based Initialization

A project must be creatable from a structured recipe that describes its initial world state and initialization procedure.

### 3. Snapshot-Based Continuity

After execution begins, the simulator must treat snapshots as the authoritative persisted representation of the project's evolved state.

### 4. Strong Node Isolation

Inside the project VM, each simulated node must run in its own virtual environment and maintain distinct persistent state unless explicit sharing is modeled.

### 5. Project-Centric Workflow

Users operate on projects as living worlds, not on loose processes. The system should make it easy to create a project from a recipe, run it, pause it, snapshot it, restore it, inspect it, and evolve it.

### 6. Observability

Users need to inspect what is happening in the running project. Logs, events, process state, topology state, and VM state must be exposed in a useful way.

### 7. Extensibility

The first version is local-only, but the model should leave room for federation of simulator instances later.

## Architecture Outline

### Host Side: Maruzzella Plugin

The `maruzzella` plugin is responsible for operator-facing actions on the host.

Likely responsibilities:

- create recipe
- create project from recipe
- boot and shut down project VM
- pause and resume project
- create and restore snapshots
- edit initial topology and node setup through the recipe
- send commands into the guest backend
- show logs, events, and health
- import/export recipes and project artifacts

The host side should own VM lifecycle and orchestration entrypoints. It should not own detailed guest-side process management.

### Guest Side: Simulator Backend

The backend runs inside the project VM.

Core responsibilities:

- interpret the recipe during initialization
- provision node environments
- launch and manage node processes
- apply topology and simulator policies
- collect logs and events
- expose status to the host control plane
- execute structured commands sent from the host

### Node Runtime

Each node runtime represents one simulated device or role inside the project VM.

Responsibilities:

- maintain its own virtual environment
- maintain its own persistent filesystem area
- launch and stop node-specific services and RNS processes
- expose logs, status, and diagnostics
- accept topology or policy changes from the simulator backend

## What Is Structured

The system should explicitly recognize which artifacts are structured and which are not.

### Structured Artifacts

These can be versioned and protocolled:

- recipes
- command definitions
- command history or event metadata
- snapshot metadata
- exported observability artifacts

### Non-Structured Living State

These cannot be completely normalized into a declarative project schema once execution has begun:

- process-local memory
- arbitrary in-process mutations
- internal queues and buffers
- ad hoc filesystem mutations inside the VM
- timer state
- message state internal to arbitrary processes

That state lives in the running VM and, when persisted faithfully, in snapshots.

## Recipe Content

The recipe should define the initial conditions of a project and the process used to instantiate it.

At minimum, the recipe should contain:

- recipe metadata
- VM baseline configuration
- template definitions
- initial element registry
- initial topology
- initial simulator policies
- initial startup sequence

The recipe should not attempt to encode arbitrary post-boot runtime state.

### VM Setup

The recipe must define the initial VM envelope for the project.

At minimum, this includes:

- operating system or base image
- RAM allocation
- CPU allocation
- other VM-level resources and capabilities needed to host the project

### Elements

An **element** is a managed thing inside the project VM.

An element is broader than a process. It is the operator-visible unit the app knows how to render, observe, and control.

Examples:

- a Reticulum backbone node
- an LXMF client
- a network appliance such as a LAN provider, bridge, or NAT
- a Python script
- a Bash script

An element may execute as one process or more than one process, but the recipe and UI should reason about the element first.

### Element Model

Each element should have at minimum:

- `id`
- `template_id`

The template is mandatory because the UI and the control plane need to know what kind of element they are dealing with.

The template provides the semantic identity of the element, including its execution family and operational semantics.

### Element Directories and Isolation

Each element should have a canonical directory derived from its `id`.

The system should also be free to assign each element its own Linux user and sandbox it within its own home directory.

This should be the default model rather than requiring each recipe author to define arbitrary working directories.

### Command Model

Execution should be command-based.

An element ultimately runs through a launch command, but that command should usually come from the resolved template rather than being repeated on every instance.

If instance-level overrides are allowed, they should be structured as command overrides, not as the primary identity of the element.

Commands should be represented as argv arrays rather than shell strings.

### Instance-Level Configuration

An element instance should be able to provide instance-specific configuration such as:

- environment variables
- assets and seeded files specific to that instance
- restart policy
- CPU and memory limits
- enabled or disabled state at initialization

These fields are instance-specific and distinct from template-level runtime dependencies.

## Template Model

Templates are required to give elements a known semantic identity.

### Base Templates

The system should ship with built-in base templates.

Base templates define known executable families and known UI/control semantics.
Base templates define known executable families and known control semantics.

They should include at least:

- `rns.rs.backbone`
- `lxmf.rs.client`
- `reticulum.python.backbone`
- `lxmf.python.client`
- `network.lan`
- `script.python`
- `script.bash`

Base templates are also responsible for defining the runtime dependency model for the image. In other words, the template defines which binaries and supporting files are expected to exist in the VM image.

### Custom Templates

Users should be able to define custom templates.

In v1, a custom template should specialize a known base template rather than introducing an entirely unknown execution family.

A custom template should be able to override or specialize:

- parameters
- environment defaults
- instance asset expectations
- restart defaults
- resource defaults
- presentation metadata

This makes a custom template a reusable preset over a known implementation, rather than an entirely new runtime model.

### Presentation Mapping

Templates are not UI objects.

The app should map `template_id` values to icons and visual treatment on its own side.

For custom templates, the app may resolve presentation by:

1. exact `template_id`
2. `extends`
3. template category
4. generic fallback

This keeps presentation concerns out of the recipe and out of the template runtime model.

### Control and Observability Surface

The control surface and monitor surface of an element should primarily come from its template.

This includes:

- what status the app can observe
- which commands the app can issue
- which logs, events, and metrics matter

In v1, these surfaces should be implicit in the template family rather than defined ad hoc on every element instance.

## Project Lifecycle

At minimum, the simulator should support:

1. Create or edit a recipe.
2. Instantiate a new project from that recipe.
3. Boot the project VM.
4. Provision and start the initial node set inside the VM.
5. Observe and interact with the running project.
6. Pause the project.
7. Save a snapshot.
8. Restore a snapshot.
9. Resume execution.

## Time Model

Time control must be defined around the project VM boundary.

### V1 Time Controls

Version 1 supports:

- pause
- resume
- slowdown
- snapshot

These controls apply to the whole project VM.

### Acceleration

Generic fast-forward is not guaranteed for arbitrary guest processes.

Changing the baseline clock inside the VM is not enough to guarantee correct acceleration because guest timers, sleeps, retries, IO waits, and scheduling behavior may diverge from the intended simulation semantics.

A future acceleration feature would require a simulator-aware logical time model for the components that need it.

## Snapshot and Recording Model

The first version uses snapshots as its recording mechanism.

Supported capability:

- pause a project
- save a snapshot
- restore that snapshot later

This is intended for:

- debugging
- checkpointing experiments
- branch-and-compare workflows
- preserving interesting emergent states

The first version does not promise exact deterministic replay of arbitrary full executions.

## Storage Model

The host-side storage layout should reflect the distinction between recipe, project artifacts, and snapshots.

A plausible structure:

```text
workspace/
  recipes/
    mesh-lab-01/
      recipe.json
      topology.json
      nodes/
  projects/
    mesh-lab-01/
      vm/
      snapshots/
      artifacts/
      history/
```

The exact format is open, but the model should preserve:

- portability of recipes
- inspectability of metadata
- clean separation between blueprint and live world artifacts
- room for multiple snapshots per project

## Observability and Debugging

The simulator is for experimentation, so debugging is a core feature.

The first broad observability set should include:

- per-node logs
- project event stream
- topology view
- VM runtime status
- node runtime status and health
- startup and crash diagnostics
- timeline of operator commands

Later phases may add:

- packet-level inspection
- route visualization
- metrics and counters
- replay-oriented traces

## Non-Goals for the First Version

Out of scope for now:

- cross-machine federation
- perfect hardware emulation
- exhaustive network physics simulation
- arbitrary deterministic replay
- attempting to serialize the full live project state as a declarative manifest

## Future Federation Direction

Federation remains a future concern.

If introduced later, it should build on the same core model:

- a project is still a living execution world
- a recipe is still the initialization blueprint
- snapshots are still the continuity mechanism

The open problem would be how multiple hosts cooperate to represent or coordinate a larger distributed project.

## Open Design Questions

These should be resolved before implementation goes deep:

1. What exact schema should the recipe use?
2. How should recipe provisioning steps be expressed?
3. What is the control protocol between host plugin and guest backend?
4. How are topology mutations applied after the project has already evolved?
5. What metadata should be stored alongside snapshots?
6. How much command history should be persisted?
7. How should project artifacts be imported and exported?

## Suggested Phased Scope

### Phase 1: Recipe and Local Project MVP

- define recipe format
- create project from recipe
- provision one VM per project
- create nodes with isolated virtual environments inside the VM
- apply initial topology
- start and stop the project
- pause, resume, and snapshot the project
- inspect logs and node status through `maruzzella`

### Phase 2: Better Operational Control

- richer topology mutation after boot
- partial failure injection
- per-node lifecycle control
- command history
- better observability
- VM slowdown controls

### Phase 3: Advanced Experimentation

- simulator-aware logical clock
- selected accelerated simulation behaviors
- richer trace and metrics analysis
- larger network ergonomics

### Phase 4: Federation

- multi-host coordination
- remote project placement
- distributed topology links
- shared control across simulator instances

## Initial Acceptance Criteria

An initial usable version should let a user:

1. Define a recipe.
2. Create a project from that recipe.
3. Provision one dedicated VM for that project.
4. Create multiple nodes with distinct identities and isolated environments inside the VM.
5. Define the initial connectivity between nodes.
6. Start the project on a single machine.
7. Pause and resume the whole project.
8. Save and restore a project snapshot.
9. Observe each node's logs and runtime status.
10. Reuse the same recipe to create a fresh project again.
