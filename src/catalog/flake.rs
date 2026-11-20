//! A Nix profile can install a flake from the verified collection checkout. Nix sees
//! just the selected folder. Furnish handles any blobs absent from a sparse checkout.
use super::{Collection, plain_path};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

const ROOT: &str = "furnishFlakeRoot";

pub(crate) fn relative(folder: &str) -> Result<String> {
    let folder = folder.strip_prefix("./").unwrap_or(folder);
    let path = Path::new(folder);
    ensure!(
        !folder.is_empty()
            && !folder.contains('\\')
            && !folder.chars().any(char::is_control)
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(n) if !n.eq_ignore_ascii_case(".git"))),
        "flake must be a collection-relative folder"
    );
    Ok(path
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/"))
}

pub(super) fn attach(root: &Path, resource: &mut Value) -> Result<()> {
    let Some(folder) = resource["properties"].get("flake") else {
        return Ok(());
    };
    let folder = relative(folder.as_str().context("flake must be a collection-relative folder")?)?;
    let root = plain_path(root.canonicalize()?);
    verify(&root, &folder)?;
    resource["properties"]["flake"] = folder.into();
    resource["metadata"][ROOT] = root.to_string_lossy().into_owned().into();
    Ok(())
}

/// Check even an existing prefix of an absent path: the version cache is populated after the
/// document is first read. Never resolve a flake through a link outside its repository.
fn verify(root: &Path, folder: &str) -> Result<()> {
    let mut prefix = root.to_owned();
    for part in Path::new(folder)
        .components()
        .chain([Component::Normal("flake.nix".as_ref())])
    {
        prefix.push(part);
        match fs::symlink_metadata(&prefix) {
            Ok(_) => ensure!(
                plain_path(prefix.canonicalize()?) == prefix,
                "flake path traverses a link: {}",
                prefix.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub(crate) fn reference(props: &Value) -> Result<String> {
    let root = Path::new(
        props["_metadata"][ROOT]
            .as_str()
            .context("flake needs a collection checkout")?,
    );
    let folder = relative(props["flake"].as_str().context("invalid flake folder")?)?;
    verify(root, &folder)?;
    let mut path = root.join(&folder);
    if props["_exist"] != false {
        ensure!(
            path.join("flake.nix").is_file(),
            "{folder}/flake.nix is missing from the collection"
        );
        if root.join(".git/info/sparse-checkout").is_file() {
            let commit = super::git(Some(root), &["rev-parse", "--verify", "HEAD"])?;
            let files = super::directory_tree(root, &commit, &folder)?;
            if files.iter().any(|(file, _)| !path.join(file).is_file()) {
                // A regular user previewing a system collection cannot change its sparse patterns.
                // Publish a complete checkout of this subtree in our own data directory instead.
                let cache = super::data_dir().join("cache/collection-flakes");
                fs::create_dir_all(&cache)?;
                let tree = super::git(
                    Some(root),
                    &[
                        "rev-parse",
                        "--verify",
                        "--end-of-options",
                        &format!("{commit}:{folder}"),
                    ],
                )?;
                let snapshot = cache.join(super::cache_key(&serde_json::to_vec(&(&tree, &folder))?));
                if !snapshot.is_dir() {
                    let tmp = tempfile::tempdir_in(&cache)?;
                    let collection = Collection {
                        name: String::new(),
                        dir: root.into(),
                        managed: false,
                    };
                    collection.write_revision_folder(&commit, &folder, tmp.path())?;
                    if let Err(e) = fs::rename(tmp.path(), &snapshot)
                        && !snapshot.is_dir()
                    {
                        return Err(e.into());
                    }
                }
                path = snapshot.join(&folder);
            }
        }
    }
    let path = path.to_str().context("non-UTF-8 flake path")?.replace(std::path::MAIN_SEPARATOR, "/");
    let encoded = path
        .split('/')
        .map(crate::endpoint::percent_encode)
        .collect::<Vec<_>>()
        .join("/");
    Ok(format!("path:{encoded}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flake_reference_is_scoped_to_its_collection_and_escapes_uri_characters() {
        let temp = tempfile::tempdir().unwrap();
        let relative_path = "files/tray #1";
        let folder = temp.path().join(relative_path);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("flake.nix"), "{}").unwrap();
        let mut r = json!({"type":"Furnish/Package", "properties":{"flake":"./files/tray #1/"},
            "metadata":{(ROOT):"/not-the-collection"}});
        attach(temp.path(), &mut r).unwrap();
        let mut input = r["properties"].clone();
        input["_metadata"] = r["metadata"].clone();
        assert_eq!(input["flake"], relative_path);
        assert!(reference(&input).unwrap().ends_with("/files/tray%20%231"));
        assert!(reference(&json!({"flake":"files/tray"})).is_err());
        for invalid in ["/absolute", "a/../other", "../other", ".git/objects", "./", "tab\tname"] {
            assert!(relative(invalid).is_err(), "{invalid}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn flake_and_definition_cannot_link_outside_the_collection() {
        use std::os::unix::fs::symlink;
        let repo = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(elsewhere.path().join("flake.nix"), "{}").unwrap();
        symlink(elsewhere.path(), repo.path().join("link")).unwrap();
        assert!(verify(repo.path(), "link").is_err());
        fs::create_dir(repo.path().join("folder")).unwrap();
        symlink(elsewhere.path().join("flake.nix"), repo.path().join("folder/flake.nix")).unwrap();
        assert!(verify(repo.path(), "folder").is_err());
    }
}
