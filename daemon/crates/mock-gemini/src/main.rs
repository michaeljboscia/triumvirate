use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let notify = serde_json::json!({
        "jsonrpc":"2.0",
        "method":"session/ready",
        "params":{"text":"mock-gemini ready"}
    });
    let _ = writeln!(stdout, "{}", notify);

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }

        let response = serde_json::json!({
            "jsonrpc":"2.0",
            "id": 1,
            "result": { "text": format!("mock-gemini received: {}", line.trim()) }
        });

        let _ = writeln!(stdout, "{}", response);
        let _ = stdout.flush();
    }
}
