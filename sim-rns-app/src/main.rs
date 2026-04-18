use gtk::prelude::ApplicationExtManual;
use maruzzella::{
    build_application_with_handle, default_product_spec, load_static_plugin, plugin_tab,
    LauncherSpec, MaruzzellaConfig, MenuItemSpec, MenuRootSpec, ShellMode, TabGroupSpec,
    WindowPolicy, WorkbenchNodeSpec, WorkspaceSession,
};
use serde::{Deserialize, Serialize};
use sim_rns_core::{
    install_project_closer, install_project_opener, load_project, set_active_project_handle,
    ProjectHandle,
};

const SESSION_SCHEMA_VERSION: u32 = 1;
const MAIN_WINDOW_ID: &str = "main";
const WORKSPACE_LAYOUT_SLOT: &str = "workspace";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppSession {
    schema_version: u32,
    windows: Vec<AppSessionWindow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppSessionWindow {
    id: String,
    project: Option<ProjectHandle>,
    layout_slot: String,
}

fn main() {
    let restored_project = load_saved_project_session();
    let mut product = default_product_spec();
    product.branding.title = "Sim RNS".to_string();
    product.branding.search_placeholder = "Search simulator views".to_string();
    product.branding.status_text =
        "Selected local project loaded into the simulator scaffold".to_string();
    product.include_base_toolbar_items = true;
    product.menu_roots = root_menu_roots();
    product.menu_items = root_menu_items();
    if let Err(error) = sync_persisted_root_menu(&product.menu_roots, &product.menu_items) {
        eprintln!("sim-rns: failed to update persisted root menu: {error}");
    }

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
        .with_startup_mode(if restored_project.is_some() {
            ShellMode::Workspace
        } else {
            ShellMode::Launcher
        })
        .with_launcher(launcher)
        .with_launcher_window_policy(WindowPolicy::new(980, 720))
        .with_product(product)
        .with_builtin_plugin(embedded_sim_rns_plugin);

    if let Some(project_handle) = restored_project.clone() {
        set_active_project_handle(Some(project_handle));
    }

    let workspace_product = config.product.clone();
    let (app, handle) = build_application_with_handle(config);
    let launcher_handle = handle.clone();
    install_project_closer(move || {
        set_active_project_handle(None);
        clear_saved_project_session();
        launcher_handle
            .switch_to_launcher()
            .map_err(|error| error.to_string())
    });
    install_project_opener(move |project_handle| {
        set_active_project_handle(Some(project_handle.clone()));
        let project_handle_bytes = project_handle.to_bytes()?;
        let result = handle
            .switch_to_workspace(WorkspaceSession {
                project_handle: Some(project_handle_bytes),
                shell_spec: Some(workspace_product.shell_spec()),
                window_policy: None,
            })
            .map_err(|error| error.to_string());
        if result.is_ok() {
            if let Err(error) = save_project_session(&project_handle) {
                eprintln!("sim-rns: failed to save project session: {error}");
            }
        } else {
            set_active_project_handle(None);
        }
        result
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

fn load_saved_project_session() -> Option<ProjectHandle> {
    let path = project_session_path()?;
    let payload = std::fs::read_to_string(path).ok()?;
    let session = serde_json::from_str::<AppSession>(&payload).ok()?;
    if session.schema_version != SESSION_SCHEMA_VERSION {
        clear_saved_project_session();
        return None;
    }
    let handle = session
        .windows
        .into_iter()
        .find(|window| window.id == MAIN_WINDOW_ID)
        .and_then(|window| window.project)?;
    if load_project(&handle.path).is_ok() {
        Some(handle)
    } else {
        clear_saved_project_session();
        None
    }
}

fn save_project_session(handle: &ProjectHandle) -> Result<(), String> {
    let path = project_session_path()
        .ok_or_else(|| "no user config directory is available".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let session = AppSession {
        schema_version: SESSION_SCHEMA_VERSION,
        windows: vec![AppSessionWindow {
            id: MAIN_WINDOW_ID.to_string(),
            project: Some(handle.clone()),
            layout_slot: WORKSPACE_LAYOUT_SLOT.to_string(),
        }],
    };
    let payload = serde_json::to_vec_pretty(&session)
        .map_err(|error| format!("failed to serialize app session: {error}"))?;
    std::fs::write(&path, payload)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn clear_saved_project_session() {
    let Some(path) = project_session_path() else {
        return;
    };
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

fn project_session_path() -> Option<std::path::PathBuf> {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        let trimmed = config_home.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed).join("sim-rns/session.json"));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(|home| {
            std::path::PathBuf::from(home)
                .join(".config")
                .join("sim-rns")
                .join("session.json")
        })
}

fn sync_persisted_root_menu(roots: &[MenuRootSpec], items: &[MenuItemSpec]) -> Result<(), String> {
    for path in persisted_layout_paths() {
        if path.is_file() {
            sync_persisted_root_menu_at(&path, roots, items)?;
        }
    }
    Ok(())
}

fn sync_persisted_root_menu_at(
    path: &std::path::Path,
    roots: &[MenuRootSpec],
    items: &[MenuItemSpec],
) -> Result<(), String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut value = serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let spec = value
        .get_mut("spec")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| format!("{} does not contain a shell spec", path.display()))?;
    spec.insert(
        "menu_roots".to_string(),
        serde_json::to_value(roots)
            .map_err(|error| format!("failed to encode root menus: {error}"))?,
    );
    spec.insert(
        "menu_items".to_string(),
        serde_json::to_value(items)
            .map_err(|error| format!("failed to encode menu items: {error}"))?,
    );
    let updated = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("failed to encode {}: {error}", path.display()))?;
    std::fs::write(path, updated)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn persisted_layout_paths() -> Vec<std::path::PathBuf> {
    let Some(root) = user_config_root() else {
        return Vec::new();
    };
    vec![
        root.join("sim-rns--workspace").join("layout.json"),
        root.join("sim-rns").join("layout.json"),
    ]
}

fn user_config_root() -> Option<std::path::PathBuf> {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        let trimmed = config_home.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(|home| std::path::PathBuf::from(home).join(".config"))
}
