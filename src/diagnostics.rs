//! Local diagnostics shared by the desktop and CLI.
use std::{
    fs,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;

/// Keep one JSON object per line, including when git prints a multiline error.
pub(crate) fn collection_update_failed(dir: &Path, collection: &str, error: &str) -> Result<()> {
    const MAX_FILE: u64 = 1 << 20;
    let mut error = crate::reports::redact(error);
    if error.len() > 8192 {
        error.truncate(error.floor_char_boundary(8192));
        error.push('…');
    }
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let line = serde_json::to_vec(&serde_json::json!({
        "time": crate::reports::rfc3339(millis),
        "event": "collectionUpdate",
        "outcome": "failed",
        "collection": collection,
        "error": error,
    }))?;

    fs::create_dir_all(dir)?;
    let path = dir.join("furnish.log");
    // The desktop app and CLI can append or rotate at the same time.
    let lock = crate::auth::open_private(&dir.join(".furnish-log.lock"), false)?;
    lock.lock()?;
    if let Ok(existing) = fs::metadata(&path)
        && existing.len() + line.len() as u64 + 1 > MAX_FILE
    {
        fs::rename(&path, dir.join("furnish.log.1"))?;
    }
    let mut file = crate::auth::open_private(&path, true)?;
    file.write_all(&line)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_a_full_log_keeps_the_previous_file_and_caps_multiline_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("furnish.log");
        fs::write(&path, vec![b'x'; 1 << 20]).unwrap();
        let error = format!("remote: HTTP 429\n{}", "ä".repeat(6000));
        collection_update_failed(dir.path(), "tools", &error).unwrap();
        assert_eq!(fs::metadata(dir.path().join("furnish.log.1")).unwrap().len(), 1 << 20);
        let text = fs::read_to_string(path).unwrap();
        assert_eq!(text.lines().count(), 1);
        let record: serde_json::Value = serde_json::from_str(&text).unwrap();
        let message = record["error"].as_str().unwrap();
        assert!(message.starts_with("remote: HTTP 429\n") && message.ends_with('…'));
        assert!(message.len() <= 8195);
    }
}
