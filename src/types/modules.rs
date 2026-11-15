//! Optional modules as the daemon reports them: components that are not part of the program and that people switch on
//! (FFmpeg today; large models later).

use serde::{Deserialize, Serialize};

/// How far a download is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: u64,
    /// 0 when the size is not known.
    pub total: u64,
}

/// One module (`modules.list`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// The module is switched on: the program uses it when it needs it.
    pub enabled: bool,
    /// Something usable is there: a copy in the modules folder, or one the system already has.
    pub available: bool,
    /// Where the usable copy is: "local" (the modules folder) or "system".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// A download is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installing: Option<Progress>,
    /// Why the last installation failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Whether the module can be downloaded on this system (otherwise only a copy the system has can be used).
    pub downloadable: bool,
    /// About how much a download is, in megabytes.
    #[serde(default)]
    pub download_mb: u32,
}
