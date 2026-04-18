use gtk::prelude::ApplicationExtManual;
use maruzzella::{
    build_application_with_handle, default_product_spec, load_static_plugin, plugin_tab,
    LauncherSpec, MaruzzellaConfig, MenuItemSpec, MenuRootSpec, ShellMode, TabGroupSpec,
    WindowPolicy, WorkbenchNodeSpec, WorkspaceSession,
};
use sim_rns_core::{install_project_closer, install_project_opener, set_active_project_handle};

fn main() {
    let mut product = default_product_spec();
    product.branding.title = "Sim RNS".to_string();
    product.branding.search_placeholder = "Search simulator views".to_string();
    product.branding.status_text =
        "Selected local project loaded into the simulator scaffold".to_string();
    product.include_base_toolbar_items = true;
    product.menu_roots = root_menu_roots();
    product.menu_items = root_menu_items();

    product.layout.workbench = WorkbenchNodeSpec::Group(TabGroupSpec::new(
        "workbench-main",
        Some("overview"),
        vec![
            plugin_tab(
                "overview",
                "workbench-main",
                "Overview",
                "com.lelloman.sim_rns.overview",
                "The simulator overview could not be created.",
                false,
            ),
            plugin_tab(
                "recipe",
                "workbench-main",
                "Recipe",
                "com.lelloman.sim_rns.recipe",
                "The recipe view could not be created.",
                false,
            ),
            plugin_tab(
                "templates",
                "workbench-main",
                "Templates",
                "com.lelloman.sim_rns.templates",
                "The templates view could not be created.",
                false,
            ),
        ],
    ));

    let launcher = LauncherSpec::new(
        "Sim RNS",
        TabGroupSpec::new(
            "launcher-home",
            Some("project-opener"),
            vec![plugin_tab(
                "project-opener",
                "launcher-home",
                "Project Opener",
                "com.lelloman.sim_rns.launcher",
                "The project opener could not be created.",
                false,
            )],
        )
        .with_tab_strip_hidden(),
    );

    let config = MaruzzellaConfig::new("com.lelloman.sim-rns")
        .with_persistence_id("sim-rns")
        .with_startup_mode(ShellMode::Launcher)
        .with_launcher(launcher)
        .with_launcher_window_policy(WindowPolicy::new(980, 720))
        .with_product(product)
        .with_builtin_plugin(embedded_sim_rns_plugin);

    let workspace_product = config.product.clone();
    let (app, handle) = build_application_with_handle(config);
    let launcher_handle = handle.clone();
    install_project_closer(move || {
        set_active_project_handle(None);
        launcher_handle
            .switch_to_launcher()
            .map_err(|error| error.to_string())
    });
    install_project_opener(move |project_handle| {
        set_active_project_handle(Some(project_handle.clone()));
        let project_handle_bytes = project_handle.to_bytes()?;
        handle
            .switch_to_workspace(WorkspaceSession {
                project_handle: Some(project_handle_bytes),
                shell_spec: Some(workspace_product.shell_spec()),
                window_policy: None,
            })
            .map_err(|error| error.to_string())
    });
    app.run();
}

fn embedded_sim_rns_plugin() -> Result<maruzzella::LoadedPlugin, maruzzella::PluginLoadError> {
    load_static_plugin(
        "builtin:sim-rns-plugin",
        sim_rns_plugin::maruzzella_plugin_entry,
    )
}

fn root_menu_roots() -> Vec<MenuRootSpec> {
    vec![
        MenuRootSpec {
            id: "file".to_string(),
            label: "File".to_string(),
        },
        MenuRootSpec {
            id: "view".to_string(),
            label: "View".to_string(),
        },
        MenuRootSpec {
            id: "help".to_string(),
            label: "Help".to_string(),
        },
    ]
}

fn root_menu_items() -> Vec<MenuItemSpec> {
    vec![
        menu_item(
            "sim-rns.project.new",
            "file",
            "New Project",
            "sim-rns.project.new",
        ),
        menu_item(
            "sim-rns.project.open",
            "file",
            "Open Project",
            "sim-rns.project.open",
        ),
        menu_item(
            "sim-rns.project.close",
            "file",
            "Close Project",
            "sim-rns.project.close",
        ),
        menu_separator("file-project-separator", "file"),
        menu_item("new-buffer", "file", "New Buffer", "shell.new_buffer"),
        menu_item("save-buffer", "file", "Save Buffer", "shell.save_buffer"),
        menu_item(
            "save-buffer-as",
            "file",
            "Save Buffer as",
            "shell.save_buffer_as",
        ),
        menu_separator("file-buffer-separator", "file"),
        menu_item("settings", "file", "Settings", "shell.settings"),
        menu_item("plugins", "file", "Plugins", "shell.plugins"),
        menu_separator("file-shell-separator", "file"),
        menu_item("sim-rns.app.exit", "file", "Exit", "sim-rns.app.exit"),
        menu_item(
            "command-palette",
            "view",
            "Command Palette",
            "shell.open_command_palette",
        ),
        menu_item("reload-theme", "view", "Reload Theme", "shell.reload_theme"),
        menu_item("browse-views", "view", "Browse Views", "shell.browse_views"),
        menu_item("about", "help", "About", "shell.about"),
    ]
}

fn menu_item(id: &str, root_id: &str, label: &str, command_id: &str) -> MenuItemSpec {
    MenuItemSpec {
        id: id.to_string(),
        root_id: root_id.to_string(),
        label: label.to_string(),
        command_id: command_id.to_string(),
        payload: Vec::new(),
    }
}

fn menu_separator(id: &str, root_id: &str) -> MenuItemSpec {
    menu_item(id, root_id, "", "")
}
