/// Errors from audio/video device operations.
#[derive(Debug, thiserror::Error)]
pub enum AvError {
    #[error("device not found: {0}")]
    DeviceNotFound(String),

    #[error("permission denied")]
    PermissionDenied,

    #[error("device in use")]
    DeviceInUse,

    #[error("format not supported: {0}")]
    FormatUnsupported(String),

    #[error("device disconnected")]
    Disconnected,

    #[error("{0}")]
    Backend(String),
}
