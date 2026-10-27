//! Callback dispatchers used by native decoder implementations.

use std::{
    any::TypeId,
    collections::HashMap,
    error::Error,
    sync::{Arc, Mutex, RwLock},
};

use crate::{
    hal::errors::SharedError,
    types::{EventCD, EventExtTrigger},
};

type ErrorCallback = Box<dyn FnMut(SharedError) + Send + 'static>;
type CDCallback = Box<dyn FnMut(&[EventCD]) + Send + 'static>;
type ExtCallback = Box<dyn FnMut(&[EventExtTrigger]) + Send + 'static>;

/// Routes typed decoder errors to registered callbacks.
///
/// Subscribers are keyed by the concrete error type. Dispatching an error
/// invokes every callback registered for that exact type.
pub struct ErrorDispatcher {
    subscribers: RwLock<HashMap<TypeId, Vec<Arc<Mutex<ErrorCallback>>>>>,
}

impl Default for ErrorDispatcher {
    fn default() -> Self {
        Self::new(0)
    }
}

impl ErrorDispatcher {
    /// Creates an empty dispatcher.
    ///
    /// `capacity` is retained for API compatibility; callbacks are stored in a
    /// growable map and the value does not currently reserve storage.
    pub fn new(_capacity: u64) -> Self {
        Self {
            subscribers: RwLock::new(HashMap::new()),
        }
    }

    /// Registers a callback for errors of type `T`.
    pub fn subscribe<T: Error + 'static>(&self, callback: ErrorCallback) {
        self.subscribers
            .write()
            .unwrap()
            .entry(TypeId::of::<T>())
            .or_default()
            .push(Arc::new(Mutex::new(callback)));
    }

    /// Removes all callbacks registered for `T`, returning whether any existed.
    pub fn unsubscribe<T: Error + 'static>(&self) -> bool {
        self.subscribers
            .write()
            .unwrap()
            .remove(&TypeId::of::<T>())
            .is_some()
    }

    /// Delivers an error to all callbacks registered for its concrete type.
    pub fn dispatch<T: Error + Send + Sync + 'static>(&self, error: T) {
        let shared: SharedError = Arc::new(error);
        if let Some(callbacks) = self
            .subscribers
            .write()
            .unwrap()
            .get_mut(&TypeId::of::<T>())
        {
            for callback in callbacks.iter() {
                (callback.lock().unwrap())(Arc::clone(&shared));
            }
        }
    }
}

/// Delivers decoded CD and external-trigger batches to subscribers.
pub struct EventDispatcher {
    cd_subscribers: RwLock<Vec<Arc<Mutex<CDCallback>>>>,
    ext_subscribers: RwLock<Vec<Arc<Mutex<ExtCallback>>>>,
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl EventDispatcher {
    /// Creates an empty event dispatcher.
    pub fn new() -> Self {
        Self {
            cd_subscribers: RwLock::new(Vec::new()),
            ext_subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Registers a callback for decoded change-detection batches.
    pub fn subscribe_cd(&self, callback: CDCallback) {
        self.cd_subscribers
            .write()
            .unwrap()
            .push(Arc::new(Mutex::new(callback)));
    }

    /// Registers a callback for decoded external-trigger batches.
    pub fn subscribe_ext(&self, callback: ExtCallback) {
        self.ext_subscribers
            .write()
            .unwrap()
            .push(Arc::new(Mutex::new(callback)));
    }

    /// Sends a change-detection batch to all CD subscribers.
    pub fn send_cd(&self, events: &[EventCD]) {
        for callback in self.cd_subscribers.read().unwrap().iter() {
            (callback.lock().unwrap())(events);
        }
    }

    /// Sends an external-trigger batch to all trigger subscribers.
    pub fn send_ext(&self, events: &[EventExtTrigger]) {
        for callback in self.ext_subscribers.read().unwrap().iter() {
            (callback.lock().unwrap())(events);
        }
    }
}
