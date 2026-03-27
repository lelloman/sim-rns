use gtk::glib::translate::IntoGlibPtr;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Label, ListBox, ListBoxRow, Orientation, PolicyType, ScrolledWindow,
    SelectionMode,
};
use maruzzella_sdk::{
    export_plugin, HostApi, MzStatusCode, MzViewPlacement, Plugin, PluginDependency,
    PluginDescriptor, SurfaceContributionSpec, Version, ViewFactorySpec,
};
use sim_rns_core::{sample_recipe, Element, Recipe, Template};

const PLUGIN_ID: &str = "com.lelloman.sim_rns";
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
        format!("Project model: one project = one dedicated QEMU VM"),
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
        format!(
            "Snapshots are the continuity mechanism once arbitrary processes start evolving"
        ),
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
        &vec![
            format!("id = {}", recipe.metadata.id),
            format!("name = {}", recipe.metadata.name),
            format!("description = {}", recipe.metadata.description),
        ],
    ));
    list.append(&section_card(
        "VM Setup",
        &vec![
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
