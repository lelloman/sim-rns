use maruzzella::{
    default_product_spec, load_static_plugin, plugin_tab, run, MaruzzellaConfig, TabGroupSpec,
    WorkbenchNodeSpec,
};

fn main() {
    let mut product = default_product_spec();
    product.branding.title = "Sim RNS".to_string();
    product.branding.search_placeholder = "Search simulator views".to_string();
    product.branding.status_text = "VM-backed Reticulum network simulator".to_string();
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

    let config = MaruzzellaConfig::new("com.lelloman.sim-rns")
        .with_persistence_id("sim-rns")
        .with_product(product)
        .with_builtin_plugin(embedded_sim_rns_plugin);

    run(config);
}

fn embedded_sim_rns_plugin() -> Result<maruzzella::LoadedPlugin, maruzzella::PluginLoadError> {
    load_static_plugin(
        "builtin:sim-rns-plugin",
        sim_rns_plugin::maruzzella_plugin_entry,
    )
}
