use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Write in the destination directory, sync, then atomically replace the old file.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or("file has no parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        ".sim-rns-write-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e: std::io::Error| format!("failed to save {}: {e}", path.display()))
}

/// Advisory lock shared by every project reader/writer in this application.
/// Closing the file releases the lock, including after a crash.
pub(crate) fn project_lock(root: &Path) -> Result<File, String> {
    let dir = root.join(".sim-rns");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("project.lock"))
        .map_err(|e| e.to_string())?;
    file.lock().map_err(|e| e.to_string())?;
    Ok(file)
}
