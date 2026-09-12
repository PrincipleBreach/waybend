use std::{
    io::Write,
    process::{Command, Stdio},
};

use serde_json::{Value, json};
use uuid::Uuid;

#[test]
fn mcp_stdout_contains_only_json_rpc_messages() {
    let database = std::env::temp_dir().join(format!("waybend-mcp-{}.sqlite3", Uuid::new_v4()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_waybend"))
        .arg("mcp")
        .env("WAYBEND_DATABASE", &database)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Waybend MCP server");

    let requests = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "integration-test", "version": "1"}
            }
        }),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    ];
    {
        let stdin = child.stdin.as_mut().expect("piped stdin");
        for request in requests {
            writeln!(stdin, "{request}").expect("write JSON-RPC request");
        }
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("wait for MCP server");
    let _ = std::fs::remove_file(database);
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    let messages = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("stdout must be JSON only"))
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    let tools = messages
        .iter()
        .find(|message| message["id"] == 2)
        .expect("tools/list response")["result"]["tools"]
        .as_array()
        .expect("tools array");
    assert_eq!(tools.len(), 5);
}
