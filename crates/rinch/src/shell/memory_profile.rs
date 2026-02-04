//! Memory profiling utilities for tracking memory consumption.
//!
//! Enable with the `memory-profile` feature:
//! ```toml
//! rinch = { path = "...", features = ["memory-profile"] }
//! ```
//!
//! This module provides utilities to track memory usage at key points
//! during application startup and rendering.

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(true);

/// Memory snapshot at a point in time.
#[derive(Debug, Clone, Copy)]
pub struct MemorySnapshot {
    /// Physical memory (RSS) in bytes.
    pub physical: usize,
    /// Virtual memory in bytes.
    pub virtual_mem: usize,
}

impl MemorySnapshot {
    /// Take a memory snapshot now.
    pub fn now() -> Option<Self> {
        #[cfg(feature = "memory-profile")]
        {
            memory_stats::memory_stats().map(|stats| Self {
                physical: stats.physical_mem,
                virtual_mem: stats.virtual_mem,
            })
        }
        #[cfg(not(feature = "memory-profile"))]
        {
            None
        }
    }

    /// Format as megabytes.
    pub fn physical_mb(&self) -> f64 {
        self.physical as f64 / 1024.0 / 1024.0
    }

    /// Format as megabytes.
    pub fn virtual_mb(&self) -> f64 {
        self.virtual_mem as f64 / 1024.0 / 1024.0
    }
}

/// Enable or disable memory profiling output.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Check if memory profiling is enabled.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Log a memory checkpoint with a label.
pub fn checkpoint(label: &str) {
    if !is_enabled() {
        return;
    }

    if let Some(snapshot) = MemorySnapshot::now() {
        tracing::info!(
            "[memory] {}: {:.1} MB (virtual: {:.1} MB)",
            label,
            snapshot.physical_mb(),
            snapshot.virtual_mb()
        );
    }
}

/// Log memory difference between two snapshots.
pub fn checkpoint_delta(label: &str, before: Option<MemorySnapshot>) {
    if !is_enabled() {
        return;
    }

    if let (Some(before), Some(after)) = (before, MemorySnapshot::now()) {
        let physical_delta = after.physical as i64 - before.physical as i64;
        let delta_mb = physical_delta as f64 / 1024.0 / 1024.0;
        let sign = if delta_mb >= 0.0 { "+" } else { "" };

        tracing::info!(
            "[memory] {}: {:.1} MB ({}{:.1} MB)",
            label,
            after.physical_mb(),
            sign,
            delta_mb
        );
    }
}

/// RAII guard that logs memory delta when dropped.
pub struct MemoryGuard {
    label: &'static str,
    before: Option<MemorySnapshot>,
}

impl MemoryGuard {
    /// Start tracking memory for a labeled section.
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            before: MemorySnapshot::now(),
        }
    }
}

impl Drop for MemoryGuard {
    fn drop(&mut self) {
        checkpoint_delta(self.label, self.before);
    }
}

/// Macro for easy memory tracking of a code block.
#[macro_export]
macro_rules! memory_track {
    ($label:expr, $body:expr) => {{
        let _guard = $crate::shell::memory_profile::MemoryGuard::new($label);
        $body
    }};
}

/// Print a detailed memory report.
pub fn print_report() {
    if !is_enabled() {
        return;
    }

    if let Some(snapshot) = MemorySnapshot::now() {
        tracing::info!("╔══════════════════════════════════════════╗");
        tracing::info!("║         Memory Profile Report            ║");
        tracing::info!("╠══════════════════════════════════════════╣");
        tracing::info!(
            "║  Physical (RSS):    {:>10.1} MB       ║",
            snapshot.physical_mb()
        );
        tracing::info!(
            "║  Virtual:           {:>10.1} MB       ║",
            snapshot.virtual_mb()
        );
        tracing::info!("╚══════════════════════════════════════════╝");
    }
}
