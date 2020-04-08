use std::fs::{DirBuilder, DirEntry};
use std::path::PathBuf;

pub fn get_temporary_directory() -> PathBuf {
    let path = std::env::var("TARGET_DIR")
        .and_then(|value| Ok(PathBuf::from(&value)))
        .unwrap_or(std::env::temp_dir().join("rsync-http"));

    if !path.exists() {
        DirBuilder::new().create(&path);
    };

    path
}
