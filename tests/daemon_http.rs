//! The whole daemon stack over real sockets: HTTP, WebSocket, authentication, the typed client.
//! The audio is simulated (`demo`), so these tests run anywhere and make no sound.

use omu::client::{Client, ClientError};
use omu::daemon::launch::{Options, Running};
use omu::protocol::methods::*;
use omu::protocol::RpcError;
use omu::types::{AppTarget, EdgeSpec, GraphSpec, NodeKind, NodeSpec, NodeState, PortRef};
use std::time::{Duration, Instant};

fn start(tag: &str) -> (Running, Client, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("omu-http-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut opts = Options::new(dir.clone());
    opts.listen = "127.0.0.1:0".parse().unwrap();
    opts.demo = true;
    let running = Running::start(opts).expect("daemon starts");
    let client = Client::new(running.info.address.to_string(), running.info.token.clone());
    (running, client, dir)
}

fn chain() -> GraphSpec {
    GraphSpec {
        nodes: vec![
            NodeSpec {
                id: "app".into(),
                display: None,
                kind: NodeKind::AppCapture {
                    target: AppTarget::ProcessName("chrome.exe".into()),
                },
            },
            NodeSpec {
                id: "mic".into(),
                display: None,
                kind: NodeKind::VirtualMic {
                    name: "Stream".into(),
                },
            },
        ],
        edges: vec![EdgeSpec {
            from: PortRef::new("app", "out"),
            to: PortRef::new("mic", "in"),
        }],
    }
}

#[test]
fn typed_calls_work_end_to_end() {
    let (running, client, dir) = start("typed");
    let info = client.call::<SystemInfoMethod>(&()).unwrap();
    assert_eq!(info.backend, "simulated");
    assert!(info.virtual_microphones.unwrap().available);

    let apps = client.call::<AppsList>(&()).unwrap();
    assert!(apps.iter().any(|a| a.name == "chrome.exe"));
    assert!(!client.call::<DevicesList>(&()).unwrap().is_empty());

    client.call::<GraphSet>(&chain()).unwrap();
    let start = Instant::now();
    let status = loop {
        let s = client.call::<EngineStatusMethod>(&()).unwrap();
        if s.nodes.len() == 2
            && s.nodes
                .iter()
                .all(|n| n.state == NodeState::Running && n.peak > 0.05)
        {
            break s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "audio never flowed: {s:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(status.running);
    assert_eq!(client.call::<GraphGet>(&()).unwrap(), chain());

    client.call::<EngineStop>(&()).unwrap();
    assert!(!client.call::<EngineStatusMethod>(&()).unwrap().running);
    running.stop();
    assert!(
        !dir.join("daemon.json").exists(),
        "the discovery file is removed on shutdown"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validation_errors_reach_the_client_with_details() {
    let (running, client, dir) = start("validation");
    let bad = GraphSpec {
        nodes: vec![
            NodeSpec {
                id: "x".into(),
                display: None,
                kind: NodeKind::Mixer,
            },
            NodeSpec {
                id: "x".into(),
                display: None,
                kind: NodeKind::Mixer,
            },
        ],
        edges: vec![],
    };
    match client.call::<GraphSet>(&bad) {
        Err(ClientError::Rpc(e)) => {
            assert_eq!(e.code, RpcError::VALIDATION);
            assert!(!e.data.unwrap().as_array().unwrap().is_empty());
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_or_missing_token_is_refused() {
    let (running, client, dir) = start("auth");
    let wrong = Client::new(client.address(), "not-the-token");
    match wrong.call::<SystemInfoMethod>(&()) {
        Err(ClientError::Rpc(e)) => assert_eq!(e.code, RpcError::UNAUTHORIZED),
        other => panic!("expected unauthorized, got {other:?}"),
    }
    let r = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let status = r
        .post(&format!("http://{}/rpc", client.address()))
        .send("{}")
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(status, 401);
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_browser_from_another_site_is_refused_even_with_the_token() {
    let (running, client, dir) = start("origin");
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let url = format!("http://{}/rpc", client.address());
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"system.info"}"#;
    let auth = format!("Bearer {}", running.info.token);
    let evil = agent
        .post(&url)
        .header("Authorization", auth.as_str())
        .header("Origin", "https://evil.example")
        .send(body)
        .unwrap();
    assert_eq!(evil.status().as_u16(), 403);
    let own_origin = format!("http://{}", client.address());
    let ok = agent
        .post(&url)
        .header("Authorization", auth.as_str())
        .header("Origin", own_origin.as_str())
        .send(body)
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn health_and_the_panel_page_need_no_token() {
    let (running, client, dir) = start("static");
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let health = agent
        .get(&format!("http://{}/health", client.address()))
        .call()
        .unwrap();
    assert_eq!(health.status().as_u16(), 200);
    let page = agent
        .get(&format!("http://{}/", client.address()))
        .call()
        .unwrap();
    assert_eq!(page.status().as_u16(), 200);
    let missing = agent
        .get(&format!("http://{}/nope.js", client.address()))
        .call()
        .unwrap();
    assert_eq!(missing.status().as_u16(), 404);
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_websocket_answers_requests_and_pushes_status_events() {
    let (running, client, dir) = start("ws");
    let url = format!("ws://{}/ws?token={}", client.address(), running.info.token);
    let (mut socket, _) = tungstenite::connect(url).expect("websocket connects");
    socket
        .send(tungstenite::Message::text(
            r#"{"jsonrpc":"2.0","id":9,"method":"system.info"}"#,
        ))
        .unwrap();
    let (mut got_reply, mut got_event) = (false, false);
    let start = Instant::now();
    while !(got_reply && got_event) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "reply {got_reply}, event {got_event}"
        );
        if let tungstenite::Message::Text(t) = socket.read().unwrap() {
            let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
            if v["id"] == 9 {
                assert_eq!(v["result"]["backend"], "simulated");
                got_reply = true;
            }
            if v["method"] == EVENT_STATUS {
                got_event = true;
            }
        }
    }
    drop(socket);
    // the same endpoint without a token is refused at the handshake
    assert!(tungstenite::connect(format!("ws://{}/ws", client.address())).is_err());
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_daemon_on_the_same_port_says_so() {
    let (running, client, dir) = start("busy");
    let mut opts = Options::new(dir.join("other"));
    opts.listen = client.address().parse().unwrap();
    opts.demo = true;
    let err = Running::start(opts).err().expect("the port is taken");
    assert!(err.contains("already in use"), "{err}");
    running.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
