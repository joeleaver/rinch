//! Hot reload support for rinch applications.
//!
//! When enabled with the `hot-reload` feature, this module provides file watching
//! capabilities that trigger UI re-renders when source files change.

use notify::{
    event::ModifyKind, Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};
use winit::event_loop::EventLoopProxy;

use super::runtime::RinchEvent;

/// Configuration for hot reload file watching.
#[derive(Debug, Clone)]
pub struct HotReloadConfig {
    /// Paths to watch for changes.
    pub watch_paths: Vec<PathBuf>,
    /// File extensions to watch (e.g., ["rs", "css", "html"]).
    pub extensions: Vec<String>,
    /// Debounce duration to prevent multiple rapid reloads.
    pub debounce: Duration,
}

impl Default for HotReloadConfig {
    fn default() -> Self {
        // Try to find source directories that exist
        let candidates = ["src", "examples", "crates"];
        let watch_paths: Vec<PathBuf> = candidates
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect();

        Self {
            watch_paths: if watch_paths.is_empty() {
                vec![PathBuf::from(".")]
            } else {
                watch_paths
            },
            extensions: vec!["rs".into(), "css".into(), "html".into()],
            debounce: Duration::from_millis(100),
        }
    }
}

impl HotReloadConfig {
    /// Create a new hot reload config watching the given paths.
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            watch_paths: paths,
            ..Default::default()
        }
    }

    /// Set the file extensions to watch.
    pub fn with_extensions(mut self, extensions: Vec<String>) -> Self {
        self.extensions = extensions;
        self
    }

    /// Set the debounce duration.
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }
}

/// Hot reloader that watches files and triggers UI re-renders.
pub struct HotReloader {
    _watcher: RecommendedWatcher,
    receiver: Receiver<Result<Event, notify::Error>>,
    config: HotReloadConfig,
    last_reload: Instant,
    proxy: EventLoopProxy<RinchEvent>,
}

impl HotReloader {
    /// Create a new hot reloader with the given configuration.
    pub fn new(
        proxy: EventLoopProxy<RinchEvent>,
        config: HotReloadConfig,
    ) -> Result<Self, notify::Error> {
        let (tx, rx) = channel();

        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        )?;

        // Watch all configured paths
        for path in &config.watch_paths {
            if path.exists() {
                watcher.watch(path, RecursiveMode::Recursive)?;
                tracing::info!("Hot reload: watching {:?}", path);
            } else {
                tracing::warn!("Hot reload: path does not exist: {:?}", path);
            }
        }

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
            config,
            last_reload: Instant::now(),
            proxy,
        })
    }

    /// Check for file changes and trigger re-render if needed.
    ///
    /// Call this periodically (e.g., in about_to_wait).
    pub fn poll(&mut self) {
        while let Ok(result) = self.receiver.try_recv() {
            match result {
                Ok(event) => {
                    if self.should_reload(&event) {
                        // Check debounce
                        let now = Instant::now();
                        if now.duration_since(self.last_reload) >= self.config.debounce {
                            self.last_reload = now;
                            tracing::info!("Hot reload: file changed, triggering re-render");
                            let _ = self.proxy.send_event(RinchEvent::ReRender);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Hot reload watch error: {:?}", e);
                }
            }
        }
    }

    /// Check if an event should trigger a reload.
    fn should_reload(&self, event: &Event) -> bool {
        // Only reload on data modifications
        if !matches!(
            event.kind,
            EventKind::Modify(ModifyKind::Data(_)) | EventKind::Create(_)
        ) {
            return false;
        }

        // Check if any of the changed files have watched extensions
        for path in &event.paths {
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if self.config.extensions.iter().any(|e| e == &ext_str) {
                    return true;
                }
            }
        }

        false
    }
}
