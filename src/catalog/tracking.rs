//! A remote ref and both of the object IDs advertised for it. Checkout always uses a verified
//! commit ID. In particular, an annotated tag can be replaced while still pointing at that commit.
use super::{git, git_bytes};
use crate::settings::{TagOrder, Tracking};
mod dates;
use anyhow::{Context, Result, ensure};
use std::{cmp::Ordering, collections::BTreeMap, fmt, path::Path, sync::LazyLock};

/// The remote responded successfully, but this selector has no target there.
#[derive(Debug)]
pub(super) struct MissingRef(String);
impl fmt::Display for MissingRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MissingRef {}

pub(super) struct Target {
    pub reference: String,
    pub object: String,
    pub commit: String,
}

fn object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Target {
    pub fn resolve(dir: Option<&Path>, remote: &str, tracking: &Tracking) -> Result<Self> {
        tracking.validate()?;
        let branch_ref;
        let selector = match tracking {
            Tracking::Default => {
                let out = git(dir, &["ls-remote", "--symref", "--end-of-options", remote, "HEAD"])?;
                let entries: Vec<_> = out.lines().filter_map(|line| line.split_once('\t')).filter(|(_, name)| *name == "HEAD").collect();
                let reference = entries
                    .iter()
                    .find_map(|(v, _)| v.strip_prefix("ref: ").filter(|r| r.starts_with("refs/heads/")))
                    .ok_or_else(|| MissingRef("the server has no default branch".into()))?;
                let commit = entries
                    .iter()
                    .find_map(|(v, _)| object_id(v).then_some(*v))
                    .ok_or_else(|| MissingRef("the default branch has no commit".into()))?;
                return Ok(Self { reference: reference.into(), object: commit.into(), commit: commit.into() });
            }
            Tracking::Branch { name } => {
                branch_ref = format!("refs/heads/{name}");
                vec!["ls-remote", "--refs", "--end-of-options", remote, &branch_ref]
            }
            Tracking::Tag { .. } => vec!["ls-remote", "--tags", "--end-of-options", remote],
        };
        let out = git_bytes(dir, &selector)?;
        // A non-UTF-8 tag is outside the regex's string domain. Don't manufacture a different name
        // by lossily replacing bytes; unrelated valid tags in the same repository remain selectable.
        let refs: BTreeMap<_, _> = out
            .split(|b| *b == b'\n')
            .filter_map(|line| std::str::from_utf8(line).ok())
            .filter_map(|line| line.split_once('\t'))
            .filter(|(id, _)| object_id(id))
            .map(|(id, name)| (name, id))
            .collect();
        let reference = match tracking {
            Tracking::Branch { name } => format!("refs/heads/{name}"),
            Tracking::Tag { pattern, order } => {
                let re = regex::Regex::new(pattern)?;
                let tags: Vec<_> = refs
                    .iter()
                    .filter_map(|(reference, object)| {
                        let name = reference.strip_prefix("refs/tags/")?;
                        (!name.ends_with("^{}") && re.is_match(name)).then_some((name, *object))
                    })
                    .collect();
                if tags.is_empty() {
                    return Err(MissingRef(format!("no remote tag matches '{pattern}'")).into());
                }
                let dates = match order {
                    TagOrder::Newest => dates::for_tags(dir, remote, &tags)?,
                    TagOrder::Version => BTreeMap::new(),
                };
                let (name, _) = tags
                    .into_iter()
                    .max_by(|(a, a_id), (b, b_id)| dates.get(*a_id).cmp(&dates.get(*b_id)).then_with(|| tag_order(a, b)))
                    .expect("at least one matching tag");
                format!("refs/tags/{name}")
            }
            Tracking::Default => unreachable!(),
        };
        let object = *refs.get(reference.as_str()).ok_or_else(|| MissingRef(format!("the server has no {reference}")))?;
        let peeled = format!("{reference}^{{}}");
        let commit = refs.get(peeled.as_str()).copied().unwrap_or(object);
        Ok(Self { reference, object: object.into(), commit: commit.into() })
    }

    pub fn short_name(&self) -> &str {
        self.reference.strip_prefix("refs/heads/").or_else(|| self.reference.strip_prefix("refs/tags/")).expect("selected a head or tag")
    }

    fn verify_fetched(&self, dir: &Path, local: &str) -> Result<()> {
        let raw = git(Some(dir), &["rev-parse", "--verify", "--end-of-options", local])?;
        ensure!(raw == self.object, "{} changed while fetching; retry the collection update", self.reference);
        let commit = git(Some(dir), &["rev-parse", "--verify", "--end-of-options", &format!("{local}^{{commit}}")])
            .with_context(|| format!("{} does not point to a commit", self.reference))?;
        ensure!(commit == self.commit, "{} resolved to an unexpected commit", self.reference);
        Ok(())
    }

    pub fn cloned(&self, dir: &Path, depth: u32) -> Result<()> {
        if let Some(branch) = self.reference.strip_prefix("refs/heads/") {
            self.verify_fetched(dir, &format!("refs/remotes/origin/{branch}"))
        } else if git(Some(dir), &["show-ref", "--verify", "--quiet", &self.reference]).is_ok() {
            self.verify_fetched(dir, &self.reference)
        } else {
            // --branch accepts a short name and a branch may have the same name as a tag.
            self.fetch(dir, depth)
        }
    }

    pub fn fetch(&self, dir: &Path, depth: u32) -> Result<()> {
        let depth_arg = format!("--depth={depth}");
        let mut args = vec!["fetch", "--quiet", "--no-tags", "--refmap=", "--recurse-submodules=no"];
        if depth > 0 {
            args.push(&depth_arg);
        } else if git(Some(dir), &["rev-parse", "--is-shallow-repository"])? == "true" {
            args.push("--unshallow");
        }
        args.extend(["--end-of-options", "origin", &self.reference]);
        git(Some(dir), &args)?;
        self.verify_fetched(dir, "FETCH_HEAD")
    }
}

/// A SemVer suffix (including one prefixed with v or release/) outranks an unversioned name.
/// Build metadata and numerically equal digit runs are disambiguated by the full tag name.
fn tag_order(a: &str, b: &str) -> Ordering {
    static SUFFIX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?:^|[^0-9.])([0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.+-]+)?)$").unwrap());
    let version = |s: &str| SUFFIX.captures(s).and_then(|m| semver::Version::parse(&m[1]).ok());
    match (version(a), version(b)) {
        (Some(a), Some(b)) => a.cmp_precedence(&b),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => natural(a.as_bytes(), b.as_bytes()),
    }
    .then_with(|| a.cmp(b))
}

fn natural(mut a: &[u8], mut b: &[u8]) -> Ordering {
    while let (Some(&x), Some(&y)) = (a.first(), b.first()) {
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let digits = |s: &[u8]| s.iter().take_while(|v| v.is_ascii_digit()).count();
            let (da, ra) = a.split_at(digits(a));
            let (db, rb) = b.split_at(digits(b));
            let leading_zeroes = |s: &[u8]| s.iter().take_while(|v| **v == b'0').count();
            let (da, db) = (&da[leading_zeroes(da)..], &db[leading_zeroes(db)..]);
            let order = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
            if order != Ordering::Equal {
                return order;
            }
            (a, b) = (ra, rb);
        } else {
            if x != y {
                return x.cmp(&y);
            }
            (a, b) = (&a[1..], &b[1..]);
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_order_is_independent_of_announcement_order() {
        let mut tags = ["misc12", "v1.10.0-rc.2", "v1.9.9", "release/v1.10.0", "v1.10.0-rc.12", "misc2"];
        tags.sort_by(|a, b| tag_order(a, b));
        assert_eq!(tags, ["misc2", "misc12", "v1.9.9", "v1.10.0-rc.2", "v1.10.0-rc.12", "release/v1.10.0"]);
    }

    #[test]
    fn remote_ref_moves_and_deletions_fail_without_touching_head() {
        let temp = tempfile::tempdir().unwrap();
        let origin = temp.path().join("origin");
        let copy = temp.path().join("checkout");
        std::fs::create_dir(&origin).unwrap();
        let publisher = |args: &[&str]| {
            let mut cmd = vec!["-c", "user.name=Test", "-c", "user.email=t@test", "-c", "commit.gpgsign=false", "-c", "tag.gpgSign=false"];
            cmd.extend_from_slice(args);
            git(Some(&origin), &cmd).unwrap()
        };
        publisher(&["init", "-q", "-b", "main"]);
        publisher(&["commit", "-qm", "first", "--allow-empty"]);
        publisher(&["tag", "-a", "v1.0.0", "-m", "first"]);
        git(None, &["clone", "-q", "--", origin.to_str().unwrap(), copy.to_str().unwrap()]).unwrap();
        let tag = Tracking::Tag { pattern: "^v".into(), order: TagOrder::Version };
        let selected = Target::resolve(Some(&copy), "origin", &tag).unwrap();
        let first = git(Some(&copy), &["rev-parse", "HEAD"]).unwrap();
        assert_ne!(selected.object, selected.commit);
        publisher(&["tag", "-fa", "v1.0.0", "-m", "replacement", "HEAD"]);
        assert!(selected.fetch(&copy, 0).unwrap_err().to_string().contains("changed while fetching"));
        assert_eq!(git(Some(&copy), &["rev-parse", "HEAD"]).unwrap(), first);
        publisher(&["tag", "-d", "v1.0.0"]);
        assert!(selected.fetch(&copy, 0).is_err());
        assert!(Target::resolve(Some(&copy), "origin", &tag).err().unwrap().is::<MissingRef>());
        publisher(&["commit", "-qm", "branch", "--allow-empty"]);
        publisher(&["branch", "v2.0.0"]);
        publisher(&["tag", "v2.0.0", &first]);
        let selected = Target::resolve(Some(&copy), "origin", &tag).unwrap();
        assert_eq!(selected.commit, first, "tag won exact namespace selection");
        selected.fetch(&copy, 0).unwrap();
        assert_eq!(git(Some(&copy), &["rev-parse", "FETCH_HEAD"]).unwrap(), first);
    }
}
