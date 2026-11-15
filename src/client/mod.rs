//! The client side of the control protocol: what the command line (and the desktop window's Rust side) uses to talk
//! to a daemon. It finds the local daemon through the [`discovery`](crate::discovery) file, or connects to an explicit
//! address and token (a daemon on another machine).
//!
//! Calls are typed by the protocol's [`Method`]s: `client.call::<GraphGet>(&())`.

use crate::discovery::{self, Discovery};
use crate::protocol::methods::Method;
use crate::protocol::{Request, Response, RpcError};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

#[derive(Debug)]
pub enum ClientError {
    /// No daemon is running (or its discovery file is missing).
    NoDaemon(String),
    /// The daemon could not be reached or answered in a way that is not JSON-RPC.
    Transport(String),
    /// The daemon answered with an error.
    Rpc(RpcError),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NoDaemon(m) | ClientError::Transport(m) => f.write_str(m),
            ClientError::Rpc(e) => write!(f, "{}", e.message),
        }
    }
}

impl std::error::Error for ClientError {}

/// How long a request may take before the client gives up: the daemon answers within moments, except when it waits
/// for a person (cleaning up old devices asks the user for administrator rights, and the answer takes as long as the
/// user does).
pub fn wait_for(method: &str) -> Duration {
    if method == crate::protocol::methods::DevicesClean::NAME {
        Duration::from_secs(200)
    } else {
        Duration::from_secs(30)
    }
}

pub struct Client {
    address: String,
    token: String,
    agent: ureq::Agent,
    next_id: AtomicI64,
}

impl Client {
    pub fn new(address: impl Into<String>, token: impl Into<String>) -> Client {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Client {
            address: address.into(),
            token: token.into(),
            agent,
            next_id: AtomicI64::new(1),
        }
    }

    /// Connects to the daemon running for this user on this machine.
    pub fn discover() -> Result<Client, ClientError> {
        Client::discover_in(&discovery::data_dir())
    }

    pub fn discover_in(dir: &std::path::Path) -> Result<Client, ClientError> {
        let d = Discovery::read(dir).ok_or_else(|| {
            ClientError::NoDaemon(format!(
                "no daemon found (no {}): start it with `{}`",
                Discovery::path_in(dir).display(),
                crate::branding::DAEMON_EXECUTABLE
            ))
        })?;
        Ok(Client::new(d.address, d.token))
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    /// The address of the web panel with the token in the fragment.
    pub fn panel_url(&self) -> String {
        format!("http://{}/#token={}", self.address, self.token)
    }

    /// Sends a raw request and returns the response (the transport is HTTP, `POST /rpc`).
    pub fn send(&self, request: &Request) -> Result<Response, ClientError> {
        let url = format!("http://{}/rpc", self.address);
        let mut response = self
            .agent
            .post(&url)
            .config()
            .timeout_global(Some(wait_for(&request.method)))
            .build()
            .header("Authorization", format!("Bearer {}", self.token))
            .send_json(request)
            .map_err(|e| self.unreachable(e))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        match serde_json::from_str::<Response>(&text) {
            Ok(r) => Ok(r),
            Err(_) if status == 403 => Err(ClientError::Transport(
                "the daemon refused the request (origin not allowed)".into(),
            )),
            Err(_) => Err(ClientError::Transport(format!(
                "unexpected answer from the daemon (HTTP {status}): {text}"
            ))),
        }
    }

    fn unreachable(&self, e: ureq::Error) -> ClientError {
        ClientError::NoDaemon(format!(
            "cannot reach the daemon at {}: {e} (is it running? start it with `{}`)",
            self.address,
            crate::branding::DAEMON_EXECUTABLE
        ))
    }

    /// Calls method `M`.
    pub fn call<M: Method>(&self, params: &M::Params) -> Result<M::Result, ClientError> {
        let params =
            serde_json::to_value(params).map_err(|e| ClientError::Transport(e.to_string()))?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = self.send(&Request::new(id, M::NAME, params))?;
        if let Some(e) = response.error {
            return Err(ClientError::Rpc(e));
        }
        serde_json::from_value(response.result.unwrap_or(serde_json::Value::Null))
            .map_err(|e| ClientError::Transport(format!("{}: unexpected result: {e}", M::NAME)))
    }
}

#[cfg(test)]
mod wait_tests {
    use super::*;

    #[test]
    fn only_the_request_that_waits_for_a_person_gets_a_long_delay() {
        assert!(wait_for("devices.clean") >= Duration::from_secs(190));
        for m in ["system.info", "graph.set", "devices.stale", "engine.status"] {
            assert_eq!(wait_for(m), Duration::from_secs(30), "{m}");
        }
    }
}
