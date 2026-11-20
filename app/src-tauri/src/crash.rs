//! A crash log, so a crash leaves something to go on: panics in the app, and errors in the window that would
//! otherwise vanish with it, are appended to `crash.log` in the app's data folder
//! (`~/.local/share/dev.tuma.app` on Linux).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// The log is cut back to its newest half past this size, so it never grows without end.
const MAX_LEN: u64 = 512 * 1024;

fn path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("dev.tuma.app").join("crash.log"))
}

/// Appends one entry, stamped with the time in seconds since 1970 (UTC). Failing to write is ignored: this runs
/// while something has already gone wrong.
pub fn write(kind: &str, text: &str) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LEN)
        && let Ok(old) = std::fs::read_to_string(&path)
    {
        let keep = floor_char(&old, old.len() / 2);
        let start = old[keep..].find("\n=== ").map_or(keep, |i| keep + i + 1);
        let _ = std::fs::write(&path, &old[start..]);
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(
            file,
            "=== {kind} at {secs} (Tuma {})\n{}\n",
            env!("CARGO_PKG_VERSION"),
            text.trim_end()
        );
    }
}

/// Logs every panic, with where it happened and a backtrace, then lets it go on as before.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        write(
            "panic",
            &format!(
                "thread '{}': {info}\n{backtrace}",
                thread.name().unwrap_or("unnamed")
            ),
        );
        previous(info);
    }));
}

/// An error the window caught: an exception, a rejected promise, or a view that failed to render.
#[tauri::command]
pub fn report_error(kind: String, message: String) {
    let end = floor_char(&message, 16 * 1024);
    write(&format!("window {kind}"), &message[..end]);
}

/// The nearest index at or before `at` that is on a character boundary, so slicing there can't panic.
fn floor_char(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}
