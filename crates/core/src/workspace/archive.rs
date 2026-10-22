//! Moving whole workspaces around: into the app's workspaces folder from another folder or a `.zip`, and out to a
//! `.zip` to send to someone. Secret values stay in the OS keychain and are not in the files, so they don't travel.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use super::{WORKSPACE_FILE, Workspace, file_name};
use crate::runner::find_workspace;

/// Hidden files and folders (`.git`, editor settings) are not part of a workspace.
fn hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// A free folder name in `dir` for a workspace called `name`: its file-safe name, numbered if taken.
pub fn free_folder(dir: &Path, name: &str) -> PathBuf {
    let base = file_name(name);
    let mut candidate = dir.join(&base);
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base} ({n})"));
        n += 1;
    }
    candidate
}

/// Copies the workspace at or above `source` (a folder, or a project with `.tuma/`) or in the `.zip` at `source` into
/// a new folder in `into`, named after the workspace, and returns that folder. The original is left as it was.
pub fn import(source: &Path, into: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(into).map_err(|e| format!("could not create {}: {e}", into.display()))?;
    if source.is_dir() {
        let root = find_workspace(source).ok_or_else(|| {
            format!(
                "{} is not a Tuma workspace (it has no workspace.yaml)",
                source.display()
            )
        })?;
        let name = Workspace::open(&root)
            .map_err(|e| e.to_string())?
            .name()
            .to_string();
        let target = free_folder(into, &name);
        if target.starts_with(&root) {
            return Err("the workspaces folder is inside this workspace; pick another one".into());
        }
        copy_dir(&root, &target).map_err(|e| format!("could not copy the workspace: {e}"))?;
        Ok(target)
    } else {
        unzip(source, into)
    }
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if hidden(&name.to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

fn unzip(file: &Path, into: &Path) -> Result<PathBuf, String> {
    let reader =
        fs::File::open(file).map_err(|e| format!("could not open {}: {e}", file.display()))?;
    let mut zip = zip::ZipArchive::new(reader)
        .map_err(|e| format!("{} is not a .zip file: {e}", file.display()))?;
    // The workspace is where the shallowest workspace.yaml is: at the top, or in one folder as exports have it.
    let mut prefix: Option<PathBuf> = None;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        if path.file_name().is_some_and(|n| n == WORKSPACE_FILE) {
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
            if prefix
                .as_ref()
                .is_none_or(|p| parent.components().count() < p.components().count())
            {
                prefix = Some(parent);
            }
        }
    }
    let prefix = prefix.ok_or_else(|| {
        format!(
            "{} has no Tuma workspace in it (no workspace.yaml)",
            file.display()
        )
    })?;
    let name = {
        let mut entry = zip
            .by_name(
                &prefix
                    .join(WORKSPACE_FILE)
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
            .map_err(|e| e.to_string())?;
        let mut text = String::new();
        entry.read_to_string(&mut text).map_err(|e| e.to_string())?;
        serde_saphyr::from_str::<super::files::WorkspaceFile>(&text)
            .map(|w| w.name)
            .unwrap_or_default()
    };
    let target = free_folder(
        into,
        if name.is_empty() {
            "Imported workspace"
        } else {
            &name
        },
    );
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name refuses paths that would leave the folder (`../`, absolute paths).
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let Ok(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        if rel.as_os_str().is_empty()
            || rel
                .components()
                .any(|c| matches!(c, Component::Normal(n) if hidden(&n.to_string_lossy())))
        {
            continue;
        }
        let out = target.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        fs::write(&out, bytes).map_err(|e| format!("could not write {}: {e}", out.display()))?;
    }
    Ok(target)
}

/// Writes the workspace's files to a `.zip` at `file`, in a folder named after the workspace, and returns how many
/// files it holds. Hidden files and secret values are left out.
pub fn export(workspace: &Workspace, file: &Path) -> Result<usize, String> {
    let out =
        fs::File::create(file).map_err(|e| format!("could not create {}: {e}", file.display()))?;
    let mut zip = zip::ZipWriter::new(out);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let top = file_name(workspace.name());
    let mut count = 0;
    let mut stack = vec![workspace.root().to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<_> = fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if hidden(&entry.file_name().to_string_lossy()) || path == file {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(workspace.root()).unwrap_or(&path);
            let name = format!("{top}/{}", rel.to_string_lossy().replace('\\', "/"));
            zip.start_file(name, options).map_err(|e| e.to_string())?;
            zip.write_all(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            count += 1;
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(count)
}
