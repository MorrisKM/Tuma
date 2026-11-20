//! Watches the open workspace's folder, so what changes there outside the app (a coding agent saving requests over
//! MCP, git, an editor) shows up without reopening it. The app is told with a `workspace-changed` event once a burst
//! of changes has settled, and reloads the tree from disk.

use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{Emitter, Manager};

/// How long the folder must be still before the app is told, so saving many files reloads the tree once.
const SETTLE: Duration = Duration::from_millis(300);

/// The watcher of the open workspace, if one is open. Dropping it stops the watching and its thread.
#[derive(Default)]
pub struct Watching(Mutex<Option<RecommendedWatcher>>);

/// Watches `root`, or stops watching with `None`. Failing to watch only means changes show up when reopened.
pub fn watch(app: &tauri::AppHandle, root: Option<&Path>) {
    let state = app.state::<Watching>();
    let mut current = state.0.lock().unwrap();
    *current = None;
    let Some(root) = root else { return };
    let root = root.to_path_buf();

    let (changed, settled) = mpsc::channel::<()>();
    let base = root.clone();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if event.paths.iter().any(|path| counts(&base, path)) {
            let _ = changed.send(());
        }
    });
    let Ok(mut watcher) = watcher else { return };
    if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || {
        // Ends when the watcher, which holds the sender, is dropped.
        while settled.recv().is_ok() {
            while settled.recv_timeout(SETTLE).is_ok() {}
            let _ = app.emit("workspace-changed", ());
        }
    });
    *current = Some(watcher);
}

/// Whether a change to `path` can change what the app shows: not inside a hidden folder such as `.git`, and not an
/// editor's temporary file.
fn counts(root: &Path, path: &Path) -> bool {
    let relative: PathBuf = path.strip_prefix(root).unwrap_or(path).to_path_buf();
    let hidden = relative.components().any(|c| match c {
        Component::Normal(name) => name.to_string_lossy().starts_with('.'),
        _ => false,
    });
    let temporary = relative
        .file_name()
        .map(|n| n.to_string_lossy())
        .is_some_and(|n| n.ends_with('~') || n.ends_with(".swp") || n.ends_with(".tmp"));
    !hidden && !temporary
}

#[cfg(test)]
mod tests {
    use super::counts;
    use std::path::Path;

    #[test]
    fn hidden_folders_and_temporary_files_do_not_count() {
        let root = Path::new("/w/.tuma");
        assert!(counts(root, Path::new("/w/.tuma/Users/create.yaml")));
        assert!(counts(root, Path::new("/w/.tuma/New folder")));
        assert!(!counts(root, Path::new("/w/.tuma/.git/index")));
        assert!(!counts(root, Path::new("/w/.tuma/Users/create.yaml~")));
        assert!(!counts(root, Path::new("/w/.tuma/Users/.create.yaml.swp")));
    }
}
