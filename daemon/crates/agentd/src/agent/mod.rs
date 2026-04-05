mod connector;
mod api_backend;
mod claude;
mod gemini;
mod codex;
mod health;
mod supervisor;

pub use connector::AgentConnector;
pub use claude::ClaudeConnector;
pub use gemini::GeminiConnector;
pub use codex::CodexConnector;
pub use health::SharedHealthRegistry;
pub use supervisor::{spawn_claude_supervisor, spawn_codex_supervisor, spawn_gemini_supervisor};
