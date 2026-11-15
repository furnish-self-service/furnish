//! Credentials for collections' git servers, besides what git itself provides (SSH keys and the
//! agent, credential helpers, credentials in the URL), which keep working as with plain git.
//!
//! - HTTP: a username and password (or access token) for a collection's URL, kept in the OS's
//!   credential store, sent as HTTP Basic for that repository only (Git LFS included).
//! - Endpoints: the user's sign-in (see `endpoint`).
//!
//! Either way git gets an `http.<url>.extraHeader` through environment variables, so nothing
//! shows up in process lists or in the repository's config.

use std::{cell::RefCell, collections::BTreeMap};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};

/// The git server wants a username and password (or access token) for this URL.
#[derive(Debug)]
pub struct CredentialsRequired {
    pub url: String,
}

impl std::fmt::Display for CredentialsRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} needs a username and password (or access token)", self.url)
    }
}
impl std::error::Error for CredentialsRequired {}

/// Whether git failed because the server wants (other) credentials.
pub fn is_auth_failure(stderr: &str) -> bool {
    ["Authentication failed", "could not read Username", "could not read Password", "terminal prompts disabled", "401", "403 Forbidden", "HTTP Basic: Access denied"]
        .iter()
        .any(|s| stderr.contains(s))
}

/// Advice for SSH failures in an app without a terminal to ask in.
pub fn ssh_advice(stderr: &str) -> Option<&'static str> {
    if stderr.contains("Host key verification failed") {
        Some("the server's SSH host key isn't known yet: connect once in a terminal (ssh <host>) to check and accept it")
    } else if stderr.contains("Permission denied (publickey") {
        Some("the server didn't accept your SSH key: add it to your SSH agent (ssh-add) or ~/.ssh/config, or use the HTTPS URL")
    } else {
        None
    }
}

const KEYRING_SERVICE: &str = "io.github.furnish-self-service.furnish.git";

/// The repository URL credentials belong to, as given (git matches `http.<url>.*` by whole path
/// segments, so `…/apps` wouldn't cover `…/apps.git/info/refs`).
fn key(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn file() -> std::path::PathBuf {
    crate::catalog::data_dir().join("git-credentials.json")
}

fn use_keyring() -> bool {
    std::env::var("FURNISH_TOKEN_STORE").map_or(true, |v| v != "file") && keyring::Entry::store_status().is_ok()
}

/// Keep a username and password (or token) for a collection's URL.
pub fn store(url: &str, username: &str, password: &str) -> Result<()> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        bail!("usernames and passwords are for HTTPS URLs; SSH uses your keys and agent");
    }
    let secret = serde_json::to_string(&(username, password))?;
    if use_keyring() {
        keyring::Entry::new(KEYRING_SERVICE, &key(url))?.set_password(&secret).context("saving the credentials in the credential store")?;
        return Ok(());
    }
    // No credential store (servers, containers): a file only the user can read.
    let mut all = file_entries();
    all.insert(key(url), secret);
    std::fs::create_dir_all(crate::catalog::data_dir())?;
    std::fs::write(file(), serde_json::to_string_pretty(&all)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file(), std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn file_entries() -> BTreeMap<String, String> {
    std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn load(url: &str) -> Option<(String, String)> {
    let secret = if use_keyring() {
        keyring::Entry::new(KEYRING_SERVICE, &key(url)).ok()?.get_password().ok()?
    } else {
        file_entries().remove(&key(url))?
    };
    serde_json::from_str(&secret).ok()
}

/// Forget a collection's credentials (when it is removed).
pub fn forget(url: &str) {
    if use_keyring()
        && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &key(url))
    {
        let _ = entry.delete_credential();
    }
    let mut all = file_entries();
    if all.remove(&key(url)).is_some() {
        let _ = std::fs::write(file(), serde_json::to_string_pretty(&all).unwrap_or_default());
    }
}

thread_local! {
    /// Headers handed over by a client (the furnish service fetches with the user's credentials).
    static OVERRIDE: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

/// Run `work` with these headers (URL prefix → `Authorization: …`) instead of the stored ones.
pub fn with_headers<T>(headers: BTreeMap<String, String>, work: impl FnOnce() -> T) -> T {
    OVERRIDE.with(|o| *o.borrow_mut() = headers);
    let result = work();
    OVERRIDE.with(|o| o.borrow_mut().clear());
    result
}

/// The `Authorization` header for a URL and the URL prefix it applies to, if furnish has one.
fn header(url: &str) -> Result<Option<(String, String)>> {
    if let Some(found) = OVERRIDE.with(|o| o.borrow().iter().find(|(prefix, _)| url.starts_with(prefix.as_str())).map(|(p, h)| (p.clone(), h.clone()))) {
        return Ok(Some(found));
    }
    if let Some((origin, description)) = crate::endpoint::for_url(url) {
        let token = crate::endpoint::token(&origin, &description)?;
        return Ok(Some((format!("{origin}/"), format!("Authorization: Bearer {token}"))));
    }
    Ok(load(url).map(|(user, pass)| (key(url), format!("Authorization: Basic {}", STANDARD.encode(format!("{user}:{pass}"))))))
}

/// Environment for a git command talking to `url`: never a terminal prompt (furnish asks
/// instead), plus furnish's credentials for that URL, if it has any.
pub fn git_env(url: &str) -> Result<Vec<(String, String)>> {
    let mut env = vec![("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())];
    if let Some((prefix, header)) = header(url)? {
        env.extend([
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            ("GIT_CONFIG_KEY_0".into(), format!("http.{prefix}.extraHeader")),
            ("GIT_CONFIG_VALUE_0".into(), header),
        ]);
    }
    Ok(env)
}

/// Headers for these URLs (the user's endpoint sign-ins and stored credentials), to hand to the
/// furnish service, which fetches the organization's collections on the user's behalf.
pub fn headers_for(urls: impl IntoIterator<Item = String>) -> BTreeMap<String, String> {
    urls.into_iter().filter_map(|url| header(&url).ok().flatten()).collect()
}

/// Ask for credentials the way git does (its credential helpers, then the terminal), for the
/// command line; `None` without a terminal.
pub fn ask_in_terminal(url: &str) -> Result<Option<(String, String)>> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut child = std::process::Command::new("git")
        .args(["credential", "fill"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("running git credential")?;
    child.stdin.take().context("git credential's input")?.write_all(format!("url={url}\n\n").as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |name: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{name}="))).map(String::from);
    Ok(field("username").zip(field("password")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handed_over_headers_apply_by_url_prefix() {
        let headers = BTreeMap::from([("https://git.acme.tld/it/apps.git".to_string(), "Authorization: Basic eDp5".to_string())]);
        with_headers(headers, || {
            let env = git_env("https://git.acme.tld/it/apps.git").unwrap();
            assert!(env.contains(&("GIT_CONFIG_KEY_0".into(), "http.https://git.acme.tld/it/apps.git.extraHeader".into())));
            assert!(env.contains(&("GIT_CONFIG_VALUE_0".into(), "Authorization: Basic eDp5".into())));
            // Other URLs get nothing but the no-prompt setting.
            assert_eq!(git_env("https://elsewhere.tld/x.git").unwrap(), [("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())]);
        });
    }

    #[test]
    fn recognizes_failures() {
        assert!(is_auth_failure("fatal: Authentication failed for 'https://x/'"));
        assert!(is_auth_failure("fatal: could not read Username for 'https://x': terminal prompts disabled"));
        assert!(!is_auth_failure("fatal: repository 'https://x/' not found"));
        assert!(ssh_advice("Host key verification failed.").is_some());
    }
}
