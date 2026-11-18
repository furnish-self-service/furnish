// Exercise the same CLI entry points a collection owner uses to publish and adopt a release.
use std::{fs, path::Path, process::Command};

fn git(repo: &Path, args: &[&str]) -> String {
    git_dated(repo, args, "2000-01-01T00:00:00Z")
}

fn git_dated(repo: &Path, args: &[&str], date: &str) -> String {
    let out = Command::new("git").current_dir(repo)
        .env("GIT_COMMITTER_DATE", date)
        .args(["-c", "user.name=Publisher", "-c", "user.email=publisher@example.test",
               "-c", "commit.gpgsign=false", "-c", "tag.gpgSign=false"])
        .args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn cli(root: &Path, args: &[&str], success: bool) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_furnish")).args(args)
        .env("FURNISH_DATA_DIR", root.join("data"))
        .env("FURNISH_SYSTEM_DIR", root.join("system"))
        .env("FURNISH_POLICY_FILE", root.join("policy.json"))
        .output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.success(), success, "furnish {args:?}: {text}");
    text
}

#[test]
fn cli_adds_a_release_and_switches_tracks_without_losing_the_previous_checkout_on_error() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = root.join("published");
    fs::create_dir_all(repo.join("apps")).unwrap();
    git(&repo, &["init", "--initial-branch=main"]);
    let publish = |note: &str| {
        fs::write(repo.join("apps/tool.dsc.yaml"), format!(
            "metadata: {{ furnish: {{ description: {note} }} }}\nresources:\n  - name: editor\n    type: Furnish/Package\n    properties: {{ name: editor }}\n"
        )).unwrap();
        git(&repo, &["add", "apps"]);
        git(&repo, &["commit", "-m", note]);
        git(&repo, &["rev-parse", "HEAD"])
    };
    let first = publish("released");
    git(&repo, &["tag", "-a", "v1.0.0", "-m", "First release"]);
    git(&repo, &["branch", "maintenance"]);
    publish("under development");
    git(&repo, &["tag", "preview"]);
    let checkout = root.join("data/collections/work");
    let head = || git(&checkout, &["rev-parse", "HEAD"]);
    let pattern = r"^v[0-9]+\.[0-9]+\.[0-9]+$";

    cli(root, &["collection", "add", repo.to_str().unwrap(), "--name", "work", "--tag-pattern", pattern], true);
    assert_eq!(head(), first);
    cli(root, &["collection", "update", "work"], true);
    assert_eq!(head(), first);
    let next = publish("second release");
    git(&repo, &["tag", "v1.0.1"]);
    cli(root, &["collection", "update", "work"], true);
    assert_eq!(head(), next);

    // Tagging a maintenance release today can take precedence over a higher version.
    git_dated(&repo, &["tag", "-a", "v0.9.4", &first, "-m", "Maintenance release"], "2025-12-03T12:00:00Z");
    cli(root, &["collection", "settings", "work", "--tag-pattern", pattern, "--tag-order", "newest"], true);
    assert_eq!(head(), first);
    assert!(cli(root, &["collection", "settings", "work"], true).contains("newest by date"));
    cli(root, &["collection", "update", "work"], true);
    assert_eq!(head(), first);
    cli(root, &["collection", "settings", "work", "--tag-pattern", pattern, "--tag-order", "version"], true);
    assert_eq!(head(), next);
    cli(root, &["collection", "settings", "work", "--tag-order", "newest"], false);
    cli(root, &["collection", "settings", "work", "--tag-pattern", pattern, "--tag-order", "unknown"], false);

    cli(root, &["collection", "settings", "work", "--tag-pattern", "^missing$"], false);
    cli(root, &["collection", "settings", "work", "--branch", "missing"], false);
    cli(root, &["collection", "settings", "work", "--tag-pattern", "["], false);
    assert_eq!(head(), next);
    assert!(cli(root, &["collection", "settings", "work"], true).contains(pattern));

    cli(root, &["collection", "settings", "work", "--branch", "maintenance"], true);
    assert_eq!(head(), first);
    cli(root, &["collection", "settings", "work", "--branch", "main", "--tag-pattern", "."], false);
    assert_eq!(head(), first);
    cli(root, &["collection", "settings", "work", "--default-branch"], true);
    assert_eq!(head(), next);
    git(&repo, &["branch", "-m", "next-default"]);
    let renamed = publish("new default branch");
    cli(root, &["collection", "update", "work"], true);
    assert_eq!(head(), renamed);
    assert!(cli(root, &["collection", "settings", "work"], true).contains("remote default branch"));
}
