use gtk::prelude::ApplicationExtManual;
use maruzzella::{
    build_application_with_handle, default_product_spec, load_static_plugin, plugin_tab,
    LauncherSpec, MaruzzellaConfig, ShellMode, TabGroupSpec, WindowPolicy, WorkspaceSession,
    WorkbenchNodeSpec,
};
use sim_rns_core::install_project_opener;

fn main() {
    let mut product = default_product_spec();
    product.branding.title = "Sim RNS".to_string();
    product.branding.search_placeholder = "Search simulator views".to_string();
    product.branding.status_text = "Selected local project loaded into the simulator scaffold".to_string();
    product.include_base_toolbar_items = true;

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
    install_project_opener(move |project_handle| {
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
