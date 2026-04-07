use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let init = serde_json::json!({"type":"init","content":"mock-claude ready"});
    let _ = writeln!(stdout, "{}", init);

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }

        let msg = serde_json::json!({
            "type":"message",
            "content": format!("mock-claude received: {}", line.trim())
        });
        let done = serde_json::json!({
            "type":"result",
            "result": { "text": "mock-claude completed" }
        });

        let _ = writeln!(stdout, "{}", msg);
        let _ = writeln!(stdout, "{}", done);
        let _ = stdout.flush();
    }
}
