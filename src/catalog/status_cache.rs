//! Last successful desktop checks, stored per app in the user's data directory.
//! This is display state. The engine always inspects the machine for a live check.
use super::{App, cache_key, data_dir};
use crate::engine::Check;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn collection_dir(profile: &Path, collection: &str) -> PathBuf {
    profile
        .join("app-status-v1")
        .join(cache_key(collection.as_bytes()))
}

fn file(profile: &Path, collection: &str, id: &str) -> PathBuf {
    collection_dir(profile, collection).join(cache_key(id.as_bytes()))
}

pub(super) fn forget(collection: &str) {
    let _ = fs::remove_dir_all(collection_dir(&data_dir(), collection));
}

fn write(path: &Path, revision: Option<&str>, check: &Check) -> anyhow::Result<()> {
    let checked_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let mut check = serde_json::to_value(check)?;
    // A running process can guard an operation only after a live check.
    check.as_object_mut().unwrap().remove("runningApps");
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)?;
    let temp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(
        temp.as_file(),
        &json!({ "revision": revision, "checkedAt": checked_at, "check": check }),
    )?;
    temp.persist(path)?;
    Ok(())
}

impl App {
    pub fn cached_check(&self) -> Option<Value> {
        let bytes = fs::read(file(&data_dir(), &self.collection, &self.id)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn remember_check(&self, revision: Option<&str>, check: &Check) {
        // A read-only or full data directory must not turn a successful status check into an error.
        let _ = write(
            &file(&data_dir(), &self.collection, &self.id),
            revision,
            check,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Status;

    #[test]
    fn successful_checks_survive_reopening_and_replace_only_their_own_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(dir.path(), "tools", "editor");
        let mut check = Check {
            status: Status::Installed,
            running_apps: vec!["editor".into()],
            reason: None,
            removable: true,
            required: None,
            external: None,
        };
        write(&path, Some("rev"), &check).unwrap();
        let read = || serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(read()["check"]["status"], "installed");
        assert!(read()["check"].get("runningApps").is_none());
        assert!(read()["checkedAt"].as_u64().unwrap() > 0);
        assert!(!file(dir.path(), "other", "editor").exists());
        assert!(!file(&dir.path().join("other-profile"), "tools", "editor").exists());
        check.status = Status::NotInstalled;
        write(&path, None, &check).unwrap();
        assert_eq!(read()["check"]["status"], "notInstalled");
    }
}
