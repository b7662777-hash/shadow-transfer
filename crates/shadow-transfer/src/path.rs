use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use shadow_protocol::validate_rel_path;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PathError {
    #[error("unsafe path: {0}")]
    Unsafe(String),
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Validates a wire path and converts it into a platform path.
///
/// The rules are deliberately identical on Windows and Android so that both
/// sides accept exactly the same names: no separators other than `/`, no
/// traversal, no NUL/control characters, no characters illegal on Windows,
/// no trailing dots or spaces, no reserved DOS device names.
pub fn verify_wire_path(rel: &str) -> Result<PathBuf, PathError> {
    validate_rel_path(rel).map_err(|e| PathError::Unsafe(e.to_string()))?;
    let mut out = PathBuf::new();
    for component in rel.split('/') {
        validate_component(component)?;
        out.push(component);
    }
    Ok(out)
}

fn validate_component(component: &str) -> Result<(), PathError> {
    if component.len() > 200 {
        return Err(PathError::Unsafe("path component too long".into()));
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(PathError::Unsafe(
            "component may not end with a dot or space".into(),
        ));
    }
    if component.chars().any(|c| {
        matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '/' | '\\') || (c as u32) < 0x20
    }) {
        return Err(PathError::Unsafe("illegal character in name".into()));
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(PathError::Unsafe("reserved device name".into()));
    }
    Ok(())
}

/// Converts a local filesystem path into a wire path, preserving the root name
/// of the selection (file name or folder name).
pub fn wire_path_for(base: &Path, path: &Path) -> Result<String, PathError> {
    let relative = path
        .strip_prefix(base)
        .map_err(|_| PathError::Unsafe("path escapes its base".into()))?;
    let mut components = Vec::new();
    let base_name = base
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| PathError::Unsafe("selection has no usable name".into()))?;
    validate_component(base_name)?;
    components.push(base_name.to_string());
    for component in relative.components() {
        let text = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| PathError::Unsafe("file name is not valid UTF-8".into()))?;
        validate_component(text)?;
        components.push(text.to_string());
    }
    Ok(components.join("/"))
}

/// Creates a new file without ever overwriting an existing one.
/// The returned file is a zero-length reservation; callers atomically rename
/// verified data over it.
pub fn reserve_unique(dest_root: &Path, rel: &Path) -> io::Result<PathBuf> {
    let base = dest_root.join(rel);
    if let Some(parent) = base.parent() {
        std::fs::create_dir_all(parent)?;
    }
    for suffix in 0u32..=9999 {
        let candidate = if suffix == 0 {
            base.clone()
        } else {
            with_suffix(&base, suffix)
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                drop(file);
                return Ok(candidate);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not find a free destination name",
    ))
}

fn with_suffix(path: &Path, suffix: u32) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let (stem, ext) = match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    };
    parent.join(format!("{stem} ({suffix}){ext}"))
}

/// Relative wire representation of a finalized file below the destination root.
pub fn wire_path_below(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = relative
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect();
    Some(parts?.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_paths() {
        assert_eq!(
            verify_wire_path("Photos/2026/img 01.jpg").unwrap(),
            PathBuf::from("Photos").join("2026").join("img 01.jpg")
        );
    }

    #[test]
    fn rejects_traversal_and_platform_hazards() {
        for bad in [
            "../escape",
            "a/../../b",
            "/abs",
            "a\\b",
            "a//b",
            "CON",
            "dir/nul.txt",
            "trailing.",
            "trailing ",
            "bad:name",
            "bad?.txt",
            "bad\u{1}name",
        ] {
            assert!(verify_wire_path(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn reservation_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let first = reserve_unique(dir.path(), Path::new("report.txt")).unwrap();
        let second = reserve_unique(dir.path(), Path::new("report.txt")).unwrap();
        assert_eq!(first.file_name().unwrap(), "report.txt");
        assert_eq!(second.file_name().unwrap(), "report (1).txt");
        assert_eq!(second.parent().unwrap(), dir.path());
    }

    #[test]
    fn wire_path_keeps_selection_root() {
        let base = Path::new("C:/data/album");
        let file = Path::new("C:/data/album/sub/photo.jpg");
        assert_eq!(wire_path_for(base, file).unwrap(), "album/sub/photo.jpg");
    }
}
