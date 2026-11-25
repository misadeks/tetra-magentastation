use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
};

struct Fixture {
    child: Child,
    config: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.config);
    }
}

#[allow(clippy::result_large_err, reason = "tungstenite requires this concrete HTTP response error type")]
fn control_handshake(request: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    assert_eq!(request.headers()["sec-websocket-protocol"], "bluestation-control-v1");
    response
        .headers_mut()
        .insert("sec-websocket-protocol", "bluestation-control-v1".parse().unwrap());
    Ok(response)
}

#[test]
fn executable_rejects_non_ms_roles_before_constructing_a_radio() {
    for role in ["Bs", "Mon"] {
        let path = std::env::temp_dir().join(format!("magentastation-role-{}-{role}.toml", std::process::id()));
        fs::write(
            &path,
            format!(
                "config_version = \"0.7\"\nstack_mode = \"{role}\"\n\
                 [phy_io]\nbackend = \"None\"\n[net_info]\nmcc = 901\nmnc = 9999\n"
            ),
        )
        .unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_magentastation")).arg(&path).output();
        fs::remove_file(path).unwrap();
        let output = result.unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains("unknown variant"));
    }
}

#[test]
fn offline_ms_process_exposes_management_without_radio_or_protocol_ticks() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = std::env::temp_dir().join(format!(
        "bluestation-offline-{}-{}.toml",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &config,
        format!(
            r#"config_version = "0.7"
stack_mode = "Ms"
[phy_io]
backend = "None"
[net_info]
mcc = 901
mnc = 9999
[ms]
issi = 1000001
subscriber_class = 1
attach_groups = []
[command]
host = "127.0.0.1"
port = {port}
use_tls = false
"#
        ),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_magentastation"))
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut fixture = Fixture { child, config };
    let deadline = Instant::now() + Duration::from_secs(15);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(fixture.child.try_wait().unwrap().is_none(), "MS process exited before connecting");
                assert!(Instant::now() < deadline, "MS control connection timed out");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("control accept failed: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut socket = tungstenite::accept_hdr(stream, control_handshake).unwrap();

    for (handle, command, variant) in [
        (41, "GetInterfaceVersion", "InterfaceVersion"),
        (42, "GetState", "State"),
        (43, "GetConfig", "Config"),
        (44, "GetState", "State"),
    ] {
        socket
            .send(Message::Binary(
                json!({"Management": {command: {"handle": handle}}}).to_string().into_bytes(),
            ))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let response: Value = loop {
            assert!(Instant::now() < deadline, "MS control response timed out");
            match socket.read().unwrap() {
                Message::Text(data) => break serde_json::from_str(&data).unwrap(),
                Message::Binary(data) => break serde_json::from_slice(&data).unwrap(),
                Message::Ping(_) | Message::Pong(_) => socket.flush().unwrap(),
                message => panic!("unexpected control message: {message:?}"),
            }
        };
        let payload = &response["Management"][variant];
        assert_eq!(payload["handle"], handle);
        match command {
            "GetInterfaceVersion" => assert_eq!(payload["version"], tetra_entities::management::MS_INTERFACE_SCHEMA_VERSION),
            "GetState" => {
                assert_eq!(payload["state"]["own_issi"], 1000001);
                assert_eq!(payload["state"]["registration_state"], "Idle");
            }
            "GetConfig" => {
                let config = payload["toml"].as_str().unwrap();
                assert!(config.contains("backend = \"None\""));
            }
            _ => unreachable!(),
        }
    }
    assert!(fixture.child.try_wait().unwrap().is_none());
    socket.close(None).unwrap();
}
