use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use chrono::Utc;

use crate::schema::TraceEvent;

struct SinkState {
    date_stamp: String,
    file: File,
}

pub struct JsonlSink {
    dir: PathBuf,
    state: Mutex<SinkState>,
}

impl JsonlSink {
    pub fn new(dir: PathBuf) -> Result<Self> {
        create_dir_all(&dir)?;
        let date_stamp = Self::current_date_stamp();
        let file = Self::open_file(&dir, &date_stamp)?;

        Ok(Self {
            dir,
            state: Mutex::new(SinkState { date_stamp, file }),
        })
    }

    pub fn append(&self, event: &TraceEvent) -> Result<()> {
        let mut state = self.state.lock().expect("jsonl sink mutex poisoned");
        self.rotate_if_needed(&mut state)?;

        let line = serde_json::to_string(event)?;
        state.file.write_all(line.as_bytes())?;
        state.file.write_all(b"\n")?;

        Ok(())
    }

    pub fn append_many(&self, events: &[TraceEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let mut state = self.state.lock().expect("jsonl sink mutex poisoned");
        self.rotate_if_needed(&mut state)?;

        for event in events {
            let line = serde_json::to_string(event)?;
            state.file.write_all(line.as_bytes())?;
            state.file.write_all(b"\n")?;
        }

        Ok(())
    }

    fn rotate_if_needed(&self, state: &mut SinkState) -> Result<()> {
        let now_stamp = Self::current_date_stamp();
        if now_stamp != state.date_stamp {
            state.file = Self::open_file(&self.dir, &now_stamp)?;
            state.date_stamp = now_stamp;
        }

        Ok(())
    }

    fn current_date_stamp() -> String {
        Utc::now().date_naive().format("%Y-%m-%d").to_string()
    }

    fn open_file(dir: &PathBuf, date_stamp: &str) -> Result<File> {
        let path = dir.join(format!("{date_stamp}.jsonl"));
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(file)
    }
}
