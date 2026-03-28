use gtk::glib::translate::IntoGlibPtr;
use gtk::prelude::*;
use gtk::{
    gio, gdk, Align, Box as GtkBox, Button, CssProvider, Frame, Label, ListBox, ListBoxRow,
    Orientation, Picture, PolicyType, ScrolledWindow, SelectionMode, Separator,
    STYLE_PROVIDER_PRIORITY_USER,
};
use maruzzella_sdk::{
    export_plugin, HostApi, MzStatusCode, MzViewPlacement, Plugin, PluginDependency,
    PluginDescriptor, SurfaceContributionSpec, Version, ViewFactorySpec,
};
use sim_rns_core::{
    create_project, load_project, open_project, sample_recipe, Element, LauncherConfig, Project,
    ProjectHandle, Recipe, Template,
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
        host.log(maruzzella_sdk::ffi::MzLogLevel::Info, "Registering Sim RNS plugin");

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
        .map(|limits| format!("{} MiB / cpu weight {}", limits.memory_mb, limits.cpu_weight))
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
        format!("image features = {}", template.runtime.image_features.join(", ")),
        format!("default command = {}", template.defaults.command.join(" ")),
    ]
}

fn load_config(host: &maruzzella_sdk::ffi::MzHostApi) -> LauncherConfig {
    HostApi::from_raw(host)
        .read_json_config::<LauncherConfig>()
        .unwrap_or_default()
}

fn save_config(
    host: &maruzzella_sdk::ffi::MzHostApi,
    config: &LauncherConfig,
) -> Result<(), MzStatusCode> {
    HostApi::from_raw(host).write_json_config(config, Some(CONFIG_SCHEMA_VERSION))
}

fn current_dir_or_home() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| {
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()))
    })
}

fn set_error(label: &Label, message: &str) {
    label.set_label(message);
    label.set_visible(!message.is_empty());
}

fn install_launcher_css() {
    let provider = CssProvider::new();
    provider.load_from_data(
        "
        .sim-rns-launcher {
            padding: 0;
        }
        .sim-rns-launcher-header {
            margin-bottom: 0;
        }
        .sim-rns-launcher-title {
            font-size: 20px;
            font-weight: 700;
            letter-spacing: -0.01em;
        }

        /* --- Recents column (left sidebar) --- */
        .sim-rns-recents-column {
            padding: 20px 16px;
        }
        .sim-rns-recents-panel {
            border-radius: 0;
            background: transparent;
            box-shadow: none;
            border: none;
        }
        .sim-rns-recents-title {
            font-size: 13px;
            font-weight: 700;
            letter-spacing: 0.04em;
            opacity: 0.55;
            text-transform: uppercase;
        }
        .sim-rns-recents-list {
            background: transparent;
        }
        .sim-rns-recents-list row {
            margin: 0;
            padding: 0;
            border-radius: 8px;
            background: transparent;
            transition: background 150ms ease;
        }
        .sim-rns-recents-list row:hover {
            background: alpha(currentColor, 0.06);
        }
        .sim-rns-recent-row {
            padding: 10px 12px;
        }
        .sim-rns-recent-info {
            min-width: 0;
        }
        .sim-rns-recent-title {
            font-size: 14px;
            font-weight: 600;
        }
        .sim-rns-recent-path {
            font-size: 11px;
            opacity: 0.50;
            font-family: monospace;
        }
        .sim-rns-recent-open-btn {
            opacity: 0;
            transition: opacity 150ms ease;
            min-height: 28px;
            min-width: 56px;
            font-size: 12px;
        }
        .sim-rns-recents-list row:hover .sim-rns-recent-open-btn {
            opacity: 1;
        }

        /* --- Right column: branding + actions --- */
        .sim-rns-actions-column {
            padding: 24px 28px;
        }
        .sim-rns-branding {
            padding: 16px;
        }
        .sim-rns-brand-title {
            font-size: 28px;
            font-weight: 800;
            letter-spacing: 0.08em;
        }
        .sim-rns-brand-version {
            font-size: 13px;
            font-weight: 500;
            opacity: 0.5;
            letter-spacing: 0.04em;
        }
        .sim-rns-actions-box {
            padding: 16px;
        }
        .sim-rns-action-btn {
            min-height: 40px;
            min-width: 240px;
            font-weight: 600;
            font-size: 14px;
            border-radius: 8px;
        }

        /* --- Divider between columns --- */
        .sim-rns-column-divider {
            background: alpha(currentColor, 0.10);
            min-width: 1px;
        }

        .sim-rns-error {
            margin: 8px 16px 0 16px;
            padding: 8px 12px;
            border-radius: 6px;
            background: alpha(@error_color, 0.10);
            color: @error_color;
            font-weight: 600;
            font-size: 13px;
        }
        ",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            STYLE_PROVIDER_PRIORITY_USER + 1,
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
    let mut config = load_config(host);
    config.remember_project(handle.clone());
    save_config(host, &config).map_err(|status| format!("failed to save recents: {status:?}"))?;
    open_project(handle)
}

fn append_empty_recent_row(list: &ListBox) {
    list.append(&section_card(
        "No Recent Projects",
        &[String::from("Create a project or open an existing sim-rns project.")],
    ));
}

fn append_recent_row(
    list: &ListBox,
    host: *const maruzzella_sdk::ffi::MzHostApi,
    project: &ProjectHandle,
    error_label: &Label,
) {
    let row = ListBoxRow::new();
    let hbox = GtkBox::new(Orientation::Horizontal, 12);
    hbox.add_css_class("sim-rns-recent-row");

    let info = GtkBox::new(Orientation::Vertical, 2);
    info.set_hexpand(true);
    info.add_css_class("sim-rns-recent-info");

    let title = Label::new(Some(&project.display_name));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("sim-rns-recent-title");

    let path = Label::new(Some(&project.path));
    path.set_xalign(0.0);
    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    path.add_css_class("sim-rns-recent-path");

    info.append(&title);
    info.append(&path);

    let button = Button::with_label("Open");
    button.set_valign(Align::Center);
    button.add_css_class("sim-rns-recent-open-btn");

    let host_copy = host;
    let project_copy = project.clone();
    let error_copy = error_label.clone();
    button.connect_clicked(move |_| {
        let Some(host_ref) = (unsafe { host_copy.as_ref() }) else {
            set_error(&error_copy, "Launcher host API is unavailable.");
            return;
        };
        match load_project(&project_copy.path)
            .and_then(|project| open_selected_project(host_ref, &project))
        {
            Ok(()) => set_error(&error_copy, ""),
            Err(error) => set_error(&error_copy, &error),
        }
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
        append_recent_row(list, host, project, error_label);
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
    root.add_css_class("sim-rns-launcher");

    let error_label = Label::new(None);
    error_label.set_xalign(0.0);
    error_label.set_wrap(true);
    error_label.add_css_class("sim-rns-error");
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
    recents_column.add_css_class("sim-rns-recents-column");
    body.append(&recents_column);

    let recents_header = GtkBox::new(Orientation::Horizontal, 8);
    let recents_title = Label::new(Some("Recent Projects"));
    recents_title.set_xalign(0.0);
    recents_title.set_hexpand(true);
    recents_title.add_css_class("sim-rns-recents-title");
    recents_header.append(&recents_title);
    recents_column.append(&recents_header);

    let recent_projects = ListBox::new();
    recent_projects.add_css_class("sim-rns-recents-list");
    recent_projects.set_selection_mode(SelectionMode::None);
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
    divider.add_css_class("sim-rns-column-divider");
    body.append(&divider);

    // --- Right column: Branding (top) + Actions (bottom) ---
    let actions_column = GtkBox::new(Orientation::Vertical, 0);
    actions_column.set_size_request(480, -1);
    actions_column.set_hexpand(true);
    actions_column.set_vexpand(true);
    actions_column.add_css_class("sim-rns-actions-column");
    body.append(&actions_column);

    // Top half: branding, centered
    let branding = GtkBox::new(Orientation::Vertical, 12);
    branding.set_vexpand(true);
    branding.set_valign(Align::Center);
    branding.set_halign(Align::Center);
    branding.add_css_class("sim-rns-branding");

    let icon_bytes = gtk::glib::Bytes::from_static(include_bytes!("../../sim-rns-icon.svg"));
    let icon_texture = gdk::Texture::from_bytes(&icon_bytes).expect("failed to load app icon");
    let icon_picture = Picture::for_paintable(&icon_texture);
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
    product_title.add_css_class("sim-rns-brand-title");

    let version_label = Label::new(Some("v0.1.0"));
    version_label.add_css_class("sim-rns-brand-version");

    branding.append(&icon_container);
    branding.append(&product_title);
    branding.append(&version_label);
    actions_column.append(&branding);

    // Bottom half: action buttons, centered
    let actions_box = GtkBox::new(Orientation::Vertical, 10);
    actions_box.set_vexpand(true);
    actions_box.set_valign(Align::Center);
    actions_box.set_halign(Align::Center);
    actions_box.add_css_class("sim-rns-actions-box");

    let open_local = Button::with_label("Open Local Project");
    open_local.add_css_class("suggested-action");
    open_local.add_css_class("sim-rns-action-btn");

    let open_remote = Button::with_label("Open Remote Project");
    open_remote.add_css_class("sim-rns-action-btn");
    open_remote.set_sensitive(false);

    let create_project_button = Button::with_label("Create New Project");
    create_project_button.add_css_class("sim-rns-action-btn");

    actions_box.append(&open_local);
    actions_box.append(&open_remote);
    actions_box.append(&create_project_button);
    actions_column.append(&actions_box);

    let host_copy = *host_ref;
    let recent_projects_copy = recent_projects.clone();
    let error_label_copy = error_label.clone();
    open_local.connect_clicked(move |button| {
        set_error(&error_label_copy, "");
        let dialog = gtk::FileDialog::builder()
            .title("Open Local Project")
            .initial_folder(&gio::File::for_path(current_dir_or_home()))
            .build();
        let parent = button
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        let host_for_dialog = host_copy;
        let recent_projects_for_dialog = recent_projects_copy.clone();
        let error_label_for_dialog = error_label_copy.clone();
        dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                set_error(&error_label_for_dialog, "The selected location has no local path.");
                return;
            };
            match load_project(&path).and_then(|project| open_selected_project(&host_for_dialog, &project)) {
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
        });
    });

    let host_copy = *host_ref;
    let recent_projects_copy = recent_projects.clone();
    let error_label_copy = error_label.clone();
    create_project_button.connect_clicked(move |button| {
        set_error(&error_label_copy, "");
        let dialog = gtk::FileDialog::builder()
            .title("Create New Project")
            .initial_folder(&gio::File::for_path(current_dir_or_home()))
            .build();
        let parent = button
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        let host_for_dialog = host_copy;
        let recent_projects_for_dialog = recent_projects_copy.clone();
        let error_label_for_dialog = error_label_copy.clone();
        dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                set_error(&error_label_for_dialog, "The selected location has no local path.");
                return;
            };

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
        });
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

    let recipe = sample_recipe();
    let root = build_root(
        "Reticulum Network Simulator",
        "The current implementation is a Maruzzella-hosted scaffold centered on recipe-driven VM initialization and template-backed elements.",
    );
    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);
    list.append(&section_card("Overview", &overview_lines(&recipe)));
    list.append(&section_card("Startup Order", &recipe.startup.order));

    for element in &recipe.elements {
        list.append(&section_card(&format!("Element {}", element.id), &element_lines(element)));
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

    let recipe = sample_recipe();
    let root = build_root(
        "Recipe",
        "Recipes are the structured DNA that initialize a project. They describe the VM envelope, templates, elements, topology, and startup order, but not the full evolving runtime state of the VM.",
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

    let recipe = sample_recipe();
    let root = build_root(
        "Templates",
        "Templates are runtime/domain objects. The app uses template IDs to map icons and presentation separately from the recipe model.",
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
