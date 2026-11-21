//! Which certificates the program trusts when it connects to a server over HTTPS (to download a module, to read a
//! stream...), and what it found in the folder where the user keeps their own.

use crate::Diagnostic;
use serde::{Deserialize, Serialize};

/// Whose certificates the program believes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustMode {
    /// What the operating system trusts, plus the certificates in the certificates folder (a company's own authority, for example).
    #[default]
    System,
    /// Only the certificates in the certificates folder. Nothing the system trusts is believed.
    Own,
}

/// One certificate found in the certificates folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateInfo {
    /// The file it was read from (its name, inside the folder).
    pub file: String,
    /// Who the certificate is about.
    pub subject: String,
    /// Who vouches for it (the same as the subject for an authority's own certificate).
    pub issuer: String,
    /// The last day it is valid, as `YYYY-MM-DD`.
    pub expires: String,
    pub expired: bool,
}

/// What the program trusts now: the mode, the folder and everything found in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustReport {
    pub mode: TrustMode,
    /// The certificates folder (where to put certificates).
    pub folder: String,
    pub folder_exists: bool,
    pub certificates: Vec<CertificateInfo>,
    /// Files that could not be read, expired certificates, a mode that would trust nothing...
    pub problems: Vec<Diagnostic>,
}
