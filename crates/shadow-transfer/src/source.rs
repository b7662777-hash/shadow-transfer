use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use shadow_protocol::{FileMeta, MAX_FILES_PER_TRANSFER};
use tracing::warn;

use crate::error::TransferError;
use crate::path::wire_path_for;

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub rel_path: String,
    pub size: u64,
    pub modified_ms: i64,
}

impl SourceFile {
    pub fn to_meta(&self, file_id: u32) -> FileMeta {
        FileMeta {
            file_id,
            rel_path: self.rel_path.clone(),
            size: self.size,
            modified_ms: self.modified_ms,
        }
    }
}

pub async fn collect_sources(inputs: Vec<PathBuf>) -> Result<Vec<SourceFile>, TransferError> {
    tokio::task::spawn_blocking(move || collect_sources_blocking(&inputs))
        .await
        .map_err(|e| TransferError::InvalidSource(format!("file collection panicked: {e}")))?
}

pub fn collect_sources_blocking(inputs: &[PathBuf]) -> Result<Vec<SourceFile>, TransferError> {
    if inputs.is_empty() {
        return Err(TransferError::EmptySelection);
    }
    let mut collected = Vec::new();
    for input in inputs {
        let metadata = fs::symlink_metadata(input).map_err(|e| {
            TransferError::InvalidSource(format!("cannot read {}: {e}", input.display()))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(TransferError::InvalidSource(format!(
                "symbolic links are not supported: {}",
                input.display()
            )));
        }
        if metadata.is_file() {
            add_file(&mut collected, input, input, &metadata)?;
        } else if metadata.is_dir() {
            let mut stack = vec![input.clone()];
            while let Some(dir) = stack.pop() {
                let entries = fs::read_dir(&dir).map_err(|e| {
                    TransferError::InvalidSource(format!("cannot list {}: {e}", dir.display()))
                })?;
                for entry in entries {
                    let entry = entry.map_err(|e| {
                        TransferError::InvalidSource(format!("cannot list {}: {e}", dir.display()))
                    })?;
                    let path = entry.path();
                    let meta = match entry.metadata() {
                        Ok(meta) => meta,
                        Err(e) => {
                            return Err(TransferError::InvalidSource(format!(
                                "cannot read {}: {e}",
                                path.display()
                            )))
                        }
                    };
                    if meta.file_type().is_symlink() {
                        warn!(path = %path.display(), "skipping symbolic link");
                        continue;
                    }
                    if meta.is_dir() {
                        stack.push(path);
                    } else if meta.is_file() {
                        add_file(&mut collected, input, &path, &meta)?;
                    }
                }
            }
        } else {
            return Err(TransferError::InvalidSource(format!(
                "unsupported file type: {}",
                input.display()
            )));
        }
    }

    if collected.is_empty() {
        return Err(TransferError::EmptySelection);
    }
    collected.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    let mut seen = HashSet::with_capacity(collected.len());
    for file in &collected {
        if !seen.insert(file.rel_path.clone()) {
            return Err(TransferError::InvalidSource(format!(
                "two selected items map to the same name: {}",
                file.rel_path
            )));
        }
    }
    if collected.len() > MAX_FILES_PER_TRANSFER {
        return Err(TransferError::InvalidSource(format!(
            "too many files: {}",
            collected.len()
        )));
    }
    Ok(collected)
}

fn add_file(
    collected: &mut Vec<SourceFile>,
    base: &Path,
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), TransferError> {
    if collected.len() >= MAX_FILES_PER_TRANSFER {
        return Err(TransferError::InvalidSource("too many files".into()));
    }
    let rel_path = wire_path_for(base, path)
        .map_err(|e| TransferError::InvalidSource(format!("{}: {e}", path.display())))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    collected.push(SourceFile {
        path: path.to_path_buf(),
        rel_path,
        size: metadata.len(),
        modified_ms,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn collects_nested_files_and_unicode_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("album");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("a.txt"), b"hello").unwrap();
        fs::write(root.join("nested").join("图片.jpg"), b"jpeg").unwrap();
        let files = collect_sources_blocking(&[root]).unwrap();
        let names: Vec<_> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert_eq!(names, vec!["album/a.txt", "album/nested/图片.jpg"]);
    }

    #[test]
    fn rejects_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        fs::write(&target, b"x").unwrap();
        let link = dir.path().join("link.txt");
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_file(&target, &link).is_ok();
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&target, &link).is_ok();
        if created {
            assert!(collect_sources_blocking(&[link]).is_err());
        }
    }

    #[test]
    fn rejects_colliding_names() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let mut file = fs::File::create(a.join("same.txt")).unwrap();
        file.write_all(b"1").unwrap();
        let mut file = fs::File::create(b.join("same.txt")).unwrap();
        file.write_all(b"2").unwrap();
        // Selecting the two directories creates distinct roots, no collision.
        assert!(collect_sources_blocking(&[a.clone(), b.clone()]).is_ok());
        // Selecting files with the same destination name collides.
        assert!(collect_sources_blocking(&[a.join("same.txt"), b.join("same.txt")]).is_err());
    }
}
