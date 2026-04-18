use gtk::glib::translate::IntoGlibPtr;
use gtk::prelude::*;
use gtk::{
    gio, Align, Box as GtkBox, Button, CssProvider, FileChooserAction, FileChooserDialog,
    Frame, GestureClick, Label, ListBox, ListBoxRow, Orientation, Picture, PolicyType, ResponseType,
    ScrolledWindow, SelectionMode, Separator,
};
use maruzzella_sdk::{
    button_css_class, export_plugin, surface_css_class, text_css_class, HostApi, MzStatusCode,
    MzViewPlacement, Plugin, PluginDependency, PluginDescriptor,
    SurfaceContributionSpec, Version, ViewFactorySpec,
};
use sim_rns_core::{
    add_node_include, add_script_include, close_project, create_project, current_project,
    load_project, open_project, project_recipe, Element, LauncherConfig,
    Project, ProjectHandle, Recipe, Template,
};

const PLUGIN_ID: &str = "com.lelloman.sim_rns";
const CONFIG_SCHEMA_VERSION: u32 = 1;
const VIEW_LAUNCHER: &str = "com.lelloman.sim_rns.launcher";
const VIEW_OVERVIEW: &str = "com.lelloman.sim_rns.overview";
const VIEW_RECIPE: &str = "com.lelloman.sim_rns.recipe";
const VIEW_TEMPLATES: &str = "com.lelloman.sim_rns.templates";

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
    let dialog = FileChooserDialog::new(
        Some(title),
        parent,
        FileChooserAction::SelectFolder,
        &[("Cancel", ResponseType::Cancel), (confirm_label, ResponseType::Accept)],
    );
    dialog.add_css_class("app-dialog");
    dialog.set_modal(true);
    if initial_path.is_dir() {
        let initial_folder = gio::File::for_path(initial_path);
        let _ = dialog.set_current_folder(Some(&initial_folder));
    }
    dialog.connect_response(move |dialog, response| {
        let selected_path = if response == ResponseType::Accept {
            dialog.file().and_then(|file| file.path())
        } else {
            None
        };
        dialog.close();
        if let Some(path) = selected_path {
            let on_selected = on_selected.clone();
            gtk::glib::idle_add_local_once(move || on_selected(path));
        }
    });
    dialog.show();
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

fn install_launcher_css() {
    let provider = CssProvider::new();
    provider.load_from_data(
        "
        .sim-rns-launcher-action {
            min-width: 240px;
            min-height: 42px;
            border-radius: 10px;
            transition: background 120ms ease, border-color 120ms ease, box-shadow 120ms ease;
        }
        .sim-rns-launcher-action:hover {
            box-shadow: inset 0 0 0 9999px alpha(currentColor, 0.05);
        }
        .sim-rns-launcher-action:active {
            box-shadow: inset 0 0 0 9999px alpha(currentColor, 0.10);
        }
        .sim-rns-launcher-action-primary:hover {
            box-shadow: inset 0 0 0 9999px alpha(white, 0.08);
        }
        .sim-rns-launcher-action-primary:active {
            box-shadow: inset 0 0 0 9999px alpha(black, 0.12);
        }
        .sim-rns-launcher-action-secondary:hover {
            box-shadow: inset 0 0 0 9999px alpha(currentColor, 0.07);
        }
        .sim-rns-launcher-action-secondary:active {
            box-shadow: inset 0 0 0 9999px alpha(currentColor, 0.12);
        }
        .sim-rns-recents-panel {
            border-radius: 0;
            box-shadow: none;
            border: none;
        }
        .sim-rns-recent-open-btn {
            opacity: 0;
            transition: opacity 150ms ease;
        }
        .sim-rns-recents-list row:hover .sim-rns-recent-open-btn {
            opacity: 1;
        }
        ",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
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

fn populate_overview_list(list: &ListBox, project: &Project, recipe: &Recipe) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    list.append(&section_card("Project", &build_project_summary(project)));
    list.append(&section_card("Overview", &overview_lines(recipe)));
    list.append(&section_card("Startup Order", &recipe.startup.order));

    for element in &recipe.elements {
        list.append(&section_card(
            &format!("Element {}", element.id),
            &element_lines(element),
        ));
    }
}

fn reload_overview(list: &ListBox, error_label: &Label) {
    match load_workspace_project().and_then(|project| {
        let recipe = project_recipe(&project)?;
        Ok((project, recipe))
    }) {
        Ok((project, recipe)) => {
            populate_overview_list(list, &project, &recipe);
            set_error(error_label, "");
        }
        Err(error) => set_error(error_label, &error),
    }
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
    button.add_css_class("sim-rns-recent-open-btn");

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
    install_launcher_css();

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
    recent_projects.add_css_class("sim-rns-recents-list");
    refresh_recent_projects(&recent_projects, host, &error_label);

    let (recents_frame, recents_panel) = build_panel_frame();
    recents_frame.add_css_class("sim-rns-recents-panel");
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
    open_local.add_css_class("sim-rns-launcher-action");
    open_local.add_css_class("sim-rns-launcher-action-primary");

    let open_remote = Button::with_label("Open Remote Project");
    open_remote.add_css_class(&button_css_class("secondary"));
    open_remote.add_css_class("sim-rns-launcher-action");
    open_remote.add_css_class("sim-rns-launcher-action-secondary");
    open_remote.set_sensitive(false);

    let create_project_button = Button::with_label("Create New Project");
    create_project_button.add_css_class(&button_css_class("secondary"));
    create_project_button.add_css_class("sim-rns-launcher-action");
    create_project_button.add_css_class("sim-rns-launcher-action-secondary");

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
        &project.file.name,
        "The current workspace is now bound to the selected project root and derives its scaffold recipe from the root file plus imported project files.",
    );
    let action_bar = GtkBox::new(Orientation::Horizontal, 8);
    let close_project_button = Button::with_label("Close Project");
    let add_script_button = Button::with_label("Add Script");
    let add_node_button = Button::with_label("Add Node");
    action_bar.append(&close_project_button);
    action_bar.append(&add_script_button);
    action_bar.append(&add_node_button);
    root.append(&action_bar);

    let error_label = workspace_error_label();
    root.append(&error_label);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);
    populate_overview_list(&list, &project, &recipe);

    let scroller = create_scroller();
    scroller.set_child(Some(&list));
    root.append(&scroller);

    let error_label_for_close = error_label.clone();
    close_project_button.connect_clicked(move |_| match close_project() {
        Ok(()) => set_error(&error_label_for_close, ""),
        Err(error) => set_error(&error_label_for_close, &error),
    });

    let list_for_script = list.clone();
    let error_label_for_script = error_label.clone();
    add_script_button.connect_clicked(move |_| {
        match load_workspace_project().and_then(|project| {
            let (_updated, relative_path) = add_script_include(&project.root_path)?;
            Ok(relative_path)
        }) {
            Ok(relative_path) => {
                reload_overview(&list_for_script, &error_label_for_script);
                set_error(
                    &error_label_for_script,
                    &format!("Added script include `{relative_path}`."),
                );
            }
            Err(error) => set_error(&error_label_for_script, &error),
        }
    });

    let list_for_node = list.clone();
    let error_label_for_node = error_label.clone();
    add_node_button.connect_clicked(move |_| {
        match load_workspace_project().and_then(|project| {
            let (_updated, relative_path) = add_node_include(&project.root_path)?;
            Ok(relative_path)
        }) {
            Ok(relative_path) => {
                reload_overview(&list_for_node, &error_label_for_node);
                set_error(
                    &error_label_for_node,
                    &format!("Added node include `{relative_path}`."),
                );
            }
            Err(error) => set_error(&error_label_for_node, &error),
        }
    });

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
