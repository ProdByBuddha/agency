use std::env;
use std::process::Command;
use serde_json::{json, Value};

fn discover_devices() -> Value {
    if env::consts::OS == "macos" {
        return json!({
            "status": "success",
            "devices": [{"name": "Mock Mac Device", "address": "00:11:22:33:44:55"}]
        });
    }

    // Linux hcitool scan
    let output = Command::new("hcitool")
        .arg("scan")
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                json!({
                    "status": "success",
                    "output": String::from_utf8_lossy(&out.stdout)
                })
            } else {
                json!({
                    "status": "error",
                    "message": String::from_utf8_lossy(&out.stderr)
                })
            }
        },
        Err(e) => json!({ "status": "error", "message": e.to_string() })
    }
}

fn interact(device_name: &str, action: &str, payload: &str) -> Value {
    match action {
        "read" => json!({
            "status": "success",
            "message": format!("[MOCK RUST] Read data from {}", device_name)
        }),
        "write" => json!({
            "status": "success",
            "message": format!("[MOCK RUST] Wrote '{}' to {}", payload, device_name)
        }),
        _ => json!({
            "status": "error",
            "message": format!("Unknown action: {}", action)
        })
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("{}", json!({ "status": "error", "message": "No parameters provided" }));
        std::process::exit(1);
    }

    let params: Value = match serde_json::from_str(&args[1]) {
        Ok(v) => v,
        Err(e) => {
            println!("{}", json!({ "status": "error", "message": format!("Invalid JSON: {}", e) }));
            std::process::exit(1);
        }
    };

    let action = params["action"].as_str().unwrap_or("");
    
    let result = match action {
        "discover" => discover_devices(),
        "read" | "write" => {
            let device_name = params["device_name"].as_str().unwrap_or("unknown");
            let payload = params["payload"].as_str().unwrap_or("");
            interact(device_name, action, payload)
        },
        _ => json!({ "status": "error", "message": format!("Unsupported action: {}", action) })
    };

    println!("{}", result);
}
