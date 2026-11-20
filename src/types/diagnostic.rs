//! What is wrong (or worth knowing) about a graph, as data: the same for every client. The command line prints it, the panel
//! shows it in its Problems list and takes the person to the block, a script reads the `code`.

use crate::node::NodeId;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The graph is refused.
    Error,
    /// It runs, but something is probably not what was meant.
    Warning,
    /// Worth knowing.
    Info,
}

/// One problem. `code` is stable (a script can rely on it), `message` is for people, `hint` says what to do about it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    /// The block it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    /// The connection point of that block it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            code: code.to_owned(),
            message: message.into(),
            node: None,
            port: None,
            hint: None,
        }
    }

    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(code, message)
        }
    }

    pub fn at(mut self, node: &NodeId) -> Self {
        self.node = Some(node.clone());
        self
    }

    pub fn at_port(mut self, node: &NodeId, port: &str) -> Self {
        self.node = Some(node.clone());
        self.port = Some(port.to_owned());
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diagnostic_is_flat_json_and_leaves_out_what_it_does_not_say() {
        let d = Diagnostic::error("type_mismatch", "a sound does not go into a number")
            .at_port(&NodeId::from("g"), "db");
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["severity"], "error");
        assert_eq!(v["code"], "type_mismatch");
        assert_eq!(v["node"], "g");
        assert_eq!(v["port"], "db");
        assert!(v.get("hint").is_none());
        assert_eq!(serde_json::from_value::<Diagnostic>(v).unwrap(), d);
        assert_eq!(d.to_string(), "a sound does not go into a number");
    }

    #[test]
    fn a_warning_is_not_an_error() {
        assert_eq!(
            Diagnostic::warning("feedback", "x").severity,
            Severity::Warning
        );
    }
}
