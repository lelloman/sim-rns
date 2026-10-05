//! Private local control endpoint. GTK objects never leave the main thread.
use base64::Engine;
use gtk::{glib, prelude::*};
use serde_json::{json, Value};
use sim_rns_control::*;
use sim_rns_core::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::os::unix::{
    fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    net::UnixListener,
};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

type Reply = mpsc::SyncSender<Response>;

pub fn install(app: &gtk::Application) {
    let installed = Rc::new(std::cell::Cell::new(false));
    app.connect_activate(move |app| {
        if installed.replace(true) {
            return;
        }
        if let Err(error) = start(app) {
            eprintln!("sim-rns: control endpoint unavailable: {error}");
        }
    });
}

fn start(app: &gtk::Application) -> Result<(), String> {
    let path = socket_path();
    let parent = path
        .parent()
        .ok_or("control socket requires a parent directory")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|e| e.to_string())?;
    let metadata = std::fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
    // Never put a control socket in a shared or another user's directory.
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("control socket parent must be an owned directory with mode 0700".into());
    }
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(lock_path)
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "another app already owns this control endpoint".to_string())?;
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.file_type().is_socket() {
            return Err("control socket path is occupied by a non-socket file".into());
        }
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (send, receive) = mpsc::sync_channel::<(Request, Reply)>(16);
    let stopped = Arc::new(AtomicBool::new(false));
    let shutdown = stopped.clone();
    app.connect_shutdown(move |_| {
        shutdown.store(true, Ordering::Relaxed);
    });
    std::thread::spawn(move || {
        let _lock = lock;
        let clients = Arc::new(AtomicUsize::new(0));
        while !stopped.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if clients.load(Ordering::Relaxed) >= 16 {
                        continue;
                    }
                    clients.fetch_add(1, Ordering::Relaxed);
                    let clients = clients.clone();
                    let send = send.clone();
                    std::thread::spawn(move || {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        let response = match read_message::<Request>(&stream) {
                            Ok(request) => {
                                let (reply, result) = mpsc::sync_channel(1);
                                if send.try_send((request, reply)).is_err() {
                                    Err("app control queue is unavailable or full".into()).into()
                                } else {
                                    result.recv_timeout(Duration::from_secs(175)).unwrap_or_else(|_| Err("app response timed out; operation may still be running; inspect state before retrying".into()).into())
                                }
                            }
                            Err(error) => Err(error).into(),
                        };
                        let response = if serde_json::to_vec(&response)
                            .map_or(true, |bytes| bytes.len() as u64 >= MAX_MESSAGE)
                        {
                            Err("response exceeds the 4 MiB limit".into()).into()
                        } else {
                            response
                        };
                        let _ = write_message(&mut stream, &response);
                        clients.fetch_sub(1, Ordering::Relaxed);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(_) => break,
            }
        }
        let _ = std::fs::remove_file(path);
    });
    let app = app.downgrade();
    let ui = RefCell::new(Ui::default());
    glib::timeout_add_local(Duration::from_millis(20), move || {
        let Some(app) = app.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if let Ok((request, reply)) = receive.try_recv() {
            if let Request::Runtime(request) = request {
                runtime(request, reply);
            } else if let Request::Project(request) = request {
                project(request, reply);
            } else {
                let result = match request {
                    Request::App(AppRequest::State) => Ok(
                        json!({"project": current_project_handle(), "runtime_busy": sim_rns_plugin::runtime_command_is_busy(), "windows": app.windows().len(), "pid": std::process::id()}),
                    ),
                    Request::App(AppRequest::Quit) => {
                        if sim_rns_plugin::runtime_command_is_busy() {
                            Err("wait for the runtime operation to finish".into())
                        } else {
                            let app = app.clone();
                            glib::timeout_add_local_once(Duration::from_millis(100), move || {
                                app.quit()
                            });
                            Ok(json!({"quitting":true}))
                        }
                    }
                    Request::Project(_) => unreachable!(),
                    Request::Ui(request) => ui.borrow_mut().dispatch(&app, request),
                    Request::Runtime(_) => unreachable!(),
                };
                let _ = reply.send(result.into());
            }
        }
        glib::ControlFlow::Continue
    });
    Ok(())
}

fn project(request: ProjectRequest, reply: Reply) {
    if let Err(error) = sim_rns_plugin::begin_external_operation() {
        let _ = reply.send(Err(error).into());
        return;
    }
    let active = current_project_handle();
    std::thread::spawn(move || {
        let result = project_work(request, active);
        glib::idle_add_once(move || {
            sim_rns_plugin::finish_external_operation();
            let result = result.and_then(|(value, transition)| {
                match transition {
                    Transition::Open(handle) => open_project(handle)?,
                    Transition::Close => close_project()?,
                    Transition::None => (),
                }
                sim_rns_plugin::refresh_runtime_store();
                Ok(value)
            });
            let _ = reply.send(result.into());
        });
    });
}

enum Transition {
    None,
    Open(ProjectHandle),
    Close,
}

fn project_work(
    request: ProjectRequest,
    active: Option<ProjectHandle>,
) -> Result<(Value, Transition), String> {
    match request {
        ProjectRequest::CreateDemo { path, bundle } => {
            let project = create_demo_project(path, bundle)?;
            let handle = project.handle();
            Ok((
                json!({"path":project.root_path,"project":project.file}),
                Transition::Open(handle),
            ))
        }
        ProjectRequest::Create { path, name } => {
            let project = create_project(path, &name)?;
            let handle = ProjectHandle::for_local_dir(&project.root_path)?;
            Ok((
                json!({"path": project.root_path, "project":project.file}),
                Transition::Open(handle),
            ))
        }
        ProjectRequest::Open { path } => {
            let project = load_project(path)?;
            let handle = ProjectHandle::for_local_dir(&project.root_path)?;
            Ok((
                json!({"path": project.root_path, "project":project.file}),
                Transition::Open(handle),
            ))
        }
        ProjectRequest::Close => Ok((json!({"closed":true}), Transition::Close)),
        request => {
            let project = load_project(active.ok_or("no active project")?.path)?;
            match request {
                ProjectRequest::Inspect => Ok((
                    json!({"path":project.root_path, "project":project.file, "recipe":project_recipe(&project)?}),
                    Transition::None,
                )),
                ProjectRequest::ReadFile { path } => Ok((
                    json!({"path":path,"contents":read_project_source(&project.root_path, &path)?}),
                    Transition::None,
                )),
                ProjectRequest::WriteFile { path, contents } => {
                    let project = write_project_source(&project.root_path, &path, &contents)?;
                    Ok((
                        json!({"saved":path}),
                        Transition::Open(ProjectHandle::for_local_dir(&project.root_path)?),
                    ))
                }
                ProjectRequest::AddNode | ProjectRequest::AddScript => {
                    let (_, path) = if matches!(request, ProjectRequest::AddNode) {
                        add_node_include(&project.root_path)?
                    } else {
                        add_script_include(&project.root_path)?
                    };
                    Ok((
                        json!({"path":path}),
                        Transition::Open(ProjectHandle::for_local_dir(&project.root_path)?),
                    ))
                }
                _ => unreachable!(),
            }
        }
    }
}

fn runtime(request: RuntimeRequest, reply: Reply) {
    let result = current_project_handle()
        .ok_or_else(|| "no active project".to_string())
        .and_then(|handle| {
            sim_rns_plugin::begin_external_operation()?;
            Ok(handle)
        });
    let handle = match result {
        Ok(handle) => handle,
        Err(error) => {
            let _ = reply.send(Err(error).into());
            return;
        }
    };
    std::thread::spawn(move || {
        let result =
            load_project(&handle.path).and_then(|project| execute_runtime(&project, request));
        glib::idle_add_once(move || {
            sim_rns_plugin::finish_external_operation();
            let _ = reply.send(result.into());
        });
    });
}

fn execute_runtime(project: &Project, request: RuntimeRequest) -> Result<Value, String> {
    let runtime = QemuRuntime::default();
    let command = match request {
        RuntimeRequest::Status => {
            return serde_json::to_value(runtime.status(project).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())
        }
        RuntimeRequest::Prepare {
            source_image,
            size_gb,
        } => RuntimeCommand::PrepareVm {
            source_image,
            size_gb,
        },
        RuntimeRequest::Boot => {
            if !runtime
                .status(project)
                .map_err(|e| e.to_string())?
                .vm_assets
                .prepared
            {
                runtime
                    .execute(
                        project,
                        RuntimeCommand::PrepareVm {
                            source_image: None,
                            size_gb: 0,
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
            RuntimeCommand::Boot
        }
        RuntimeRequest::Shutdown => RuntimeCommand::Shutdown,
        RuntimeRequest::Pause => RuntimeCommand::Pause,
        RuntimeRequest::Resume => RuntimeCommand::Resume,
        RuntimeRequest::StartNode { element_id } => RuntimeCommand::StartNode { element_id },
        RuntimeRequest::StopNode { element_id } => RuntimeCommand::StopNode { element_id },
        RuntimeRequest::RestartNode { element_id } => RuntimeCommand::RestartNode { element_id },
        RuntimeRequest::CreateSnapshot { name, note } => {
            RuntimeCommand::CreateSnapshot { name, note }
        }
        RuntimeRequest::RestoreSnapshot { snapshot_id } => {
            RuntimeCommand::RestoreSnapshot { snapshot_id }
        }
        RuntimeRequest::DeleteSnapshot { snapshot_id } => {
            RuntimeCommand::DeleteSnapshot { snapshot_id }
        }
        RuntimeRequest::AddTopologyLink {
            element_id,
            network_id,
        } => RuntimeCommand::AddTopologyLink {
            element_id,
            network_id,
        },
        RuntimeRequest::RemoveTopologyLink {
            element_id,
            network_id,
        } => RuntimeCommand::RemoveTopologyLink {
            element_id,
            network_id,
        },
    };
    serde_json::to_value(
        runtime
            .execute(project, command)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[derive(Default)]
struct Ui {
    next: u64,
    widgets: BTreeMap<u64, glib::WeakRef<gtk::Widget>>,
}

impl Ui {
    fn id(&mut self, widget: &gtk::Widget) -> u64 {
        if let Some((id, _)) = self
            .widgets
            .iter()
            .find(|(_, weak)| weak.upgrade().as_ref() == Some(widget))
        {
            return *id;
        }
        self.next += 1;
        self.widgets.insert(self.next, widget.downgrade());
        self.next
    }
    fn widget(&self, id: u64) -> Result<gtk::Widget, String> {
        self.widgets
            .get(&id)
            .and_then(|weak| weak.upgrade())
            .ok_or_else(|| "stale or unknown widget ID; refresh ui tree".into())
    }
    fn window(&self, id: u64) -> Result<gtk::Window, String> {
        self.widget(id)?
            .downcast()
            .map_err(|_| "ID is not a window".into())
    }
    fn tree(&mut self, widget: &gtk::Widget, depth: usize) -> Value {
        let id = self.id(widget);
        let mut value = json!({"id":id,"type":widget.type_().name(),"name":widget.widget_name().as_str(),"visible":widget.is_visible(),"mapped":widget.is_mapped(),"sensitive":widget.is_sensitive(),"focused":widget.has_focus(),"css_classes":widget.css_classes().iter().map(|s| s.as_str()).collect::<Vec<_>>()});
        let allocation = widget.allocation();
        value["bounds"] = json!({"x":allocation.x(),"y":allocation.y(),"width":allocation.width(),"height":allocation.height()});
        if let Some(bounds) = widget
            .parent()
            .and_then(|parent| widget.compute_bounds(&parent))
        {
            value["bounds"] = json!({"x":bounds.x(),"y":bounds.y(),"width":bounds.width(),"height":bounds.height()});
        }
        value["tooltip"] = json!(widget.tooltip_text().as_deref());
        value["controllers"] = json!((0..widget.observe_controllers().n_items())
            .filter_map(|i| widget
                .observe_controllers()
                .item(i)
                .map(|c| c.type_().name().to_string()))
            .collect::<Vec<_>>());
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            value["text"] = json!(label.text().as_str());
        }
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            value["label"] = json!(button.label().as_deref());
        }
        if let Some(editable) = widget.dynamic_cast_ref::<gtk::Editable>() {
            value["text"] = json!(editable.text().as_str());
        }
        if let Some(text) = widget.downcast_ref::<gtk::TextView>() {
            let b = text.buffer();
            value["text"] = json!(b
                .text(&b.start_iter(), &b.end_iter(), true)
                .chars()
                .take(16000)
                .collect::<String>());
        }
        if let Some(check) = widget.downcast_ref::<gtk::CheckButton>() {
            value["active"] = json!(check.is_active());
        }
        if let Some(toggle) = widget.downcast_ref::<gtk::ToggleButton>() {
            value["active"] = json!(toggle.is_active());
        }
        if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
            value["active"] = json!(switch.is_active());
        }
        if let Some(stack) = widget.downcast_ref::<gtk::Stack>() {
            value["visible_child"] = json!(stack.visible_child_name().as_deref());
        }
        if let Some(dropdown) = widget.downcast_ref::<gtk::DropDown>() {
            value["selected"] = json!(dropdown.selected());
            if let Some(model) = dropdown.model() {
                value["items"] = json!((0..model.n_items())
                    .map(|index| model
                        .item(index)
                        .and_then(|item| item.downcast::<gtk::StringObject>().ok())
                        .map(|item| item.string().to_string()))
                    .collect::<Vec<_>>());
            }
        }
        if let Some(spin) = widget.downcast_ref::<gtk::SpinButton>() {
            value["value"] = json!(spin.value());
        }
        if let Some(range) = widget.downcast_ref::<gtk::Range>() {
            value["value"] = json!(range.value());
        }
        if let Some(window) = widget.downcast_ref::<gtk::Window>() {
            value["title"] = json!(window.title().as_deref());
        }
        if let Some(window) = widget.downcast_ref::<gtk::ApplicationWindow>() {
            value["commands"] = json!(window
                .list_actions()
                .iter()
                .map(|name| {
                    let menu = super::root_menu_items().into_iter().find(|item|
                        maruzzella::spec::command_name(&item.command_id) == name.as_str() || maruzzella::spec::command_name(&item.id) == name.as_str());
                    let toolbar = super::runtime_toolbar_items().into_iter().find(|item|
                        maruzzella::spec::command_name(&item.command_id) == name.as_str() || maruzzella::spec::command_name(&item.id) == name.as_str());
                    json!({"name":name.as_str(),"enabled":window.is_action_enabled(name),"command_id":menu.as_ref().map(|item| item.command_id.as_str()).or_else(|| toolbar.as_ref().map(|item| item.command_id.as_str()))})
                })
                .collect::<Vec<_>>());
        }
        if depth < 80 {
            let mut children = Vec::new();
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                children.push(self.tree(&current, depth + 1));
            }
            value["children"] = json!(children);
        }
        value
    }
    fn dispatch(&mut self, _app: &gtk::Application, request: UiRequest) -> Result<Value, String> {
        use UiRequest::*;
        match request {
            Tree => {
                self.widgets.retain(|_, weak| weak.upgrade().is_some());
                return Ok(
                    json!({"windows":gtk::Window::list_toplevels().iter().map(|window| self.tree(window, 0)).collect::<Vec<_>>()}),
                );
            }
            Screenshot { window_id } => {
                let window = self.window(window_id)?;
                if !window.is_mapped() {
                    return Err("window is not displayed".into());
                }
                let paintable = gtk::WidgetPaintable::new(Some(&window));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
                let node = snapshot
                    .to_node()
                    .ok_or("window has no rendered content yet")?;
                let renderer = window.renderer().ok_or("window has no renderer")?;
                let texture = renderer.render_texture(&node, None);
                return Ok(
                    json!({"mime_type":"image/png", "image":base64::engine::general_purpose::STANDARD.encode(texture.save_to_png_bytes())}),
                );
            }
            Command { window_id, name } => {
                let window = self
                    .window(window_id)?
                    .downcast::<gtk::ApplicationWindow>()
                    .map_err(|_| "window has no command registry")?;
                let action = window
                    .lookup_action(&name)
                    .or_else(|| window.lookup_action(&maruzzella::spec::command_name(&name)))
                    .ok_or("unknown command; inspect ui tree")?;
                if !action.is_enabled() {
                    return Err("command is disabled".into());
                }
                if action.parameter_type().is_some() {
                    return Err("command requires a parameter".into());
                }
                action.activate(None);
            }
            Present { window_id } => self.window(window_id)?.present(),
            Resize {
                window_id,
                width,
                height,
            } => {
                if !(100..=16384).contains(&width) || !(100..=16384).contains(&height) {
                    return Err("window dimensions must be between 100 and 16384".into());
                }
                self.window(window_id)?.set_default_size(width, height);
            }
            Maximize {
                window_id,
                maximized,
            } => {
                let window = self.window(window_id)?;
                if maximized {
                    window.maximize()
                } else {
                    window.unmaximize()
                }
            }
            CloseWindow { window_id } => self.window(window_id)?.close(),
            request => {
                let id = match &request {
                    Activate { widget_id }
                    | ContextMenu { widget_id }
                    | DismissPopup { widget_id }
                    | Drag { widget_id, .. }
                    | Focus { widget_id }
                    | SetText { widget_id, .. }
                    | SetActive { widget_id, .. }
                    | SetValue { widget_id, .. }
                    | Select { widget_id, .. }
                    | Scroll { widget_id, .. } => *widget_id,
                    _ => unreachable!(),
                };
                let widget = self.widget(id)?;
                if !widget.is_sensitive() || !widget.is_mapped() {
                    return Err("widget is disabled or not displayed".into());
                }
                match request {
                    ContextMenu { .. } => {
                        let controllers = widget.observe_controllers();
                        let mut activated = false;
                        for index in 0..controllers.n_items() {
                            if let Some(gesture) = controllers
                                .item(index)
                                .and_then(|item| item.downcast::<gtk::GestureClick>().ok())
                            {
                                if gesture.button() == 3 {
                                    gesture.emit_by_name::<()>("pressed", &[&1i32, &0f64, &0f64]);
                                    activated = true;
                                }
                            }
                        }
                        if !activated {
                            return Err("widget has no secondary-click controller".into());
                        }
                    }
                    DismissPopup { .. } => widget
                        .downcast_ref::<gtk::Popover>()
                        .ok_or("widget is not a popover")?
                        .popdown(),
                    Drag {
                        start_x,
                        start_y,
                        offset_x,
                        offset_y,
                        ..
                    } => {
                        if ![start_x, start_y, offset_x, offset_y]
                            .iter()
                            .all(|v| v.is_finite())
                        {
                            return Err("drag coordinates must be finite".into());
                        }
                        let controllers = widget.observe_controllers();
                        let gesture = (0..controllers.n_items())
                            .find_map(|index| {
                                controllers
                                    .item(index)
                                    .and_then(|item| item.downcast::<gtk::GestureDrag>().ok())
                            })
                            .ok_or("widget has no drag controller")?;
                        gesture.emit_by_name::<()>("drag-begin", &[&start_x, &start_y]);
                        // The shell uses ancestor motion trackers to resolve cross-group drops.
                        // Keep those trackers in sync with the synthetic drag position.
                        let mut ancestor = Some(widget.clone());
                        let mut trackers = Vec::new();
                        let point = gtk::graphene::Point::new(
                            (start_x + offset_x) as f32,
                            (start_y + offset_y) as f32,
                        );
                        while let Some(target) = ancestor {
                            if let Some(point) = widget
                                .compute_point(&target, &point)
                                .filter(|_| target.has_css_class("custom-workbench-group"))
                            {
                                let controllers = target.observe_controllers();
                                for index in 0..controllers.n_items() {
                                    if let Some(motion) = controllers.item(index).and_then(|item| {
                                        item.downcast::<gtk::EventControllerMotion>().ok()
                                    }) {
                                        motion.emit_by_name::<()>(
                                            "motion",
                                            &[&(point.x() as f64), &(point.y() as f64)],
                                        );
                                        trackers.push(motion);
                                    }
                                }
                            }
                            ancestor = target.parent();
                        }
                        gesture.emit_by_name::<()>("drag-update", &[&offset_x, &offset_y]);
                        gesture.emit_by_name::<()>("drag-end", &[&offset_x, &offset_y]);
                        for tracker in trackers {
                            tracker.emit_by_name::<()>("leave", &[]);
                        }
                    }
                    Activate { .. } => {
                        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                            button.emit_clicked();
                        } else if let Some(button) = widget.downcast_ref::<gtk::MenuButton>() {
                            button.popup();
                        } else if widget.has_css_class("tab-header")
                            || widget.has_css_class("custom-tab-header")
                        {
                            let controllers = widget.observe_controllers();
                            let mut activated = false;
                            for index in 0..controllers.n_items() {
                                if let Some(gesture) = controllers
                                    .item(index)
                                    .and_then(|item| item.downcast::<gtk::GestureClick>().ok())
                                {
                                    if gesture.button() != 0 && gesture.button() != 1 {
                                        continue;
                                    }
                                    gesture.emit_by_name::<()>("pressed", &[&1i32, &0f64, &0f64]);
                                    activated = true;
                                }
                            }
                            if !activated {
                                return Err("tab has no click controller".into());
                            }
                        } else if !widget.activate() {
                            return Err(
                                "widget does not support activation; select a button or tab header"
                                    .into(),
                            );
                        }
                    }
                    Focus { .. } => {
                        if !widget.grab_focus() {
                            return Err("widget cannot receive focus".into());
                        }
                    }
                    SetText { text, .. } => {
                        if let Some(editable) = widget.dynamic_cast_ref::<gtk::Editable>() {
                            if !editable.is_editable() {
                                return Err("text is read-only".into());
                            }
                            editable.set_text(&text);
                        } else if let Some(view) = widget.downcast_ref::<gtk::TextView>() {
                            if !view.is_editable() {
                                return Err("text is read-only".into());
                            }
                            view.buffer().set_text(&text);
                        } else {
                            return Err("widget is not a text editor".into());
                        }
                    }
                    SetActive { active, .. } => {
                        if let Some(check) = widget.downcast_ref::<gtk::CheckButton>() {
                            check.set_active(active);
                        } else if let Some(toggle) = widget.downcast_ref::<gtk::ToggleButton>() {
                            toggle.set_active(active);
                        } else if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
                            switch.set_active(active);
                        } else {
                            return Err("widget is not a toggle".into());
                        }
                    }
                    SetValue { value, .. } => {
                        if !value.is_finite() {
                            return Err("value must be finite".into());
                        }
                        if let Some(spin) = widget.downcast_ref::<gtk::SpinButton>() {
                            spin.set_value(value);
                        } else if let Some(range) = widget.dynamic_cast_ref::<gtk::Range>() {
                            range.set_value(value);
                        } else {
                            return Err("widget is not a numeric control".into());
                        }
                    }
                    Select { index, .. } => {
                        if let Some(dropdown) = widget.downcast_ref::<gtk::DropDown>() {
                            if index >= dropdown.model().map_or(0, |model| model.n_items()) {
                                return Err("selection index out of range".into());
                            }
                            dropdown.set_selected(index);
                        } else if let Some(list) = widget.downcast_ref::<gtk::ListBox>() {
                            let row = i32::try_from(index)
                                .ok()
                                .and_then(|index| list.row_at_index(index))
                                .ok_or("selection index out of range")?;
                            list.select_row(Some(&row));
                        } else {
                            return Err("widget is not a supported selection control".into());
                        }
                    }
                    Scroll {
                        horizontal,
                        vertical,
                        ..
                    } => {
                        let scroll = widget
                            .downcast_ref::<gtk::ScrolledWindow>()
                            .ok_or("widget is not a scroll container")?;
                        if !horizontal.is_finite() || !vertical.is_finite() {
                            return Err("scroll values must be finite".into());
                        }
                        scroll.hadjustment().set_value(horizontal);
                        scroll.vadjustment().set_value(vertical);
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(json!({"applied":true}))
    }
}
