use gtk::glib::translate::IntoGlibPtr;
use gtk::prelude::*;
use gtk::{
    gio, Align, Box as GtkBox, Button, CssProvider, Frame, Label, ListBox, ListBoxRow,
    Orientation, PolicyType, ScrolledWindow, SelectionMode, Separator,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use maruzzella_sdk::{
    export_plugin, HostApi, MzStatusCode, MzViewPlacement, Plugin, PluginDependency,
    PluginDescriptor, SurfaceContributionSpec, Version, ViewFactorySpec,
};
use sim_rns_core::{
    open_project, sample_recipe, Element, LauncherConfig, ProjectHandle, Recipe, Template,
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
            padding: 12px;
        }
        .sim-rns-launcher-header {
            margin-bottom: 4px;
        }
        .sim-rns-launcher-title {
            font-size: 30px;
            font-weight: 700;
            letter-spacing: -0.03em;
        }
        .sim-rns-launcher-subtitle {
            font-size: 13px;
            opacity: 0.66;
        }
        .sim-rns-welcome-panel,
        .sim-rns-recents-panel {
            border-radius: 0;
            background: transparent;
            box-shadow: none;
            border: none;
        }
        .sim-rns-welcome-panel {
            min-width: 260px;
            background: alpha(black, 0.035);
            border-radius: 12px;
        }
        .sim-rns-mark {
            font-size: 11px;
            font-weight: 700;
            letter-spacing: 0.16em;
            opacity: 0.58;
        }
        .sim-rns-welcome-title {
            font-size: 16px;
            font-weight: 650;
        }
        .sim-rns-welcome-copy {
            font-size: 13px;
            line-height: 1.3;
            opacity: 0.74;
        }
        .sim-rns-primary-action {
            min-height: 36px;
            font-weight: 650;
        }
        .sim-rns-secondary-note {
            font-size: 12px;
            opacity: 0.6;
        }
        .sim-rns-recents-title {
            font-size: 22px;
            font-weight: 700;
            letter-spacing: -0.01em;
        }
        .sim-rns-recents-list row {
            margin: 0;
            border-radius: 0;
            background: transparent;
            border-bottom: 1px solid alpha(currentColor, 0.08);
        }
        .sim-rns-recents-list row:last-child {
            border-bottom: none;
        }
        .sim-rns-recent-title {
            font-size: 15px;
            font-weight: 650;
        }
        .sim-rns-recent-path {
            font-size: 12px;
            opacity: 0.62;
        }
        .sim-rns-error {
            padding: 8px 10px;
            border-radius: 6px;
            background: alpha(@error_color, 0.10);
            color: @error_color;
            font-weight: 600;
        }
        ",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
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

fn build_welcome_panel() -> (Frame, GtkBox) {
    let (frame, content) = build_panel_frame();
    content.set_spacing(14);
    (frame, content)
}

fn open_selected_project(
    host: &maruzzella_sdk::ffi::MzHostApi,
    handle: ProjectHandle,
) -> Result<(), String> {
    let mut config = load_config(host);
    config.remember_project(handle.clone());
    save_config(host, &config).map_err(|status| format!("failed to save recents: {status:?}"))?;
    open_project(handle)
}

fn append_empty_recent_row(list: &ListBox) {
    list.append(&section_card(
        "No Recent Projects",
        &[String::from(
            "Open a local directory once and it will appear here for quick re-entry.",
        )],
    ));
}

fn append_recent_row(
    list: &ListBox,
    host: *const maruzzella_sdk::ffi::MzHostApi,
    project: &ProjectHandle,
    error_label: &Label,
) {
    let row = ListBoxRow::new();
    let card = GtkBox::new(Orientation::Vertical, 8);
    card.set_margin_top(8);
    card.set_margin_bottom(8);
    card.set_margin_start(6);
    card.set_margin_end(6);

    let title = Label::new(Some(&project.display_name));
    title.set_xalign(0.0);
    title.add_css_class("sim-rns-recent-title");

    let path = Label::new(Some(&project.path));
    path.set_xalign(0.0);
    path.set_wrap(true);
    path.set_selectable(true);
    path.add_css_class("sim-rns-recent-path");

    let button = Button::with_label("Open");
    button.set_halign(Align::Start);

    let host_copy = host;
    let project_copy = project.clone();
    let error_copy = error_label.clone();
    button.connect_clicked(move |_| {
        let Some(host_ref) = (unsafe { host_copy.as_ref() }) else {
            set_error(&error_copy, "Launcher host API is unavailable.");
            return;
        };
        if let Err(error) = open_selected_project(host_ref, project_copy.clone()) {
            set_error(&error_copy, &error);
        }
    });

    card.append(&title);
    card.append(&path);
    card.append(&button);
    row.set_child(Some(&card));
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

    let root = GtkBox::new(Orientation::Vertical, 18);
    root.set_margin_top(24);
    root.set_margin_bottom(24);
    root.set_margin_start(24);
    root.set_margin_end(24);
    root.add_css_class("sim-rns-launcher");

    let header = GtkBox::new(Orientation::Vertical, 6);
    header.add_css_class("sim-rns-launcher-header");
    let title = Label::new(Some("Open a project"));
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.add_css_class("sim-rns-launcher-title");
    header.append(&title);
    root.append(&header);

    let error_label = Label::new(None);
    error_label.set_xalign(0.0);
    error_label.set_wrap(true);
    error_label.add_css_class("sim-rns-error");
    error_label.set_visible(false);
    root.append(&error_label);

    let body = GtkBox::new(Orientation::Horizontal, 18);
    body.set_hexpand(true);
    body.set_vexpand(true);
    root.append(&body);

    let recents_column = GtkBox::new(Orientation::Vertical, 12);
    recents_column.set_hexpand(true);
    recents_column.set_vexpand(true);
    body.append(&recents_column);

    let recents_header = GtkBox::new(Orientation::Vertical, 4);
    let recents_title = Label::new(Some("Recent Projects"));
    recents_title.set_xalign(0.0);
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
    let recents_scroller = create_scroller();
    recents_scroller.set_min_content_height(320);
    recents_scroller.set_child(Some(&recent_projects));
    recents_panel.append(&recents_scroller);
    recents_column.append(&recents_frame);

    let actions_column = GtkBox::new(Orientation::Vertical, 12);
    actions_column.set_size_request(240, -1);
    actions_column.set_vexpand(true);
    body.append(&actions_column);

    let (welcome_frame, welcome_panel) = build_welcome_panel();
    welcome_frame.add_css_class("sim-rns-welcome-panel");
    actions_column.append(&welcome_frame);

    let product_mark = Label::new(Some("SIM RNS"));
    product_mark.set_xalign(0.0);
    product_mark.add_css_class("sim-rns-mark");

    let welcome_title = Label::new(Some("Welcome"));
    welcome_title.set_xalign(0.0);
    welcome_title.add_css_class("sim-rns-welcome-title");

    let welcome_copy = Label::new(Some(
        "Choose a local directory to open the current simulator scaffold.",
    ));
    welcome_copy.set_xalign(0.0);
    welcome_copy.set_wrap(true);
    welcome_copy.add_css_class("sim-rns-welcome-copy");

    let open_local = Button::with_label("Open Local Project");
    open_local.set_halign(Align::Start);
    open_local.add_css_class("suggested-action");
    open_local.add_css_class("sim-rns-primary-action");

    let secondary_actions = GtkBox::new(Orientation::Vertical, 8);
    let open_remote = Button::with_label("Open Remote Project");
    open_remote.set_halign(Align::Start);
    open_remote.set_sensitive(false);

    let create_project = Button::with_label("Create Project");
    create_project.set_halign(Align::Start);
    create_project.set_sensitive(false);

    let staged_note = Label::new(Some("Remote attach and creation are staged next."));
    staged_note.set_xalign(0.0);
    staged_note.set_wrap(true);
    staged_note.add_css_class("sim-rns-secondary-note");

    secondary_actions.append(&open_remote);
    secondary_actions.append(&create_project);

    welcome_panel.append(&product_mark);
    welcome_panel.append(&welcome_title);
    welcome_panel.append(&welcome_copy);
    welcome_panel.append(&open_local);
    welcome_panel.append(&Separator::new(Orientation::Horizontal));
    welcome_panel.append(&secondary_actions);
    welcome_panel.append(&staged_note);

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
            match ProjectHandle::for_local_dir(&path)
                .and_then(|handle| open_selected_project(&host_for_dialog, handle))
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
