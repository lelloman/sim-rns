use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use sim_rns_core::{Project, Recipe, RuntimeStatus, RuntimeVmState};

type RuntimeObserver = Rc<dyn Fn(&RuntimeViewSnapshot)>;

#[derive(Clone, Default)]
pub(crate) struct RuntimeController {
    store: RuntimeStore,
}

impl RuntimeController {
    pub(crate) fn subscribe(&self, observer: RuntimeObserver) -> RuntimeSubscription {
        self.store.subscribe(observer)
    }

    pub(crate) fn latest_or_refresh(
        &self,
        load_snapshot: impl FnOnce() -> RuntimeViewSnapshot,
    ) -> RuntimeViewSnapshot {
        self.store.latest_or_refresh(load_snapshot)
    }

    pub(crate) fn refresh(
        &self,
        load_snapshot: impl FnOnce() -> RuntimeViewSnapshot,
    ) -> RuntimeViewSnapshot {
        self.store.refresh(load_snapshot)
    }

    pub(crate) fn vm_state(
        &self,
        load_snapshot: impl FnOnce() -> RuntimeViewSnapshot,
    ) -> Option<RuntimeVmState> {
        self.latest_or_refresh(load_snapshot).vm_state()
    }

    pub(crate) fn run_command(
        &self,
        command: impl FnOnce() -> Result<(), String>,
        load_snapshot: impl FnOnce() -> RuntimeViewSnapshot,
    ) -> Result<(), String> {
        match command() {
            Ok(()) => {
                self.refresh(load_snapshot);
                Ok(())
            }
            Err(error) => {
                self.publish_error(error.clone(), load_snapshot);
                Err(error)
            }
        }
    }

    pub(crate) fn publish_error(
        &self,
        error: String,
        load_snapshot: impl FnOnce() -> RuntimeViewSnapshot,
    ) {
        let snapshot = self.latest_or_refresh(load_snapshot).with_error(error);
        self.store.publish(snapshot);
    }
}

#[derive(Clone, Default)]
struct RuntimeStore {
    inner: Rc<RuntimeStoreInner>,
}

#[derive(Default)]
struct RuntimeStoreInner {
    next_observer_id: Cell<u64>,
    snapshot: RefCell<Option<RuntimeViewSnapshot>>,
    observers: RefCell<BTreeMap<u64, RuntimeObserver>>,
}

impl RuntimeStore {
    fn subscribe(&self, observer: RuntimeObserver) -> RuntimeSubscription {
        let id = self.inner.next_observer_id.get() + 1;
        self.inner.next_observer_id.set(id);
        self.inner
            .observers
            .borrow_mut()
            .insert(id, observer.clone());
        if let Some(snapshot) = self.inner.snapshot.borrow().as_ref() {
            observer(snapshot);
        }
        RuntimeSubscription {
            id,
            store: Rc::downgrade(&self.inner),
        }
    }

    fn latest_or_refresh(
        &self,
        load_snapshot: impl FnOnce() -> RuntimeViewSnapshot,
    ) -> RuntimeViewSnapshot {
        let current = self.inner.snapshot.borrow().clone();
        if let Some(snapshot) = current {
            snapshot
        } else {
            self.refresh(load_snapshot)
        }
    }

    fn refresh(&self, load_snapshot: impl FnOnce() -> RuntimeViewSnapshot) -> RuntimeViewSnapshot {
        let snapshot = load_snapshot();
        self.publish(snapshot.clone());
        snapshot
    }

    fn publish(&self, snapshot: RuntimeViewSnapshot) {
        let unchanged = self
            .inner
            .snapshot
            .borrow()
            .as_ref()
            .is_some_and(|current| current == &snapshot);
        if unchanged {
            return;
        }
        self.inner.snapshot.replace(Some(snapshot.clone()));
        let observers = self
            .inner
            .observers
            .borrow()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for observer in observers {
            observer(&snapshot);
        }
    }

    fn unsubscribe(&self, id: u64) {
        self.inner.observers.borrow_mut().remove(&id);
    }

    #[cfg(test)]
    fn observer_count(&self) -> usize {
        self.inner.observers.borrow().len()
    }
}

pub(crate) struct RuntimeSubscription {
    id: u64,
    store: Weak<RuntimeStoreInner>,
}

impl Drop for RuntimeSubscription {
    fn drop(&mut self) {
        let Some(store) = self.store.upgrade() else {
            return;
        };
        RuntimeStore { inner: store }.unsubscribe(self.id);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeViewSnapshot {
    pub(crate) project: Option<Project>,
    pub(crate) recipe: Option<Recipe>,
    pub(crate) status: Option<RuntimeStatus>,
    pub(crate) error: Option<String>,
}

impl RuntimeViewSnapshot {
    pub(crate) fn loaded(project: Project, recipe: Recipe, status: RuntimeStatus) -> Self {
        Self {
            project: Some(project),
            recipe: Some(recipe),
            status: Some(status),
            error: None,
        }
    }

    pub(crate) fn project_error(project: Project, error: String) -> Self {
        Self {
            project: Some(project),
            recipe: None,
            status: None,
            error: Some(error),
        }
    }

    pub(crate) fn runtime_error(project: Project, recipe: Recipe, error: String) -> Self {
        Self {
            project: Some(project),
            recipe: Some(recipe),
            status: None,
            error: Some(error),
        }
    }

    pub(crate) fn error(error: String) -> Self {
        Self {
            project: None,
            recipe: None,
            status: None,
            error: Some(error),
        }
    }

    pub(crate) fn with_error(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }

    pub(crate) fn vm_state(&self) -> Option<RuntimeVmState> {
        self.status.as_ref().map(|status| status.vm_state.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn snapshot(message: &str) -> RuntimeViewSnapshot {
        RuntimeViewSnapshot::error(message.to_string())
    }

    #[test]
    fn subscriber_receives_published_snapshots() {
        let store = RuntimeStore::default();
        let received = Rc::new(RefCell::new(Vec::new()));
        let received_for_observer = received.clone();

        let _subscription = store.subscribe(Rc::new(move |snapshot| {
            received_for_observer
                .borrow_mut()
                .push(snapshot.error.clone().unwrap_or_default());
        }));

        store.publish(snapshot("first"));
        store.publish(snapshot("second"));

        assert_eq!(
            received.borrow().as_slice(),
            ["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn subscriber_replays_latest_snapshot_on_subscribe() {
        let store = RuntimeStore::default();
        store.publish(snapshot("current"));
        let received = Rc::new(RefCell::new(Vec::new()));
        let received_for_observer = received.clone();

        let _subscription = store.subscribe(Rc::new(move |snapshot| {
            received_for_observer
                .borrow_mut()
                .push(snapshot.error.clone().unwrap_or_default());
        }));

        assert_eq!(received.borrow().as_slice(), ["current".to_string()]);
    }

    #[test]
    fn publishing_unchanged_snapshot_does_not_notify_again() {
        let store = RuntimeStore::default();
        let received = Rc::new(RefCell::new(Vec::new()));
        let received_for_observer = received.clone();
        let value = snapshot("same");

        let _subscription = store.subscribe(Rc::new(move |snapshot| {
            received_for_observer
                .borrow_mut()
                .push(snapshot.error.clone().unwrap_or_default());
        }));

        store.publish(value.clone());
        store.publish(value);

        assert_eq!(received.borrow().as_slice(), ["same".to_string()]);
    }

    #[test]
    fn dropping_subscription_stops_updates() {
        let store = RuntimeStore::default();
        let received = Rc::new(RefCell::new(Vec::new()));
        let received_for_observer = received.clone();

        let subscription = store.subscribe(Rc::new(move |snapshot| {
            received_for_observer
                .borrow_mut()
                .push(snapshot.error.clone().unwrap_or_default());
        }));
        assert_eq!(store.observer_count(), 1);

        drop(subscription);
        assert_eq!(store.observer_count(), 0);
        store.publish(snapshot("ignored"));

        assert!(received.borrow().is_empty());
    }

    #[test]
    fn latest_or_refresh_loads_once_until_replaced() {
        let store = RuntimeStore::default();
        let loads = Cell::new(0);

        let first = store.latest_or_refresh(|| {
            loads.set(loads.get() + 1);
            snapshot("loaded")
        });
        let second = store.latest_or_refresh(|| {
            loads.set(loads.get() + 1);
            snapshot("not loaded")
        });

        assert_eq!(loads.get(), 1);
        assert_eq!(first, snapshot("loaded"));
        assert_eq!(second, snapshot("loaded"));
    }

    #[test]
    fn controller_refreshes_after_successful_command() {
        let controller = RuntimeController::default();
        let loads = Cell::new(0);

        controller
            .run_command(
                || Ok(()),
                || {
                    loads.set(loads.get() + 1);
                    snapshot("fresh")
                },
            )
            .expect("command should succeed");

        assert_eq!(loads.get(), 1);
        assert_eq!(
            controller
                .latest_or_refresh(|| snapshot("stale"))
                .error
                .as_deref(),
            Some("fresh")
        );
    }

    #[test]
    fn controller_publishes_error_without_discarding_previous_snapshot() {
        let controller = RuntimeController::default();
        let loads = Cell::new(0);
        controller.refresh(|| snapshot("previous"));

        let result = controller.run_command(
            || Err("failed command".to_string()),
            || {
                loads.set(loads.get() + 1);
                snapshot("unused")
            },
        );

        assert_eq!(result, Err("failed command".to_string()));
        assert_eq!(loads.get(), 0);
        assert_eq!(
            controller
                .latest_or_refresh(|| snapshot("unused"))
                .error
                .as_deref(),
            Some("failed command")
        );
    }
}
