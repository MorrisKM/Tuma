//! Every request sent, newest last, in a JSON Lines file. Appending one line per request keeps writes cheap and a
//! crash can lose at most the line being written.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::workspace::files::RequestFile;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// Unix time in milliseconds.
    pub sent_at: u64,
    /// The request as written, before variables were resolved.
    pub request: RequestFile,
    /// Where the request is saved in the workspace, if it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `None` when the request failed before a response arrived.
    pub status: Option<u16>,
    pub error: Option<String>,
    pub total_ms: Option<f64>,
    pub size: Option<usize>,
    /// Who sent it when it was not the desktop app: `cli`, or `mcp` plus the agent's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

pub struct History {
    path: PathBuf,
}

impl History {
    /// The history the desktop app shows, shared with the CLI and the MCP server: Tauri's app data folder for the
    /// app's identifier, or `TUMA_DATA_DIR` when set.
    pub fn default_location() -> Option<Self> {
        if let Some(dir) = std::env::var_os("TUMA_DATA_DIR") {
            return Some(Self::new(dir));
        }
        dirs::data_dir().map(|d| Self::new(d.join("dev.tuma.app")))
    }

    /// Keeps history in `history.jsonl` inside `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            path: dir.into().join("history.jsonl"),
        }
    }

    pub fn append(&self, entry: &Entry) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?
            .write_all(line.as_bytes())
    }

    /// The newest `limit` entries, newest first. Lines that do not parse (a torn write, an older format) are skipped.
    pub fn recent(&self, limit: usize) -> io::Result<Vec<Entry>> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut entries: Vec<Entry> = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str(&line).ok())
            .collect();
        entries.reverse();
        entries.truncate(limit);
        Ok(entries)
    }

    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            sent_at: 1,
            request: RequestFile {
                url: "example.com".into(),
                ..Default::default()
            },
            path: None,
            status: Some(200),
            error: None,
            total_ms: Some(1.5),
            size: Some(10),
            source: None,
        }
    }

    #[test]
    fn append_recent_clear() {
        let dir = std::env::temp_dir().join(format!("tuma-history-{}", std::process::id()));
        let history = History::new(&dir);
        assert!(history.recent(10).unwrap().is_empty());

        for id in ["a", "b", "c"] {
            history.append(&entry(id)).unwrap();
        }
        // A torn line from a crash is skipped.
        OpenOptions::new()
            .append(true)
            .open(dir.join("history.jsonl"))
            .unwrap()
            .write_all(b"{\"id\":")
            .unwrap();

        let ids: Vec<_> = history
            .recent(2)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, ["c", "b"]);
        assert_eq!(history.recent(10).unwrap()[2], entry("a"));

        history.clear().unwrap();
        assert!(history.recent(10).unwrap().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
