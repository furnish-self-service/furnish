//! Remote ref advertisements contain object IDs but no dates. Fetch previously unseen tag and
//! commit objects at depth one in a scratch repository. Dumb HTTP requires a full fetch of the
//! missing tags' reachable history. Both paths reuse immutable dates without touching collection refs.
use crate::catalog::{cache_key, data_dir, git, git_bytes_input, origin_url};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

type Dates = BTreeMap<String, i64>;

pub(super) fn for_tags(dir: Option<&Path>, remote: &str, tags: &[(&str, &str)]) -> Result<Dates> {
    let url = match dir {
        Some(dir) => origin_url(dir)?,
        None => remote.to_string(),
    };
    cached_dates(&data_dir().join("tag-dates"), dir, &url, tags)
}

fn cached_dates(folder: &Path, dir: Option<&Path>, url: &str, tags: &[(&str, &str)]) -> Result<Dates> {
    let local = local_repository(dir, url);
    // Git normalizes a local path when it clones; use that same identity before the first checkout.
    let identity = local.as_deref().and_then(Path::to_str).unwrap_or(url);
    let file = folder.join(format!("{}.json", cache_key(identity.as_bytes())));
    let mut dates: Dates = fs::read(&file).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    let missing: BTreeMap<_, _> = tags.iter().copied().filter(|(_, id)| !dates.contains_key(*id)).map(|(name, id)| (id, name)).collect();
    if missing.is_empty() {
        return Ok(dates);
    }

    let mut found = if let Some(local) = local {
        // Local repositories already have these objects. Only use IDs actually advertised.
        read_dates(&local, "refs/tags")?
    } else {
        fetch_dates(url, &missing)?
    };
    for (id, name) in missing {
        let date =
            found.remove(id).with_context(|| format!("tag '{name}' changed or has no Git creation date; check its target and retry"))?;
        dates.insert(id.to_owned(), date);
    }
    // Store only immutable object IDs and dates. Atomic replacement avoids partial cache reads.
    // Concurrent checks can redownload a missing date; the selected object remains the same.
    if fs::create_dir_all(folder).is_ok()
        && let Ok(tmp) = tempfile::NamedTempFile::new_in(folder)
        && serde_json::to_writer(tmp.as_file(), &dates).is_ok()
    {
        let _ = tmp.persist(&file);
    }
    Ok(dates)
}

fn local_repository(dir: Option<&Path>, url: &str) -> Option<PathBuf> {
    // Nontrivial file URLs (escapes, network hosts) are handled by Git's normal transport.
    let path = match url.strip_prefix("file://") {
        Some(path) if Path::new(path).is_absolute() && !path.contains('%') => PathBuf::from(path),
        Some(_) => return None,
        None if !Path::new(url).is_absolute() && url.contains(':') => return None,
        None => dir.unwrap_or_else(|| Path::new(".")).join(url),
    };
    path.canonicalize().ok().filter(|path| path.is_dir())
}

fn fetch_dates(url: &str, missing: &BTreeMap<&str, &str>) -> Result<Dates> {
    let mut refspecs = String::new();
    for (index, name) in missing.values().enumerate() {
        // Internal destinations are unrelated to collection refs. --stdin works for large tag sets.
        refspecs.push_str(&format!("refs/tags/{name}:refs/furnish/{index}\n"));
    }
    let fetch = |shallow| -> Result<Dates> {
        // A failed filtered fetch can leave origin marked as a promisor remote. A fresh scratch
        // repo lets dumb HTTP fetch normally without recursive requests for missing objects.
        let probe = tempfile::tempdir()?;
        let repo = probe.path();
        let sha256 = missing.keys().next().is_some_and(|id| id.len() == 64);
        git(Some(repo), &["init", "--bare", "--quiet", if sha256 { "--object-format=sha256" } else { "--object-format=sha1" }])?;
        // git_bytes_input supplies the credential environment for this exact remote origin.
        git(Some(repo), &["config", "remote.origin.url", url])?;
        let mut args = vec!["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--recurse-submodules=no"];
        if shallow {
            args.extend(["--depth=1", "--filter=tree:0"]);
        }
        args.extend(["--stdin", "--", "origin"]);
        git_bytes_input(Some(repo), &args, Some(refspecs.as_bytes()))?;
        read_dates(repo, "refs/furnish")
    };
    match fetch(true) {
        Err(e) if e.to_string().contains("dumb http transport does not support shallow capabilities") => fetch(false),
        result => result,
    }
}

fn read_dates(repo: &Path, prefix: &str) -> Result<Dates> {
    // creatordate is Git's tagger date for annotated tags and committer date for lightweight tags.
    // Avoid dereferencing the tag in this query; the final target is peeled and verified separately.
    let text = git(Some(repo), &["for-each-ref", "--format=%(objectname)%09%(creatordate:unix)", prefix])?;
    let mut dates = BTreeMap::new();
    for line in text.lines() {
        if let Some((id, date)) = line.split_once('\t')
            && !date.is_empty()
        {
            ensure!(super::object_id(id), "invalid Git object ID while reading tag dates");
            dates.insert(id.to_owned(), date.parse::<i64>().context("invalid Git creation date")?);
        }
    }
    Ok(dates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_from_remote_tags_off_the_default_branch_are_reused_by_object_id() {
        let temp = tempfile::tempdir().unwrap();
        let origin = temp.path().join("date origin");
        fs::create_dir(&origin).unwrap();
        let run = |args: &[&str], date: &str| {
            let out = crate::catalog::git_command()
                .current_dir(&origin)
                .env("GIT_COMMITTER_DATE", date)
                .args(["-c", "user.name=Tester", "-c", "user.email=t@test", "-c", "commit.gpgSign=false", "-c", "tag.gpgSign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        let old = "2000-01-01T00:00:00Z";
        run(&["init", "-q", "-b", "main"], old);
        run(&["config", "uploadpack.allowFilter", "true"], old);
        run(&["commit", "--allow-empty", "-qm", "current"], old);
        let commit = run(&["rev-parse", "HEAD"], old);
        run(&["tag", "v2.0.0"], old);
        run(&["checkout", "--orphan", "maintenance"], old);
        run(&["commit", "--allow-empty", "-qm", "backport"], old);
        run(&["tag", "-a", "v1.0.1", "-m", "late backport"], "2000-01-02T00:00:00Z");
        let annotated = run(&["rev-parse", "refs/tags/v1.0.1"], old);
        run(&["checkout", "main"], old);
        let candidates = [("v2.0.0", commit.as_str()), ("v1.0.1", annotated.as_str())];
        // Escaped file URL goes through the real Git transport and bare metadata probe.
        let path = origin.to_string_lossy().replace('\\', "/").replace(' ', "%20");
        let url = format!("file:///{}", path.trim_start_matches('/'));
        let cache = temp.path().join("dates");
        let dates = cached_dates(&cache, None, &url, &candidates).unwrap();
        assert_eq!(dates[&commit], 946684800);
        assert_eq!(dates[&annotated], 946771200);
        // Remote date lookups would now fail. The immutable objects are already in the cache.
        fs::rename(&origin, temp.path().join("offline")).unwrap();
        assert_eq!(cached_dates(&cache, None, &url, &candidates).unwrap(), dates);
    }
}
