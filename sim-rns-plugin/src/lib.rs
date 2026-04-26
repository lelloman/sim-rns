use std::rc::Rc;

mod runtime_store;

use gtk::glib::translate::IntoGlibPtr;
use gtk::prelude::*;
use gtk::{
    gio, Align, Box as GtkBox, Button, FileDialog, Frame, GestureClick, Label, ListBox, ListBoxRow,
    Orientation, Picture, PolicyType, ScrolledWindow, SelectionMode, Separator,
};
use maruzzella_sdk::ffi::{MzBytes, MzStatus};
use maruzzella_sdk::{
    button_css_class, export_plugin, surface_css_class, text_css_class, CommandSpec, HostApi,
    MzStatusCode, MzViewPlacement, Plugin, PluginDependency, PluginDescriptor,
    SurfaceContributionSpec, Version, ViewFactorySpec,
};
use sim_rns_core::{
    close_project, create_project, current_project, load_project, open_project, project_recipe,
    Element, LauncherConfig, Project, ProjectHandle, ProjectRuntime, QemuRuntime, Recipe,
    RuntimeCommand, RuntimeStatus, RuntimeVmState, Template,
};

use runtime_store::{RuntimeController, RuntimeViewSnapshot};

const PLUGIN_ID: &str = "com.lelloman.sim_rns";
const CONFIG_SCHEMA_VERSION: u32 = 1;
const VIEW_LAUNCHER: &str = "com.lelloman.sim_rns.launcher";
const VIEW_OVERVIEW: &str = "com.lelloman.sim_rns.overview";
const VIEW_RECIPE: &str = "com.lelloman.sim_rns.recipe";
const VIEW_TEMPLATES: &str = "com.lelloman.sim_rns.templates";
const CMD_NEW_PROJECT: &str = "sim-rns.project.new";
const CMD_OPEN_PROJECT: &str = "sim-rns.project.open";
const CMD_CLOSE_PROJECT: &str = "sim-rns.project.close";
const CMD_EXIT_APP: &str = "sim-rns.app.exit";
const CMD_RUN_PROJECT: &str = "sim-rns.runtime.run";
const CMD_PAUSE_PROJECT: &str = "sim-rns.runtime.pause";
const CMD_STOP_PROJECT: &str = "sim-rns.runtime.stop";

thread_local! {
    static RUNTIME_CONTROLLER: RuntimeController = RuntimeController::default();
}

pub struct SimRnsPlugin;

impl Plugin for SimRnsPlugin {
    fn descriptor() -> PluginDescriptor {
        static DEPENDENCIES: &[PluginDependency] = &[PluginDependency::required(
            "maruzzella.base",
            Version::new(1, 0, 0),
            Version::new(2, 0, 0),
        )];

        PluginDescriptor::new(PLUGIN_ID, "Sim RNS", Version::new(0, 1, 0))
            .with_description("Reticulum Network Simulator workspace plugin")
            .with_dependencies(DEPENDENCIES)
    }

    fn register(host: &HostApi<'_>) -> Result<(), MzStatusCode> {
        host.log(
            maruzzella_sdk::ffi::MzLogLevel::Info,
            "Registering Sim RNS plugin",
        );

        host.register_surface_contribution(SurfaceContributionSpec::about_section(
            PLUGIN_ID,
            "com.lelloman.sim_rns.about",
            "Sim RNS",
            "VM-backed Reticulum network simulator hosted inside Maruzzella.",
        ))?;

        host.register_command(
            CommandSpec::new(PLUGIN_ID, CMD_NEW_PROJECT, "New Project")
                .with_handler(open_project_launcher_command),
        )?;
        host.register_command(
            CommandSpec::new(PLUGIN_ID, CMD_OPEN_PROJECT, "Open Project")
                .with_handler(open_project_launcher_command),
        )?;
        host.register_command(
            CommandSpec::new(PLUGIN_ID, CMD_CLOSE_PROJECT, "Close Project")
                .with_handler(close_project_command),
        )?;
        host.register_command(
            CommandSpec::new(PLUGIN_ID, CMD_EXIT_APP, "Exit").with_handler(exit_app_command),
        )?;
        host.register_command(
            CommandSpec::new(PLUGIN_ID, CMD_RUN_PROJECT, "Run")
                .with_handler(run_project_command)
                .with_enabled(can_run_project_command),
        )?;
        host.register_command(
            CommandSpec::new(PLUGIN_ID, CMD_PAUSE_PROJECT, "Pause")
                .with_handler(pause_project_command)
                .with_enabled(can_pause_project_command),
        )?;
        host.register_command(
            CommandSpec::new(PLUGIN_ID, CMD_STOP_PROJECT, "Stop")
                .with_handler(stop_project_command)
                .with_enabled(can_stop_project_command),
        )?;

        host.register_view_factory(ViewFactorySpec::new(
            PLUGIN_ID,
            VIEW_LAUNCHER,
            "Project Opener",
            MzViewPlacement::Workbench,
            create_launcher_view,
        ))?;
        host.register_view_factory(ViewFactorySpec::new(
            PLUGIN_ID,
            VIEW_OVERVIEW,
            "Sim RNS Overview",
            MzViewPlacement::Workbench,
            create_overview_view,
        ))?;
        host.register_view_factory(ViewFactorySpec::new(
            PLUGIN_ID,
            VIEW_RECIPE,
            "Recipe",
            MzViewPlacement::Workbench,
            create_recipe_view,
        ))?;
        host.register_view_factory(ViewFactorySpec::new(
            PLUGIN_ID,
            VIEW_TEMPLATES,
            "Templates",
            MzViewPlacement::Workbench,
            create_templates_view,
        ))?;

        Ok(())
    }
}

extern "C" fn open_project_launcher_command(_payload: MzBytes) -> MzStatus {
    close_project_command(MzBytes::empty())
}

extern "C" fn close_project_command(_payload: MzBytes) -> MzStatus {
    match close_project() {
        Ok(()) => {
            refresh_runtime_store();
            MzStatus::OK
        }
        Err(error) => {
            publish_runtime_error(error);
            MzStatus::new(MzStatusCode::InternalError)
        }
    }
}

extern "C" fn exit_app_command(_payload: MzBytes) -> MzStatus {
    if let Some(application) = gio::Application::default() {
        application.quit();
    } else {
        std::process::exit(0);
    }
    MzStatus::OK
}

extern "C" fn run_project_command(_payload: MzBytes) -> MzStatus {
    runtime_command_status(run_project_runtime)
}

extern "C" fn pause_project_command(_payload: MzBytes) -> MzStatus {
    runtime_command_status(|| execute_runtime_command(RuntimeCommand::Pause))
}

extern "C" fn stop_project_command(_payload: MzBytes) -> MzStatus {
    runtime_command_status(|| execute_runtime_command(RuntimeCommand::Shutdown))
}

extern "C" fn can_run_project_command() -> bool {
    matches!(
        current_runtime_vm_state(),
        Some(RuntimeVmState::Stopped | RuntimeVmState::Paused)
    )
}

extern "C" fn can_pause_project_command() -> bool {
    matches!(current_runtime_vm_state(), Some(RuntimeVmState::Running))
}

extern "C" fn can_stop_project_command() -> bool {
    matches!(
        current_runtime_vm_state(),
        Some(RuntimeVmState::Running | RuntimeVmState::Paused)
    )
}

fn runtime_command_status(command: impl FnOnce() -> Result<(), String>) -> MzStatus {
    match RUNTIME_CONTROLLER
        .with(|controller| controller.run_command(command, load_runtime_snapshot))
    {
        Ok(()) => MzStatus::OK,
        Err(error) => {
            eprintln!("sim-rns: runtime command failed: {error}");
            MzStatus::new(MzStatusCode::InternalError)
        }
    }
}

fn current_runtime_vm_state() -> Option<RuntimeVmState> {
    RUNTIME_CONTROLLER.with(|controller| controller.vm_state(load_runtime_snapshot))
}

fn refresh_runtime_store() {
    RUNTIME_CONTROLLER.with(|controller| {
        controller.refresh(load_runtime_snapshot);
    });
}

fn publish_runtime_error(error: String) {
    RUNTIME_CONTROLLER.with(|controller| {
        controller.publish_error(error, load_runtime_snapshot);
    });
}

fn load_runtime_snapshot() -> RuntimeViewSnapshot {
    let project = match load_workspace_project() {
        Ok(project) => project,
        Err(error) => return RuntimeViewSnapshot::error(error),
    };
    let recipe = match project_recipe(&project) {
        Ok(recipe) => recipe,
        Err(error) => return RuntimeViewSnapshot::project_error(project, error),
    };
    let status = match QemuRuntime::default().status(&project) {
        Ok(status) => status,
        Err(error) => {
            return RuntimeViewSnapshot::runtime_error(project, recipe, error.to_string())
        }
    };
    RuntimeViewSnapshot::loaded(project, recipe, status)
}

fn build_root(title: &str, subtitle: &str) -> GtkBox {
    let root = GtkBox::new(Orientation::Vertical, 12);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);

    let title_label = Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.add_css_class("title-2");

    let subtitle_label = Label::new(Some(subtitle));
    subtitle_label.set_xalign(0.0);
    subtitle_label.set_wrap(true);
    subtitle_label.add_css_class("dim-label");

    root.append(&title_label);
    root.append(&subtitle_label);
    root
}

fn workspace_error_label() -> Label {
    let label = Label::new(None);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class("error");
    label.set_visible(false);
    label
}

fn create_scroller() -> ScrolledWindow {
    let scroller = ScrolledWindow::new();
    scroller.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller
}

fn section_card(title: &str, body: &[String]) -> ListBoxRow {
    let row = ListBoxRow::new();
    let card = GtkBox::new(Orientation::Vertical, 6);
    card.set_margin_top(10);
    card.set_margin_bottom(10);
    card.set_margin_start(10);
    card.set_margin_end(10);

    let heading = Label::new(Some(title));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    card.append(&heading);

    for line in body {
        let label = Label::new(Some(line));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_selectable(true);
        card.append(&label);
    }

    row.set_child(Some(&card));
    row
}

fn overview_lines(recipe: &Recipe) -> Vec<String> {
    vec![
        "Project model: one project = one dedicated QEMU VM".to_string(),
        format!(
            "Recipe `{}` defines {} elements and {} templates",
            recipe.metadata.id,
            recipe.elements.len(),
            recipe.templates.len()
        ),
        format!(
            "VM envelope: {} / {} MiB / {} cores",
            recipe.vm.base_image, recipe.vm.ram_mb, recipe.vm.cpu_cores
        ),
        "Launcher mode now selects a local project before entering this scaffold workspace"
            .to_string(),
    ]
}

fn element_lines(element: &Element) -> Vec<String> {
    let command = element
        .command_override
        .as_ref()
        .map(|parts| parts.join(" "))
        .unwrap_or_else(|| "template default".to_string());
    let resources = element
        .resources
        .as_ref()
        .map(|limits| {
            format!(
                "{} MiB / cpu weight {}",
                limits.memory_mb, limits.cpu_weight
            )
        })
        .unwrap_or_else(|| "template default".to_string());
    vec![
        format!("template = {}", element.template_id),
        format!("enabled = {}", element.enabled),
        format!("command = {}", command),
        format!("resources = {}", resources),
        format!("assets = {}", element.assets.len()),
    ]
}

fn template_lines(template: &Template) -> Vec<String> {
    vec![
        format!("category = {:?}", template.category),
        format!("extends = {}", template.extends.as_deref().unwrap_or("-")),
        format!("runtime = {:?}", template.runtime.family),
        format!(
            "image features = {}",
            template.runtime.image_features.join(", ")
        ),
        format!("default command = {}", template.defaults.command.join(" ")),
    ]
}

fn load_config(host: &maruzzella_sdk::ffi::MzHostApi) -> LauncherConfig {
    if host.read_config_record.is_some() {
        if let Ok(config) = HostApi::from_raw(host).read_json_config::<LauncherConfig>() {
            return config;
        }
    }
    read_local_launcher_config().unwrap_or_default()
}

fn save_config(
    host: &maruzzella_sdk::ffi::MzHostApi,
    config: &LauncherConfig,
) -> Result<(), MzStatusCode> {
    if host.write_config_record.is_some() {
        return HostApi::from_raw(host).write_json_config(config, Some(CONFIG_SCHEMA_VERSION));
    }
    write_local_launcher_config(config)
}

fn local_launcher_config_path() -> Option<std::path::PathBuf> {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        let trimmed = config_home.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed).join("sim-rns/launcher.json"));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(|home| {
            std::path::PathBuf::from(home)
                .join(".config")
                .join("sim-rns")
                .join("launcher.json")
        })
}

fn read_local_launcher_config() -> Result<LauncherConfig, MzStatusCode> {
    let Some(path) = local_launcher_config_path() else {
        return Ok(LauncherConfig::default());
    };
    if !path.is_file() {
        return Ok(LauncherConfig::default());
    }
    let payload = std::fs::read_to_string(path).map_err(|_| MzStatusCode::InternalError)?;
    serde_json::from_str(&payload).map_err(|_| MzStatusCode::InternalError)
}

fn write_local_launcher_config(config: &LauncherConfig) -> Result<(), MzStatusCode> {
    let Some(path) = local_launcher_config_path() else {
        return Err(MzStatusCode::NotFound);
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| MzStatusCode::InternalError)?;
    }
    let payload = serde_json::to_string_pretty(config).map_err(|_| MzStatusCode::InternalError)?;
    std::fs::write(path, payload).map_err(|_| MzStatusCode::InternalError)
}

fn home_dir_or_root() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()))
}

fn prompt_directory_picker(
    parent: Option<&gtk::Window>,
    title: &str,
    confirm_label: &str,
    initial_path: &std::path::Path,
    on_selected: impl Fn(std::path::PathBuf) + 'static,
) {
    let on_selected = std::rc::Rc::new(on_selected) as std::rc::Rc<dyn Fn(std::path::PathBuf)>;
    let mut builder = FileDialog::builder()
        .title(title)
        .accept_label(confirm_label)
        .modal(true);
    if initial_path.is_dir() {
        builder = builder.initial_folder(&gio::File::for_path(initial_path));
    }
    let dialog = builder.build();
    dialog.select_folder(parent, gio::Cancellable::NONE, move |result| {
        if let Ok(file) = result {
            let Some(path) = file.path() else {
                return;
            };
            let on_selected = on_selected.clone();
            gtk::glib::idle_add_local_once(move || on_selected(path));
        }
    });
}

fn set_error(label: &Label, message: &str) {
    label.set_label(message);
    label.set_visible(!message.is_empty());
}

fn host_api_log(
    host: &maruzzella_sdk::ffi::MzHostApi,
    level: maruzzella_sdk::ffi::MzLogLevel,
    message: &str,
) {
    if let Some(log) = host.log {
        log(
            level,
            maruzzella_sdk::ffi::MzStr {
                ptr: message.as_ptr(),
                len: message.len(),
            },
        );
    } else {
        eprintln!("{message}");
    }
}

fn build_panel_frame() -> (Frame, GtkBox) {
    let frame = Frame::new(None);
    frame.set_hexpand(true);
    frame.set_vexpand(true);

    let content = GtkBox::new(Orientation::Vertical, 10);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(6);
    content.set_margin_end(6);
    frame.set_child(Some(&content));

    (frame, content)
}

fn open_selected_project(
    host: &maruzzella_sdk::ffi::MzHostApi,
    project: &Project,
) -> Result<(), String> {
    let handle = project.handle();
    host_api_log(
        host,
        maruzzella_sdk::ffi::MzLogLevel::Info,
        &format!(
            "sim-rns: open_selected_project path={} display_name={}",
            handle.path, handle.display_name
        ),
    );
    let mut config = load_config(host);
    config.remember_project(handle.clone());
    if let Err(status) = save_config(host, &config) {
        let (level, message) = if status == MzStatusCode::NotFound {
            (
                maruzzella_sdk::ffi::MzLogLevel::Info,
                "sim-rns: recents persistence is unavailable".to_string(),
            )
        } else {
            (
                maruzzella_sdk::ffi::MzLogLevel::Warn,
                format!("sim-rns: failed to save recents, continuing: {status:?}"),
            )
        };
        host_api_log(host, level, &message);
    }
    host_api_log(
        host,
        maruzzella_sdk::ffi::MzLogLevel::Info,
        "sim-rns: opening project through installed project opener",
    );
    if let Err(error) = open_project(handle) {
        host_api_log(
            host,
            maruzzella_sdk::ffi::MzLogLevel::Error,
            &format!("sim-rns: project opener failed: {error}"),
        );
        return Err(error);
    }
    host_api_log(
        host,
        maruzzella_sdk::ffi::MzLogLevel::Info,
        "sim-rns: project opened successfully",
    );
    refresh_runtime_store();
    Ok(())
}

fn load_workspace_project() -> Result<Project, String> {
    current_project()
}

fn build_project_summary(project: &Project) -> Vec<String> {
    vec![
        format!("Project root = {}", project.root_path.display()),
        format!(
            "Project file = {}",
            project.root_path.join("sim-rns.project.json").display()
        ),
        format!(
            "Includes: {} node files, {} scripts, {} configs, {} assets",
            project.file.includes.nodes.len(),
            project.file.includes.scripts.len(),
            project.file.includes.configs.len(),
            project.file.includes.assets.len()
        ),
    ]
}

fn runtime_summary_lines(status: &RuntimeStatus) -> Vec<String> {
    vec![
        format!("VM state = {:?}", status.vm_state),
        format!("Backend state = {:?}", status.backend_state),
        format!("VM assets prepared = {}", status.vm_assets.prepared),
        format!("VM disk = {}", status.vm_assets.disk_image_path),
        format!("QMP socket = {}", status.vm_assets.qmp_socket_path),
        format!("VM log = {}", status.vm_assets.log_path),
        format!("Nodes = {}", status.nodes.len()),
        format!(
            "Effective topology links = {}",
            status.effective_topology.len()
        ),
        format!("Snapshots = {}", status.snapshots.len()),
    ]
}

fn runtime_node_lines(status: &RuntimeStatus) -> Vec<String> {
    status
        .nodes
        .iter()
        .map(|node| {
            format!(
                "{} [{}] enabled={} state={:?}",
                node.element_id, node.template_id, node.enabled, node.state
            )
        })
        .collect()
}

fn runtime_topology_lines(status: &RuntimeStatus) -> Vec<String> {
    if status.effective_topology.is_empty() {
        return vec!["No active topology links.".to_string()];
    }
    status
        .effective_topology
        .iter()
        .map(|attachment| format!("{} -> {}", attachment.element_id, attachment.network_id))
        .collect()
}

fn runtime_snapshot_lines(status: &RuntimeStatus) -> Vec<String> {
    if status.snapshots.is_empty() {
        return vec!["No snapshots have been created.".to_string()];
    }
    status
        .snapshots
        .iter()
        .map(|snapshot| {
            format!(
                "{} `{}` vm={:?}",
                snapshot.id, snapshot.name, snapshot.vm_state
            )
        })
        .collect()
}

fn runtime_event_lines(status: &RuntimeStatus) -> Vec<String> {
    if status.recent_events.is_empty() {
        return vec!["No runtime commands have been recorded.".to_string()];
    }
    status
        .recent_events
        .iter()
        .map(|event| format!("#{} {}", event.id, event.message))
        .collect()
}

fn populate_overview_from_snapshot(
    list: &ListBox,
    error_label: &Label,
    snapshot: &RuntimeViewSnapshot,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    if let Some(error) = &snapshot.error {
        set_error(error_label, error);
    } else {
        set_error(error_label, "");
    }

    if let Some(project) = &snapshot.project {
        list.append(&section_card("Project", &build_project_summary(project)));
    }
    if let Some(status) = &snapshot.status {
        list.append(&section_card("Runtime", &runtime_summary_lines(status)));
        list.append(&section_card("Runtime Nodes", &runtime_node_lines(status)));
        list.append(&section_card(
            "Runtime Topology",
            &runtime_topology_lines(status),
        ));
        list.append(&section_card("Snapshots", &runtime_snapshot_lines(status)));
        list.append(&section_card(
            "Recent Runtime Events",
            &runtime_event_lines(status),
        ));
    }
    if let Some(recipe) = &snapshot.recipe {
        list.append(&section_card("Overview", &overview_lines(recipe)));
        list.append(&section_card("Startup Order", &recipe.startup.order));
        for element in &recipe.elements {
            list.append(&section_card(
                &format!("Element {}", element.id),
                &element_lines(element),
            ));
        }
    }
}

fn execute_runtime_command(command: RuntimeCommand) -> Result<(), String> {
    let project = load_workspace_project()?;
    QemuRuntime::default()
        .execute(&project, command)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn run_project_runtime() -> Result<(), String> {
    let project = load_workspace_project()?;
    let runtime = QemuRuntime::default();
    let status = runtime
        .status(&project)
        .map_err(|error| error.to_string())?;
    if status.vm_state == RuntimeVmState::Paused {
        runtime
            .execute(&project, RuntimeCommand::Resume)
            .map_err(|error| error.to_string())?;
    } else {
        if !status.vm_assets.prepared {
            runtime
                .execute(
                    &project,
                    RuntimeCommand::PrepareVm {
                        source_image: None,
                        size_gb: 8,
                    },
                )
                .map_err(|error| error.to_string())?;
        }
        runtime
            .execute(&project, RuntimeCommand::Boot)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn append_empty_recent_row(list: &ListBox) {
    list.append(&section_card(
        "No Recent Projects",
        &[String::from(
            "Create a project or open an existing sim-rns project.",
        )],
    ));
}

fn open_recent_project(
    host: &maruzzella_sdk::ffi::MzHostApi,
    project: &ProjectHandle,
    error_label: &Label,
) {
    match load_project(&project.path).and_then(|project| open_selected_project(host, &project)) {
        Ok(()) => set_error(error_label, ""),
        Err(error) => set_error(error_label, &error),
    }
}

fn append_recent_row(
    list: &ListBox,
    host: maruzzella_sdk::ffi::MzHostApi,
    project: &ProjectHandle,
    error_label: &Label,
) {
    let row = ListBoxRow::new();
    row.set_activatable(true);
    row.set_selectable(false);
    let hbox = GtkBox::new(Orientation::Horizontal, 12);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);

    let info = GtkBox::new(Orientation::Vertical, 2);
    info.set_hexpand(true);

    let title = Label::new(Some(&project.display_name));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class(&text_css_class("body-strong"));

    let path = Label::new(Some(&project.path));
    path.set_xalign(0.0);
    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    path.add_css_class(&text_css_class("meta"));

    info.append(&title);
    info.append(&path);

    let info_click = GestureClick::new();
    let info_host = host;
    let info_project = project.clone();
    let info_error = error_label.clone();
    info_click.connect_released(move |_, _, _, _| {
        open_recent_project(&info_host, &info_project, &info_error);
    });
    info.add_controller(info_click);

    let button = Button::with_label("Open");
    button.set_valign(Align::Center);
    button.add_css_class(&button_css_class("secondary"));

    let button_host = host;
    let button_project = project.clone();
    let button_error = error_label.clone();
    button.connect_clicked(move |_| {
        open_recent_project(&button_host, &button_project, &button_error);
    });

    let row_host = host;
    let row_project = project.clone();
    let row_error = error_label.clone();
    row.connect_activate(move |_| {
        open_recent_project(&row_host, &row_project, &row_error);
    });

    hbox.append(&info);
    hbox.append(&button);
    row.set_child(Some(&hbox));
    list.append(&row);
}

fn refresh_recent_projects(
    list: &ListBox,
    host: *const maruzzella_sdk::ffi::MzHostApi,
    error_label: &Label,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let Some(host_ref) = (unsafe { host.as_ref() }) else {
        append_empty_recent_row(list);
        set_error(error_label, "Launcher host API is unavailable.");
        return;
    };

    let config = load_config(host_ref);
    if config.recent_projects.is_empty() {
        append_empty_recent_row(list);
        return;
    }

    for project in &config.recent_projects {
        append_recent_row(list, *host_ref, project, error_label);
    }
}

extern "C" fn create_launcher_view(
    host: *const maruzzella_sdk::ffi::MzHostApi,
    _request: *const maruzzella_sdk::ffi::MzViewRequest,
) -> *mut std::ffi::c_void {
    let Some(host_ref) = (unsafe { host.as_ref() }) else {
        return std::ptr::null_mut();
    };
    if !gtk::is_initialized_main_thread() && gtk::init().is_err() {
        return std::ptr::null_mut();
    }
    let root = GtkBox::new(Orientation::Vertical, 0);

    let error_label = Label::new(None);
    error_label.set_xalign(0.0);
    error_label.set_wrap(true);
    error_label.add_css_class("error");
    error_label.set_visible(false);
    root.append(&error_label);

    let body = GtkBox::new(Orientation::Horizontal, 0);
    body.set_hexpand(true);
    body.set_vexpand(true);
    root.append(&body);

    // --- Left column: Recent Projects ---
    let recents_column = GtkBox::new(Orientation::Vertical, 12);
    recents_column.set_size_request(320, -1);
    recents_column.set_vexpand(true);
    recents_column.set_margin_top(20);
    recents_column.set_margin_bottom(20);
    recents_column.set_margin_start(16);
    recents_column.set_margin_end(16);
    recents_column.add_css_class(&surface_css_class("secondary"));
    body.append(&recents_column);

    let recents_header = GtkBox::new(Orientation::Horizontal, 8);
    let recents_title = Label::new(Some("Recent Projects"));
    recents_title.set_xalign(0.0);
    recents_title.set_hexpand(true);
    recents_title.add_css_class(&text_css_class("section-label"));
    recents_header.append(&recents_title);
    recents_column.append(&recents_header);

    let recent_projects = ListBox::new();
    recent_projects.set_selection_mode(SelectionMode::None);
    recent_projects.set_activate_on_single_click(true);
    refresh_recent_projects(&recent_projects, host, &error_label);

    let (recents_frame, recents_panel) = build_panel_frame();
    recents_panel.set_spacing(0);
    recents_panel.set_margin_top(0);
    recents_panel.set_margin_bottom(0);
    recents_panel.set_margin_start(0);
    recents_panel.set_margin_end(0);
    let recents_scroller = create_scroller();
    recents_scroller.set_vexpand(true);
    recents_scroller.set_child(Some(&recent_projects));
    recents_panel.append(&recents_scroller);
    recents_column.append(&recents_frame);

    // --- Vertical divider ---
    let divider = Separator::new(Orientation::Vertical);
    body.append(&divider);

    // --- Right column: Branding (top) + Actions (bottom) ---
    let actions_column = GtkBox::new(Orientation::Vertical, 0);
    actions_column.set_size_request(480, -1);
    actions_column.set_hexpand(true);
    actions_column.set_vexpand(true);
    actions_column.set_margin_top(24);
    actions_column.set_margin_bottom(24);
    actions_column.set_margin_start(28);
    actions_column.set_margin_end(28);
    body.append(&actions_column);

    // Top half: branding, centered
    let branding = GtkBox::new(Orientation::Vertical, 12);
    branding.set_vexpand(true);
    branding.set_valign(Align::Center);
    branding.set_halign(Align::Center);

    let icon_file =
        gio::File::for_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../sim-rns-icon.svg"));
    let icon_picture = Picture::for_file(&icon_file);
    icon_picture.set_can_shrink(true);
    icon_picture.set_halign(Align::Center);
    icon_picture.set_valign(Align::Center);
    let icon_container = GtkBox::new(Orientation::Vertical, 0);
    icon_container.set_size_request(96, 96);
    icon_container.set_halign(Align::Center);
    icon_container.set_valign(Align::Center);
    icon_container.set_hexpand(false);
    icon_container.set_vexpand(false);
    icon_container.append(&icon_picture);

    let product_title = Label::new(Some("SIM RNS"));
    product_title.add_css_class(&text_css_class("title"));

    let version_label = Label::new(Some("v0.1.0"));
    version_label.add_css_class(&text_css_class("meta"));

    branding.append(&icon_container);
    branding.append(&product_title);
    branding.append(&version_label);
    actions_column.append(&branding);

    // Bottom half: action buttons, centered
    let actions_box = GtkBox::new(Orientation::Vertical, 10);
    actions_box.set_vexpand(true);
    actions_box.set_valign(Align::Center);
    actions_box.set_halign(Align::Center);

    let open_local = Button::with_label("Open Local Project");
    open_local.add_css_class(&button_css_class("primary"));

    let open_remote = Button::with_label("Open Remote Project");
    open_remote.add_css_class(&button_css_class("secondary"));
    open_remote.set_sensitive(false);

    let create_project_button = Button::with_label("Create New Project");
    create_project_button.add_css_class(&button_css_class("secondary"));

    actions_box.append(&open_local);
    actions_box.append(&open_remote);
    actions_box.append(&create_project_button);
    actions_column.append(&actions_box);

    let host_copy = *host_ref;
    let recent_projects_copy = recent_projects.clone();
    let error_label_copy = error_label.clone();
    open_local.connect_clicked(move |button| {
        set_error(&error_label_copy, "");
        let parent = button
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        let host_for_dialog = host_copy;
        let recent_projects_for_dialog = recent_projects_copy.clone();
        let error_label_for_dialog = error_label_copy.clone();
        prompt_directory_picker(
            parent.as_ref(),
            "Open Local Project",
            "Open",
            &home_dir_or_root(),
            move |path| {
                if path.as_os_str().is_empty() {
                    set_error(
                        &error_label_for_dialog,
                        "The selected location has no local path.",
                    );
                    return;
                }
                match load_project(&path)
                    .and_then(|project| open_selected_project(&host_for_dialog, &project))
                {
                    Ok(()) => {
                        refresh_recent_projects(
                            &recent_projects_for_dialog,
                            &host_for_dialog,
                            &error_label_for_dialog,
                        );
                        set_error(&error_label_for_dialog, "");
                    }
                    Err(error) => set_error(&error_label_for_dialog, &error),
                }
            },
        );
    });

    let host_copy = *host_ref;
    let recent_projects_copy = recent_projects.clone();
    let error_label_copy = error_label.clone();
    create_project_button.connect_clicked(move |button| {
        set_error(&error_label_copy, "");
        let parent = button
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        let host_for_dialog = host_copy;
        let recent_projects_for_dialog = recent_projects_copy.clone();
        let error_label_for_dialog = error_label_copy.clone();
        prompt_directory_picker(
            parent.as_ref(),
            "Create New Project",
            "Create",
            &home_dir_or_root(),
            move |path| {
                if path.as_os_str().is_empty() {
                    set_error(
                        &error_label_for_dialog,
                        "The selected location has no local path.",
                    );
                    return;
                }

                let project_name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Sim RNS Project")
                    .to_string();

                match create_project(&path, &project_name)
                    .and_then(|project| open_selected_project(&host_for_dialog, &project))
                {
                    Ok(()) => {
                        refresh_recent_projects(
                            &recent_projects_for_dialog,
                            &host_for_dialog,
                            &error_label_for_dialog,
                        );
                        set_error(&error_label_for_dialog, "");
                    }
                    Err(error) => set_error(&error_label_for_dialog, &error),
                }
            },
        );
    });

    unsafe {
        <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
            as *mut std::ffi::c_void
    }
}

extern "C" fn create_overview_view(
    _host: *const maruzzella_sdk::ffi::MzHostApi,
    _request: *const maruzzella_sdk::ffi::MzViewRequest,
) -> *mut std::ffi::c_void {
    if !gtk::is_initialized_main_thread() && gtk::init().is_err() {
        return std::ptr::null_mut();
    }

    let snapshot =
        RUNTIME_CONTROLLER.with(|controller| controller.latest_or_refresh(load_runtime_snapshot));
    let title = snapshot
        .project
        .as_ref()
        .map(|project| project.file.name.as_str())
        .unwrap_or("Open a project");
    let subtitle = if snapshot.project.is_some() {
        "The workspace is bound to the selected project root and drives the simulated runtime contract that will later be backed by the VM."
    } else {
        snapshot
            .error
            .as_deref()
            .unwrap_or("Select or create a project to inspect the runtime.")
    };
    let root = build_root(title, subtitle);
    let error_label = workspace_error_label();
    root.append(&error_label);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);
    populate_overview_from_snapshot(&list, &error_label, &snapshot);

    let list_for_updates = list.downgrade();
    let error_label_for_updates = error_label.downgrade();
    let subscription = RUNTIME_CONTROLLER.with(|controller| {
        controller.subscribe(Rc::new(move |snapshot| {
            let Some(list) = list_for_updates.upgrade() else {
                return;
            };
            let Some(error_label) = error_label_for_updates.upgrade() else {
                return;
            };
            populate_overview_from_snapshot(&list, &error_label, snapshot);
        }))
    });
    unsafe {
        root.set_data("sim-rns-runtime-subscription", subscription);
    }

    let scroller = create_scroller();
    scroller.set_child(Some(&list));
    root.append(&scroller);

    unsafe {
        <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
            as *mut std::ffi::c_void
    }
}

extern "C" fn create_recipe_view(
    _host: *const maruzzella_sdk::ffi::MzHostApi,
    _request: *const maruzzella_sdk::ffi::MzViewRequest,
) -> *mut std::ffi::c_void {
    if !gtk::is_initialized_main_thread() && gtk::init().is_err() {
        return std::ptr::null_mut();
    }

    let project = match load_workspace_project() {
        Ok(project) => project,
        Err(error) => {
            let root = build_root("Open a project", &error);
            return unsafe {
                <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
                    as *mut std::ffi::c_void
            };
        }
    };
    let recipe = match project_recipe(&project) {
        Ok(recipe) => recipe,
        Err(error) => {
            let root = build_root("Project failed to load", &error);
            return unsafe {
                <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
                    as *mut std::ffi::c_void
            };
        }
    };
    let root = build_root(
        "Project File",
        "The root project file defines the VM envelope and imports companion files from the project tree. The recipe below is derived from that project definition, not stored as one monolithic blob.",
    );
    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);
    list.append(&section_card(
        "Metadata",
        &[
            format!("id = {}", recipe.metadata.id),
            format!("name = {}", recipe.metadata.name),
            format!("description = {}", recipe.metadata.description),
        ],
    ));
    list.append(&section_card(
        "Includes",
        &[
            format!("node files = {}", project.file.includes.nodes.join(", ")),
            format!("scripts = {}", project.file.includes.scripts.join(", ")),
            format!("configs = {}", project.file.includes.configs.join(", ")),
            format!("assets = {}", project.file.includes.assets.join(", ")),
        ],
    ));
    list.append(&section_card(
        "VM Setup",
        &[
            format!("base image = {}", recipe.vm.base_image),
            format!("os family = {}", recipe.vm.os_family),
            format!("ram = {} MiB", recipe.vm.ram_mb),
            format!("cpu cores = {}", recipe.vm.cpu_cores),
        ],
    ));
    list.append(&section_card(
        "Topology",
        &recipe
            .topology
            .attachments
            .iter()
            .map(|attachment| format!("{} -> {}", attachment.element_id, attachment.network_id))
            .collect::<Vec<_>>(),
    ));

    let scroller = create_scroller();
    scroller.set_child(Some(&list));
    root.append(&scroller);

    unsafe {
        <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
            as *mut std::ffi::c_void
    }
}

extern "C" fn create_templates_view(
    _host: *const maruzzella_sdk::ffi::MzHostApi,
    _request: *const maruzzella_sdk::ffi::MzViewRequest,
) -> *mut std::ffi::c_void {
    if !gtk::is_initialized_main_thread() && gtk::init().is_err() {
        return std::ptr::null_mut();
    }

    let project = match load_workspace_project() {
        Ok(project) => project,
        Err(error) => {
            let root = build_root("Open a project", &error);
            return unsafe {
                <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
                    as *mut std::ffi::c_void
            };
        }
    };
    let recipe = match project_recipe(&project) {
        Ok(recipe) => recipe,
        Err(error) => {
            let root = build_root("Project failed to load", &error);
            return unsafe {
                <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
                    as *mut std::ffi::c_void
            };
        }
    };
    let root = build_root(
        "Templates",
        "Templates remain shared runtime definitions, while the current project selects and configures them through imported node and script files.",
    );
    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);

    for template in &recipe.templates {
        list.append(&section_card(&template.label, &template_lines(template)));
    }

    let scroller = create_scroller();
    scroller.set_child(Some(&list));
    root.append(&scroller);

    unsafe {
        <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
            as *mut std::ffi::c_void
    }
}

export_plugin!(SimRnsPlugin);
