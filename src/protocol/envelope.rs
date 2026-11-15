//! JSON-RPC 2.0 envelopes (<https://www.jsonrpc.org/specification>).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

/// A request id: a number or a string, echoed in the response.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(i64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// Absent for a notification (no response expected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    pub fn new(id: i64, method: &str, params: Value) -> Request {
        Request {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(Id::Number(id)),
            method: method.into(),
            params,
        }
    }
}

/// A message pushed by the daemon (an event): a request without an id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

impl Notification {
    pub fn new(method: &str, params: Value) -> Notification {
        Notification {
            jsonrpc: JSONRPC_VERSION.into(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub const PARSE: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL: i32 = -32603;
    /// The graph was rejected by validation; `data` lists the problems.
    pub const VALIDATION: i32 = -32001;
    /// The request needs an audio facility this system does not have.
    pub const UNAVAILABLE: i32 = -32002;
    pub const UNAUTHORIZED: i32 = -32003;
    /// The user refused something the system asked them (administrator rights); nothing was done.
    pub const CANCELLED: i32 = -32004;
    /// What was asked needs an optional module that is not switched on; `data` is the module's id.
    pub const MODULE_NEEDED: i32 = -32005;

    pub fn new(code: i32, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
            data: None,
        }
    }
    pub fn with_data(mut self, data: Value) -> RpcError {
        self.data = Some(data);
        self
    }
    pub fn parse(message: impl Into<String>) -> RpcError {
        RpcError::new(Self::PARSE, message)
    }
    pub fn invalid_request(message: impl Into<String>) -> RpcError {
        RpcError::new(Self::INVALID_REQUEST, message)
    }
    pub fn method_not_found(method: &str) -> RpcError {
        RpcError::new(Self::METHOD_NOT_FOUND, format!("unknown method {method:?}"))
    }
    pub fn invalid_params(message: impl Into<String>) -> RpcError {
        RpcError::new(Self::INVALID_PARAMS, message)
    }
    pub fn internal(message: impl Into<String>) -> RpcError {
        RpcError::new(Self::INTERNAL, message)
    }
    pub fn unauthorized() -> RpcError {
        RpcError::new(Self::UNAUTHORIZED, "missing or wrong access token")
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

/// A response: exactly one of `result` and `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    /// `None` (serialised as `null`) when the request id could not be read.
    pub id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Option<Id>, result: Value) -> Response {
        Response {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: Option<Id>, error: RpcError) -> Response {
        Response {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trips_with_numeric_and_text_ids() {
        for id in [Id::Number(7), Id::Text("abc".into())] {
            let r = Request {
                jsonrpc: "2.0".into(),
                id: Some(id),
                method: "m".into(),
                params: json!({"a": 1}),
            };
            let back: Request = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
            assert_eq!(r, back);
        }
    }

    #[test]
    fn missing_params_and_id_are_accepted() {
        let r: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"system.info"}"#).unwrap();
        assert_eq!(r.id, None);
        assert!(r.params.is_null());
    }

    #[test]
    fn a_response_has_result_or_error_never_both() {
        let ok = serde_json::to_value(Response::ok(Some(Id::Number(1)), json!(true))).unwrap();
        assert!(ok.get("error").is_none() && ok["result"] == true);
        let err =
            serde_json::to_value(Response::err(None, RpcError::method_not_found("x"))).unwrap();
        assert!(err.get("result").is_none());
        assert_eq!(err["id"], serde_json::Value::Null);
        assert_eq!(err["error"]["code"], RpcError::METHOD_NOT_FOUND);
    }

    #[test]
    fn error_data_is_optional_and_kept() {
        let e = RpcError::new(RpcError::VALIDATION, "bad").with_data(json!(["a", "b"]));
        let back: RpcError = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back.data, Some(json!(["a", "b"])));
        assert!(serde_json::to_value(RpcError::internal("x"))
            .unwrap()
            .get("data")
            .is_none());
    }
}
