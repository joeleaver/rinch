/// Identifier for an audio or video device.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

/// Human-readable device information.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub is_default: bool,
}

/// The kind of media device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    AudioOutput,
    AudioInput,
    Camera,
}
