use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    Some(value.trim().into())
}

fn watch_revision(repo: &Path) {
    // rev-parse resolves paths in linked worktrees and packed repositories.
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(repo, &["rev-parse", "--git-path", name]) {
            let path = repo.join(path);
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    if let Some(path) = git(repo, &["symbolic-ref", "-q", "HEAD"])
        .and_then(|branch| git(repo, &["rev-parse", "--git-path", &branch]))
    {
        // A packed-only branch may acquire a loose ref on the next commit.
        if let Some(existing) = repo.join(path).ancestors().find(|p| p.exists()) {
            println!("cargo:rerun-if-changed={}", existing.display());
        }
    }
}

fn desktop_build() {
    for key in [
        "FURNISH_VERSION",
        "FURNISH_BUILD_REVISION",
        "SOURCE_DATE_EPOCH",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let repo = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let checkout = repo.join(".git").exists();
    let mut revision = env::var("FURNISH_BUILD_REVISION").ok();
    let mut epoch = env::var("SOURCE_DATE_EPOCH").ok();
    if checkout && (revision.is_none() || epoch.is_none()) {
        watch_revision(&repo);
        if revision.is_none() {
            revision = git(&repo, &["rev-parse", "HEAD"]);
        }
        if epoch.is_none() {
            epoch = git(
                &repo,
                &["show", "-s", "--no-show-signature", "--format=%ct", "HEAD"],
            );
        }
    }
    let revision = revision.filter(|r| !r.is_empty());
    if let Some(rev) = &revision {
        assert!(
            matches!(rev.len(), 40 | 64) && rev.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid FURNISH_BUILD_REVISION"
        );
    }
    let source_date_epoch = epoch
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u64>().expect("invalid SOURCE_DATE_EPOCH"))
        // Reproducible build environments use 0 or 1 when no source timestamp exists.
        .filter(|&t| t > 1);
    // A source checkout may have local changes even if HEAD has a release tag.
    let version = env::var("FURNISH_VERSION")
        .map(|v| v.replace("+alpha.", "-alpha."))
        .unwrap_or_else(|_| {
            let base = env!("CARGO_PKG_VERSION");
            match &revision {
                Some(rev) => format!("{base}-alpha+git.{}", &rev[..12]),
                None => format!("{base}-alpha"),
            }
        });
    semver::Version::parse(&version).expect("invalid desktop version");
    let json = serde_json::json!({
        "version": version, "revision": revision, "target": env::var("TARGET").unwrap(), "sourceDateEpoch": source_date_epoch,
    });
    println!("cargo:rustc-env=FURNISH_DESKTOP_BUILD_JSON={json}");
}

fn main() {
    desktop_build();
    tauri_build::build()
}
