use super::*;
use sim_rns_core::{
    editor::{self, Edit, EditorSnapshot},
    ProjectNodeFile, ResourceLimits, RestartPolicy,
};
use std::collections::BTreeMap;

pub(super) extern "C" fn create_view(
    _host: *const maruzzella_sdk::ffi::MzHostApi,
    _request: *const maruzzella_sdk::ffi::MzViewRequest,
) -> *mut std::ffi::c_void {
    if !gtk::is_initialized_main_thread() && gtk::init().is_err() {
        return std::ptr::null_mut();
    }
    let root = build_root(
        "Nodes",
        "Configure nodes, LAN connections, and scripts while the simulation is stopped.",
    );
    let list = GtkBox::new(Orientation::Vertical, 10);
    root.append(&list);
    populate(&list);
    let weak = list.downgrade();
    let subscription = RUNTIME_CONTROLLER.with(|controller| {
        controller.subscribe(Rc::new(move |_| {
            if let Some(list) = weak.upgrade() {
                populate(&list);
            }
        }))
    });
    unsafe {
        root.set_data("sim-rns-editor-subscription", subscription);
    }
    unsafe {
        <gtk::Widget as IntoGlibPtr<*mut gtk::ffi::GtkWidget>>::into_glib_ptr(root.upcast())
            as *mut std::ffi::c_void
    }
}

fn populate(list: &GtkBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let snapshot = match current_project().and_then(|p| editor::load(&p.root_path)) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            list.append(&Label::new(Some(&error)));
            return;
        }
    };
    let editable =
        !runtime_command_is_busy() && current_runtime_vm_state() == Some(RuntimeVmState::Stopped);
    let add = Button::with_label("Add Node / LAN");
    add.set_widget_name("nodes-add");
    add.set_sensitive(editable);
    add.connect_clicked(|button| open_node(button, None));
    list.append(&add);
    if !editable {
        list.append(&Label::new(Some(
            "Stop the simulation and wait for the current operation to finish to edit.",
        )));
    }
    for (_, node) in snapshot.nodes {
        let row = GtkBox::new(Orientation::Horizontal, 12);
        let label = Label::new(Some(&format!(
            "{} · {}{}\nLANs: {}",
            node.id,
            node.template_id,
            if node.enabled { "" } else { " (disabled)" },
            node.attachments.join(", ")
        )));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_wrap(true);
        row.append(&label);
        let edit = Button::with_label("Edit");
        edit.set_widget_name(&format!("node-edit-{}", node.id));
        edit.set_sensitive(editable);
        let id = node.id;
        edit.connect_clicked(move |button| open_node(button, Some(&id)));
        row.append(&edit);
        list.append(&row);
    }
    let heading = Label::new(Some("Project scripts"));
    heading.set_xalign(0.0);
    list.append(&heading);
    for (path, _) in snapshot.scripts {
        let button = Button::with_label(&format!("Edit {path}"));
        button.set_sensitive(editable);
        button.connect_clicked(move |button| open_script(button, &path));
        list.append(&button);
    }
}

fn dialog(button: &Button, title: &str) -> (gtk::Window, GtkBox) {
    let parent = button.root().and_downcast::<gtk::Window>();
    let window = gtk::Window::builder()
        .title(title)
        .default_width(680)
        .default_height(760)
        .modal(true)
        .build();
    window.set_transient_for(parent.as_ref());
    if let Some(app) = parent.and_then(|p| p.application()) {
        window.set_application(Some(&app));
    }
    let form = build_root(title, "Changes are validated before saving.");
    let scroller = create_scroller();
    scroller.set_child(Some(&form));
    window.set_child(Some(&scroller));
    (window, form)
}
fn field(form: &GtkBox, label: &str, widget: &impl IsA<gtk::Widget>) {
    let heading = Label::new(Some(label));
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    form.append(&heading);
    form.append(widget);
}
fn text_area(form: &GtkBox, label: &str, name: &str, contents: &str) -> gtk::TextView {
    let text = gtk::TextView::builder()
        .monospace(true)
        .height_request(100)
        .build();
    text.set_widget_name(name);
    text.buffer().set_text(contents);
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    field(form, label, &text);
    text
}
fn contents(text: &gtk::TextView) -> String {
    let b = text.buffer();
    b.text(&b.start_iter(), &b.end_iter(), true).to_string()
}
fn open_node(button: &Button, id: Option<&str>) {
    let snapshot = match current_project().and_then(|p| editor::load(&p.root_path)) {
        Ok(s) => s,
        Err(e) => {
            publish_runtime_error(e);
            return;
        }
    };
    let original = id.and_then(|id| {
        snapshot
            .nodes
            .iter()
            .find(|(_, n)| n.id == id)
            .map(|(_, n)| n.clone())
    });
    let (window, form) = dialog(
        button,
        if original.is_some() {
            "Edit Node"
        } else {
            "Add Node / LAN"
        },
    );
    let id_field = gtk::Entry::new();
    id_field.set_widget_name("node-id");
    if let Some(n) = &original {
        id_field.set_text(&n.id);
        id_field.set_editable(false);
    }
    field(
        &form,
        "Node ID (letters, digits, dots, underscores, hyphens)",
        &id_field,
    );
    let mut templates = editor::SUPPORTED_TEMPLATES
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    if let Some(n) = &original {
        if !templates.contains(&n.template_id) {
            templates.push(n.template_id.clone());
        }
    }
    let refs = templates.iter().map(String::as_str).collect::<Vec<_>>();
    let template = gtk::DropDown::from_strings(&refs);
    template.set_widget_name("node-template");
    template.set_selected(
        original
            .as_ref()
            .and_then(|n| templates.iter().position(|t| t == &n.template_id))
            .unwrap_or(0) as u32,
    );
    field(&form, "Template", &template);
    let enabled = gtk::CheckButton::with_label("Enabled");
    enabled.set_widget_name("node-enabled");
    enabled.set_active(original.as_ref().is_none_or(|n| n.enabled));
    form.append(&enabled);
    let networks = snapshot
        .nodes
        .iter()
        .filter(|(_, n)| n.template_id == "network.lan" && Some(n.id.as_str()) != id)
        .map(|(_, n)| {
            let check = gtk::CheckButton::with_label(&format!(
                "Connect to {}{}",
                n.id,
                if n.enabled { "" } else { " (disabled)" }
            ));
            check.set_widget_name(&format!("node-lan-{}", n.id));
            check.set_active(
                original
                    .as_ref()
                    .is_some_and(|o| o.attachments.contains(&n.id)),
            );
            form.append(&check);
            (n.id.clone(), check)
        })
        .collect::<Vec<_>>();
    let env_text = original
        .as_ref()
        .map(|n| {
            n.env
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let env = text_area(
        &form,
        "Environment (one NAME=value per line)",
        "node-env",
        &env_text,
    );
    let defaults = original
        .as_ref()
        .and_then(|n| {
            sim_rns_core::base_templates()
                .into_iter()
                .find(|t| t.id == n.template_id)
        })
        .map(|t| t.defaults.resources)
        .unwrap_or(ResourceLimits {
            memory_mb: 256,
            cpu_weight: 100,
        });
    let limits = original
        .as_ref()
        .and_then(|n| n.resources.clone())
        .unwrap_or(defaults);
    let memory = gtk::SpinButton::with_range(0.0, u32::MAX as f64, 16.0);
    memory.set_value(limits.memory_mb as f64);
    memory.set_widget_name("node-memory");
    field(&form, "Memory limit (MiB; 0 means unlimited)", &memory);
    let cpu = gtk::SpinButton::with_range(1.0, 10000.0, 10.0);
    cpu.set_value(limits.cpu_weight.max(1) as f64);
    cpu.set_widget_name("node-cpu");
    field(&form, "CPU weight (1–10000)", &cpu);
    let restart = gtk::DropDown::from_strings(&["Never", "On failure", "Always"]);
    restart.set_widget_name("node-restart");
    restart.set_selected(
        match original.as_ref().and_then(|n| n.restart_policy.as_ref()) {
            Some(RestartPolicy::Never) => 0,
            Some(RestartPolicy::Always) => 2,
            _ => 1,
        },
    );
    field(&form, "Restart policy", &restart);
    let original_argv = original
        .as_ref()
        .and_then(|n| n.command_override.clone())
        .unwrap_or_default();
    let inline = original
        .as_ref()
        .is_some_and(|n| n.template_id.starts_with("script."))
        && original_argv.len() == 3
        && original_argv[1] == "-c"
        && ["python3", "bash"].contains(&original_argv[0].as_str());
    let command_text = if inline {
        String::new()
    } else {
        original_argv.join("\n")
    };
    let command = text_area(
        &form,
        "Command override (one argument per line; blank uses template default)",
        "node-command",
        &command_text,
    );
    let script = text_area(
        &form,
        "Python / Bash script (for script templates; when nonempty, replaces command override)",
        "node-script",
        if inline { &original_argv[2] } else { "" },
    );
    let fresh = fresh_choice(&form, &snapshot);
    let error = workspace_error_label();
    form.append(&error);
    let save = Button::with_label("Save Node");
    save.set_widget_name("node-save");
    save.add_css_class("suggested-action");
    form.append(&save);
    let weak = window.downgrade();
    let form_copy = form.clone();
    let error_copy = error.clone();
    let original_copy = original.clone();
    let snapshot_copy = snapshot.clone();
    save.connect_clicked(move |_| {
        let result = (|| {
            let mut vars = BTreeMap::new();
            for line in contents(&env)
                .lines()
                .filter(|l| !l.trim().is_empty() && contents(&env) != env_text)
            {
                let (key, value) = line
                    .split_once('=')
                    .ok_or("Environment entries must use NAME=value")?;
                if vars.insert(key.to_string(), value.to_string()).is_some() {
                    return Err("Duplicate environment variable".to_string());
                }
            }
            if contents(&env) == env_text {
                vars = original_copy
                    .as_ref()
                    .map(|n| n.env.clone())
                    .unwrap_or_default();
            }
            let template_id = templates[template.selected() as usize].clone();
            let argv = contents(&command);
            let mut command_override = if argv.trim().is_empty() {
                None
            } else {
                Some(argv.lines().map(str::to_string).collect())
            };
            if !inline && argv == command_text {
                command_override = original_copy
                    .as_ref()
                    .and_then(|n| n.command_override.clone());
            }
            let code = contents(&script);
            if !code.trim().is_empty() {
                let interpreter = match template_id.as_str() {
                    "script.python" => "python3",
                    "script.bash" => "bash",
                    _ => return Err("Script text requires a Python or Bash script template".into()),
                };
                command_override = Some(vec![interpreter.into(), "-c".into(), code]);
            }
            let node = ProjectNodeFile {
                id: id_field.text().trim().into(),
                template_id,
                enabled: enabled.is_active(),
                env: vars,
                assets: original_copy
                    .as_ref()
                    .map(|n| n.assets.clone())
                    .unwrap_or_default(),
                resources: Some(ResourceLimits {
                    memory_mb: memory.value() as u32,
                    cpu_weight: cpu.value() as u32,
                }),
                restart_policy: Some(match restart.selected() {
                    0 => RestartPolicy::Never,
                    2 => RestartPolicy::Always,
                    _ => RestartPolicy::OnFailure,
                }),
                command_override,
                attachments: networks
                    .iter()
                    .filter(|(_, c)| c.is_active())
                    .map(|(id, _)| id.clone())
                    .collect(),
            };
            Ok(Edit::SaveNode {
                original_id: original_copy.as_ref().map(|n| n.id.clone()),
                node,
            })
        })();
        match result {
            Ok(edit) => {
                if let Some(window) = weak.upgrade() {
                    submit(
                        &window,
                        &form_copy,
                        &error_copy,
                        snapshot_copy.clone(),
                        edit,
                        fresh.is_active(),
                    );
                }
            }
            Err(e) => set_error(&error_copy, &e),
        }
    });
    if let Some(node) = original {
        let remove = Button::with_label("Remove Node");
        remove.set_widget_name("node-remove");
        remove.add_css_class("destructive-action");
        form.append(&remove);
        let weak = window.downgrade();
        let form = form.clone();
        let error = error.clone();
        remove.connect_clicked(move |_| {
            let Some(window) = weak.upgrade() else { return; };
            let confirm = gtk::AlertDialog::builder().modal(true).message(format!("Remove {}?", node.id))
                .detail("This removes the node from the simulation and disconnects its LAN links. Source files are retained. A prepared VM requires the fresh-guest choice in the editor.")
                .buttons(["Cancel", "Remove"]).cancel_button(0).default_button(0).build();
            let window_copy = window.clone(); let form = form.clone(); let error = error.clone(); let snapshot = snapshot.clone(); let id = node.id.clone();
            // Read the same explicit choice used by Save.
            let fresh = form.observe_children();
            let fresh_guest = (0..fresh.n_items()).filter_map(|i| fresh.item(i)).filter_map(|o| o.downcast::<gtk::CheckButton>().ok()).any(|c| c.widget_name() == "node-fresh-guest" && c.is_active());
            confirm.choose(Some(&window), gio::Cancellable::NONE, move |answer| {
                if answer == Ok(1) { submit(&window_copy, &form, &error, snapshot, Edit::RemoveNode { id }, fresh_guest); }
            });
        });
    }
    cancel_button(&form, &window);
    window.present();
}
fn fresh_choice(form: &GtkBox, snapshot: &EditorSnapshot) -> gtk::CheckButton {
    let check = gtk::CheckButton::with_label(
        "Start a fresh guest on next Run (new identities and guest files)",
    );
    check.set_widget_name("node-fresh-guest");
    if QemuRuntime::default()
        .layout(&snapshot.project)
        .disk_image_path
        .exists()
    {
        form.append(&Label::new(Some(
            "The previous VM will be preserved in .sim-rns/previous-vm-*; backups use disk space.",
        )));
        form.append(&check);
    }
    check
}
fn cancel_button(form: &GtkBox, window: &gtk::Window) {
    let cancel = Button::with_label("Cancel");
    cancel.set_widget_name("node-cancel");
    let weak = window.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(w) = weak.upgrade() {
            w.close();
        }
    });
    form.append(&cancel);
}
fn open_script(button: &Button, path: &str) {
    let snapshot = match current_project().and_then(|p| editor::load(&p.root_path)) {
        Ok(s) => s,
        Err(e) => {
            publish_runtime_error(e);
            return;
        }
    };
    let Some((_, source)) = snapshot.scripts.iter().find(|(p, _)| p == path) else {
        return;
    };
    let (window, form) = dialog(button, &format!("Edit {path}"));
    let text = text_area(&form, "Script source", "project-script", source);
    text.set_height_request(400);
    let fresh = fresh_choice(&form, &snapshot);
    let error = workspace_error_label();
    form.append(&error);
    let save = Button::with_label("Save Script");
    save.set_widget_name("script-save");
    form.append(&save);
    let weak = window.downgrade();
    let form_copy = form.clone();
    let path = path.to_string();
    save.connect_clicked(move |_| {
        if let Some(w) = weak.upgrade() {
            submit(
                &w,
                &form_copy,
                &error,
                snapshot.clone(),
                Edit::SaveScript {
                    path: path.clone(),
                    contents: contents(&text),
                },
                fresh.is_active(),
            );
        }
    });
    cancel_button(&form, &window);
    window.present();
}
fn submit(
    window: &gtk::Window,
    form: &GtkBox,
    error: &Label,
    snapshot: EditorSnapshot,
    edit: Edit,
    fresh: bool,
) {
    if current_project_handle().as_ref().map(|h| &h.path) != Some(&snapshot.project.handle().path) {
        set_error(error, "Active project changed; reopen the editor.");
        return;
    }
    if let Err(e) = begin_external_operation() {
        set_error(error, &e);
        return;
    }
    set_error(error, "Saving…");
    form.set_sensitive(false);
    let guard = window.connect_close_request(|_| gtk::glib::Propagation::Stop);
    let mut guard = Some(guard);
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = send.send(editor::apply(&snapshot, edit, fresh));
    });
    let weak = window.downgrade();
    let form = form.clone();
    let error = error.clone();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
        let result = match receive.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return gtk::glib::ControlFlow::Continue,
            Err(_) => Err("Save worker exited unexpectedly".into()),
        };
        finish_external_operation();
        form.set_sensitive(true);
        if let Some(w) = weak.upgrade() {
            if let Some(guard) = guard.take() {
                w.disconnect(guard);
            }
            match result {
                Ok(_) => w.close(),
                Err(e) => set_error(&error, &e),
            }
        }
        gtk::glib::ControlFlow::Break
    });
}
