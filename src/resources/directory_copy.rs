//! Reconcile a collection folder. The ownership record lives outside the destination so
//! extra local files survive both a collection update and an uninstall.
use super::{DirectoryProps, Plan, common, exist, note, permissions};
use crate::catalog;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Tree {
    files: BTreeSet<PathBuf>,
    dirs: BTreeSet<PathBuf>,
}

fn relative(path: &Path) -> Result<()> {
    ensure!(
        path.components()
            .all(|c| matches!(c, Component::Normal(n) if !n.eq_ignore_ascii_case(".git"))),
        "unsafe name in directory copy: {}",
        path.display()
    );
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    File,
    Dir,
    Missing,
}

fn kind(path: &Path) -> Result<Kind> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => return Ok(Kind::Missing),
        Err(e) => return Err(e).with_context(|| format!("checking {}", path.display())),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            meta.file_attributes() & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT == 0,
            "cannot copy through a reparse point: {}",
            path.display()
        );
    }
    if meta.is_dir() {
        Ok(Kind::Dir)
    } else if meta.is_file() {
        Ok(Kind::File)
    } else {
        bail!("expected a regular file or directory at {}", path.display())
    }
}

/// Check every component before using it, including when looking up an obsolete managed file.
/// A user may have replaced a previously managed directory with a link.
fn inspect(root: &Path, rel: &Path) -> Result<Kind> {
    relative(rel)?;
    let mut path = root.to_path_buf();
    let mut found = kind(root)?;
    for part in rel.components() {
        if found != Kind::Dir {
            return Ok(Kind::Missing);
        }
        path.push(part);
        found = kind(&path)?;
    }
    Ok(found)
}

fn physical(path: &Path) -> Result<PathBuf> {
    let existing = path.ancestors().find(|p| p.exists()).context("no existing ancestor")?;
    Ok(existing.canonicalize()?.join(path.strip_prefix(existing)?))
}

struct Source {
    path: PathBuf,
    repo: Option<PathBuf>,
    tree: Tree,
    // In a sparse checkout some tracked files have no working-tree copy.
    sparse: BTreeSet<PathBuf>,
}

impl Source {
    fn load(path: &Path, root: Option<&Path>) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "directory source must be a resolved collection path"
        );
        let mut source = Self {
            path: path.into(),
            repo: root.map(Path::to_owned),
            tree: Tree::default(),
            sparse: BTreeSet::new(),
        };
        let mut present = kind(path)?;
        if let Some(repo) = root {
            let folder = path
                .strip_prefix(repo)
                .context("directory source is outside the collection")?;
            ensure!(
                !folder.as_os_str().is_empty(),
                "source must name a folder within the collection"
            );
            present = inspect(repo, folder)?;
            if repo.join(".git/info/sparse-checkout").is_file() {
                let folder = folder
                    .to_str()
                    .context("non-UTF-8 source path")?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                for name in catalog::directory_tree(repo, "HEAD", &folder)? {
                    let name = PathBuf::from(name);
                    if inspect(path, &name)? == Kind::Missing {
                        source.sparse.insert(name.clone());
                        source.tree.files.insert(name.clone());
                        source.tree.dirs.extend(name.ancestors().skip(1).map(Path::to_owned));
                    }
                }
            }
        }
        if present == Kind::Dir {
            source.walk(Path::new(""))?;
        } else {
            ensure!(
                present == Kind::Missing && !source.sparse.is_empty(),
                "source {} is not a folder",
                path.display()
            );
        }
        Ok(source)
    }

    fn walk(&mut self, dir: &Path) -> Result<()> {
        self.tree.dirs.insert(dir.into());
        for entry in fs::read_dir(self.path.join(dir))? {
            let entry = entry?;
            let name = dir.join(entry.file_name());
            relative(&name)?;
            match kind(&entry.path())? {
                Kind::Dir => self.walk(&name)?,
                Kind::File => {
                    self.tree.files.insert(name);
                }
                Kind::Missing => bail!("{} disappeared while reading the source", entry.path().display()),
            }
        }
        Ok(())
    }

    fn bytes(&self, file: &Path) -> Result<Vec<u8>> {
        let path = self.path.join(file);
        if self.sparse.contains(file) {
            let root = self.repo.as_deref().context("missing collection root")?;
            let name = path
                .strip_prefix(root)?
                .to_str()
                .context("non-UTF-8 source file")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            return catalog::git_bytes(
                Some(root),
                &["cat-file", "--filters", "--end-of-options", &format!("HEAD:{name}")],
            );
        }
        let (root, rel) = match &self.repo {
            Some(repo) => (repo.as_path(), path.strip_prefix(repo)?),
            None => (self.path.as_path(), file),
        };
        ensure!(
            inspect(root, rel)? == Kind::File,
            "source file {} changed type",
            path.display()
        );
        fs::read(&path).with_context(|| format!("reading {}", path.display()))
    }
}

#[derive(Default, PartialEq, Serialize, Deserialize)]
struct Record {
    owned: Tree,
    // An interrupted install may have published only some of its new files. Recovery
    // may claim a pending name only when its bytes match the planned copy.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pending: BTreeMap<PathBuf, String>,
}

impl Record {
    fn owned_at(&self, path: &Path) -> Result<Tree> {
        let mut owned = self.owned.clone();
        for (name, digest) in &self.pending {
            if inspect(path, name)? == Kind::File && catalog::cache_key(&fs::read(path.join(name))?) == *digest {
                owned.files.insert(name.clone());
            }
        }
        Ok(owned)
    }
}

fn read(path: &Path) -> Result<Option<Record>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let record: Record = serde_json::from_slice(&bytes).context("reading directory ownership record")?;
    for name in record
        .owned
        .files
        .iter()
        .chain(&record.owned.dirs)
        .chain(record.pending.keys())
    {
        relative(name)?;
    }
    ensure!(
        !record.owned.files.contains(Path::new("")) && !record.pending.contains_key(Path::new("")),
        "invalid directory ownership record"
    );
    Ok(Some(record))
}

fn save(path: &Path, record: &Record, shared: bool) -> Result<()> {
    let tmp = tempfile::NamedTempFile::new_in(path.parent().context("missing record directory")?)?;
    serde_json::to_writer(tmp.as_file(), record)?;
    #[cfg(unix)]
    if shared {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file().set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    #[cfg(not(unix))]
    let _ = shared;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    Ok(())
}

#[derive(Default)]
struct Changes {
    write: Vec<PathBuf>,
    added: usize,
    remove_files: Vec<PathBuf>,
    remove_dirs: Vec<PathBuf>,
    create_dirs: Vec<PathBuf>,
}

impl Changes {
    fn between(path: &Path, old: &Tree, wanted: &Tree, source: Option<&Source>) -> Result<Self> {
        let mut out = Self::default();
        for rel in old.files.difference(&wanted.files) {
            if inspect(path, rel)? == Kind::File {
                out.remove_files.push(rel.clone());
            }
        }
        for rel in old.dirs.iter().rev().filter(|dir| !wanted.dirs.contains(*dir)) {
            if inspect(path, rel)? == Kind::Dir {
                out.remove_dirs.push(rel.clone());
            }
        }
        for rel in &wanted.dirs {
            match inspect(path, rel)? {
                Kind::Dir => {}
                Kind::Missing => out.create_dirs.push(rel.clone()),
                Kind::File if old.files.contains(rel) => out.create_dirs.push(rel.clone()),
                _ => bail!("{} is a file where the source needs a folder", path.join(rel).display()),
            }
        }
        if let Some(source) = source {
            for file in &wanted.files {
                match inspect(path, file)? {
                    Kind::File if fs::read(path.join(file))? == source.bytes(file)? => continue,
                    Kind::File if !old.files.contains(file) => bail!(
                        "{} already exists with different contents; move the local file before copying",
                        path.join(file).display()
                    ),
                    Kind::File => {}
                    Kind::Missing => out.added += 1,
                    Kind::Dir if old.dirs.contains(file) => out.added += 1,
                    Kind::Dir => bail!(
                        "{} is a local folder where the source needs a file",
                        path.join(file).display()
                    ),
                }
                out.write.push(file.clone());
            }
        }
        Ok(out)
    }
}

pub(super) fn plan(props: &Value, p: &DirectoryProps, source: &Path) -> Result<Plan> {
    let meta = &props["_metadata"];
    let shared =
        meta["furnishDirectorySystem"] == true || std::env::var_os("FURNISH_SERVICE").is_some_and(|v| v == "1");
    let data = if shared {
        catalog::system_dir()
    } else {
        catalog::data_dir()
    };
    let owner = meta["furnishDirectoryOwner"].as_str().unwrap_or_default();
    let path: PathBuf = common::expand(&p.path)?.components().collect();
    let record = data
        .join("directory-copies")
        .join(catalog::cache_key(&serde_json::to_vec(&(owner, &path))?));
    prepare(props, p, source, &path, &record, shared)
}

fn prepare(props: &Value, p: &DirectoryProps, folder: &Path, path: &Path, record: &Path, shared: bool) -> Result<Plan> {
    let installing = exist(p.exist);
    let previous = read(record)?;
    let old = previous
        .as_ref()
        .map(|r| r.owned_at(path))
        .transpose()?
        .unwrap_or_default();
    let mut plan = Plan::new(props);
    // A locally populated directory left after uninstall isn't evidence of an installed copy.
    plan.observe("_exist", Some(previous.is_some().into()), Some(installing.into()));
    if !installing && previous.is_none() {
        return Ok(plan);
    }
    let source = if installing {
        let source = Source::load(
            folder,
            props["_metadata"]["furnishDirectoryRoot"].as_str().map(Path::new),
        )?;
        let physical_source = physical(folder)?;
        let physical_dest = physical(path)?;
        ensure!(
            !physical_dest.starts_with(&physical_source) && !physical_source.starts_with(&physical_dest),
            "directory source and destination overlap"
        );
        Some(source)
    } else {
        None
    };
    let empty = Tree::default();
    let wanted = source.as_ref().map(|s| &s.tree).unwrap_or(&empty);
    let changes = Changes::between(path, &old, wanted, source.as_ref())?;
    let next = Tree {
        // An identical local file that Furnish never wrote still belongs to its creator.
        files: old
            .files
            .intersection(&wanted.files)
            .cloned()
            .chain(changes.write.iter().cloned())
            .collect(),
        dirs: old
            .dirs
            .intersection(&wanted.dirs)
            .cloned()
            .chain(changes.create_dirs.iter().cloned())
            .collect(),
    };
    let mode = p.mode.as_deref().map(common::parse_mode).transpose()?;
    let private = p.private.unwrap_or(false);
    let mut notes = if installing && kind(path)? == Kind::Dir {
        permissions(&mut plan, path, &p.path, mode, private, true)?
    } else {
        vec![]
    };
    if previous
        .as_ref()
        .is_some_and(|r| r.pending.is_empty() && r.owned == next)
        && installing
        && changes.write.is_empty()
        && changes.create_dirs.is_empty()
        && notes.is_empty()
    {
        return Ok(plan);
    }
    notes.push(note(
        "dir.sync",
        json!({"path":p.path, "filesAdded":changes.added,
        "filesChanged":changes.write.len()-changes.added, "filesRemoved":changes.remove_files.len()}),
    ));
    let record = record.to_owned();
    let path = path.to_owned();
    plan.change(notes, move || {
        fs::create_dir_all(record.parent().context("missing record parent")?)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(record.with_file_name("copies.lock"))?;
        lock.lock()?;
        ensure!(read(&record)? == previous, "directory copy changed concurrently; retry");
        // Keep a union until every change succeeds. A kill or failed write cannot lose the
        // names needed to finish an update or uninstall the partially copied tree.
        let mut intent = Record {
            owned: Tree {
                files: old.files.clone(),
                dirs: old.dirs.union(&next.dirs).cloned().collect(),
            },
            ..Record::default()
        };
        if let Some(source) = &source {
            for rel in next.files.difference(&old.files) {
                intent
                    .pending
                    .insert(rel.clone(), catalog::cache_key(&source.bytes(rel)?));
            }
        }
        save(&record, &intent, shared)?;
        for rel in changes.remove_files {
            if inspect(&path, &rel)? == Kind::File {
                fs::remove_file(path.join(rel))?;
            }
        }
        for rel in changes.remove_dirs {
            if inspect(&path, &rel)? == Kind::Dir {
                match fs::remove_dir(path.join(&rel)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        if let Some(source) = source {
            for rel in changes.create_dirs {
                if inspect(&path, &rel)? != Kind::Dir {
                    fs::create_dir_all(path.join(rel))?;
                }
            }
            if let Some(mode) = mode {
                common::set_mode(&path, mode)?;
            }
            if private {
                common::set_private(&path, true)?;
            }
            for rel in changes.write {
                ensure!(
                    inspect(&path, &rel)? != Kind::Dir,
                    "cannot replace a nonempty local folder: {}",
                    path.join(&rel).display()
                );
                let target = path.join(&rel);
                let bytes = source.bytes(&rel)?;
                if let Some(expected) = intent.pending.get(&rel) {
                    ensure!(
                        catalog::cache_key(&bytes) == *expected,
                        "source changed during copy; retry"
                    );
                    // Never replace a local file created between the preview and publication.
                    let builder = tempfile::Builder::new();
                    #[cfg(unix)]
                    let builder = {
                        use std::os::unix::fs::PermissionsExt;
                        let mut builder = builder;
                        builder.permissions(fs::Permissions::from_mode(0o666));
                        builder
                    };
                    let mut tmp = builder.tempfile_in(target.parent().context("no destination parent")?)?;
                    std::io::Write::write_all(tmp.as_file_mut(), &bytes)?;
                    tmp.as_file().sync_all()?;
                    tmp.persist_noclobber(&target)
                        .with_context(|| format!("creating {}", target.display()))?;
                } else {
                    let file = common::Managed::open(target.to_str().context("non-UTF-8 destination")?)?;
                    common::write(&file, &bytes, &common::WriteOptions::default())?;
                }
            }
            save(
                &record,
                &Record {
                    owned: next,
                    pending: BTreeMap::new(),
                },
                shared,
            )?;
        } else {
            fs::remove_file(record)?;
        }
        Ok(())
    });
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_plan(folder: &Path, path: &Path, record: &Path, installing: bool) -> Result<Plan> {
        let props = json!({"path":path, "source":folder, "_exist":installing});
        prepare(
            &props,
            &serde_json::from_value(props.clone())?,
            folder,
            path,
            record,
            false,
        )
    }

    #[test]
    fn matching_local_files_and_preexisting_folders_stay_on_uninstall() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("source");
        let path = temp.path().join("target");
        let record = temp.path().join("data/record");
        for dir in [&folder, &path] {
            fs::create_dir_all(dir.join("keep")).unwrap();
            fs::write(dir.join("keep/already-matches"), "mine").unwrap();
        }
        fs::write(folder.join("copied"), "collection").unwrap();
        test_plan(&folder, &path, &record, true).unwrap().apply.unwrap()().unwrap();
        fs::remove_dir_all(folder.join("keep")).unwrap();
        assert!(test_plan(&folder, &path, &record, true).unwrap().apply.is_none());
        test_plan(&folder, &path, &record, false).unwrap().apply.unwrap()().unwrap();
        assert!(path.join("keep/already-matches").is_file());
        assert!(!path.join("copied").exists());
    }

    #[test]
    fn newly_managed_names_cannot_clobber_local_files_even_during_a_partial_install() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("source");
        let path = temp.path().join("target");
        let record = temp.path().join("data/record");
        fs::create_dir(&folder).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(folder.join("a"), "managed").unwrap();
        fs::write(folder.join("b"), "from the collection").unwrap();
        let plan = test_plan(&folder, &path, &record, true).unwrap();
        // The user saves b after Furnish prepares the copy, before a and b are published.
        fs::write(path.join("b"), "mine").unwrap();
        assert!(plan.apply.unwrap()().is_err());
        assert!(test_plan(&folder, &path, &record, true).is_err());
        test_plan(&folder, &path, &record, false).unwrap().apply.unwrap()().unwrap();
        assert!(!path.join("a").exists());
        assert_eq!(fs::read(path.join("b")).unwrap(), b"mine");
    }

    #[cfg(unix)]
    #[test]
    fn source_links_and_replaced_destination_directories_never_reach_their_targets() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("source");
        let path = temp.path().join("target");
        let record = temp.path().join("data/record");
        let outside = temp.path().join("local");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("page"), "local").unwrap();
        fs::create_dir_all(folder.join("sub")).unwrap();
        fs::write(folder.join("sub/page"), "managed").unwrap();
        symlink(&outside, folder.join("link")).unwrap();
        assert!(test_plan(&folder, &path, &record, true).is_err());
        fs::remove_file(folder.join("link")).unwrap();
        test_plan(&folder, &path, &record, true).unwrap().apply.unwrap()().unwrap();
        fs::remove_dir_all(path.join("sub")).unwrap();
        symlink(&outside, path.join("sub")).unwrap();
        assert!(test_plan(&folder, &path, &record, false).is_err());
        assert!(test_plan(&folder, &path, &record, true).is_err());
        assert_eq!(fs::read(outside.join("page")).unwrap(), b"local");
    }

    #[test]
    fn replaced_file_and_directory_can_swap_types_without_deleting_local_children() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("source");
        let path = temp.path().join("target");
        let record = temp.path().join("data/record");
        fs::create_dir(&folder).unwrap();
        let item = folder.join("item");
        fs::write(&item, "").unwrap();
        let apply = || test_plan(&folder, &path, &record, true)?.apply.unwrap()();
        apply().unwrap();
        fs::remove_file(&item).unwrap();
        fs::create_dir(&item).unwrap();
        fs::write(item.join("child"), "child").unwrap();
        apply().unwrap();
        fs::write(path.join("item/personal"), "mine").unwrap();
        fs::remove_dir_all(&item).unwrap();
        fs::write(item, "now a file").unwrap();
        assert!(apply().is_err());
        assert_eq!(fs::read(path.join("item/personal")).unwrap(), b"mine");
        fs::remove_file(path.join("item/personal")).unwrap();
        apply().unwrap();
        assert_eq!(fs::read(path.join("item")).unwrap(), b"now a file");
    }
}
