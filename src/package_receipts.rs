//! Package identity is installation state. A receipt is keyed by the collection, app and
//! resource's name path (the same stable keys used for Nix configuration declarations).
use crate::{catalog, nixos_apps};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub manager: String,
    pub name: String,
    base: String,
    feed: Option<String>,
}

fn directory(profile: &Path, app_hash: &str) -> PathBuf {
    profile.join("package-owners").join(app_hash)
}

fn path(profile: &Path, metadata: &Value) -> Option<PathBuf> {
    let key = metadata[nixos_apps::PACKAGE]["key"]
        .as_str()
        .filter(|s| nixos_apps::package_key(s))?;
    let (app, step) = key.strip_prefix("package:")?.split_once(':')?;
    Some(directory(profile, app).join(step))
}

fn read(path: &Path) -> Result<Option<Receipt>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).with_context(|| {
            format!("reading package ownership at {}", path.display())
        })?)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn lookup(profile: &Path, metadata: &Value, props: &Value) -> Result<Option<Receipt>> {
    let Some(path) = path(profile, metadata) else {
        return Ok(None);
    };
    let exact = read(&path)?;
    if exact.is_some()
        || metadata[nixos_apps::PACKAGE]["removal"] != true
        || props["_exist"] != false
    {
        return Ok(exact);
    }
    // Custom uninstall steps often have a different display name. Match only within
    // this app, and require an unambiguous package identity.
    let entries = match fs::read_dir(path.parent().unwrap()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut found = None;
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if let Some(owner) = read(&entry.path())?
            && (props["name"] == owner.base || props["name"] == owner.name)
            && props["packageManager"]
                .as_str()
                .is_none_or(|id| id == owner.manager)
        {
            ensure!(
                found.is_none(),
                "ambiguous package ownership for {}; use the install step name for removal",
                props["name"]
            );
            found = Some(owner);
        }
    }
    Ok(found)
}

impl Receipt {
    pub fn apply(&self, props: &mut Value) {
        let map = props
            .as_object_mut()
            .expect("package properties are an object");
        map.remove("namesByDistribution");
        map.remove("namesByManager");
        map.insert("packageManager".into(), self.manager.as_str().into());
        map.insert("name".into(), self.name.as_str().into());
        if let Some(feed) = &self.feed {
            map.insert("feed".into(), feed.as_str().into());
        }
    }
}

pub(crate) fn remember(props: &Value, manager: &str, name: &str) -> Result<()> {
    let Some(path) = path(&catalog::data_dir(), &props["_metadata"]) else {
        return Ok(());
    };
    let owner = Receipt {
        manager: manager.into(),
        name: name.into(),
        base: props["name"]
            .as_str()
            .context("missing package name")?
            .into(),
        feed: (manager == "0install")
            .then(|| props["feed"].as_str().unwrap_or_default().to_owned()),
    };
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)?;
    let temp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(temp.as_file(), &owner)?;
    // Desktop clients read the service's receipts when checking preset apps.
    #[cfg(unix)]
    if catalog::data_dir() == catalog::system_dir() {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    temp.persist(path)?;
    Ok(())
}

pub(crate) fn forget(profile: &Path, app: &str) -> Result<()> {
    match fs::remove_dir_all(directory(profile, &catalog::cache_key(app.as_bytes()))) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}
