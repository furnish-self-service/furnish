//! Endpoints: an organization's furnish address (e.g. `furnish.myorg.tld`) that lists its
//! collections and serves them to its signed-in people over the internet, without a VPN.
//!
//! - `https://<endpoint>/.well-known/furnish.json` describes it ([`Description`]): the
//!   organization's name, its OpenID Connect provider and client ID, its collections' git URLs,
//!   and branding, setting and option defaults and apps to install on first launch. Those
//!   defaults never lock anything and never override the admin policy.
//! - Users sign in with the organization's SSO in their browser: OIDC authorization code with
//!   PKCE and a loopback redirect (a public client: furnish holds no secret). The tokens are
//!   kept in the OS's credential store and renewed with the refresh token.
//! - git sends `Authorization: Bearer <token>` to the endpoint's URLs only (see `auth`).
//!
//! The endpoint itself is a reverse proxy with SSO in front of the organization's git server;
//! see packaging/endpoint.

use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// What `/.well-known/furnish.json` says about an endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    /// The organization, e.g. "Acme".
    pub name: String,
    /// The OpenID Connect provider (its `/.well-known/openid-configuration` is fetched).
    pub issuer: String,
    /// A public client at that provider that allows loopback redirects (`http://127.0.0.1:<port>/callback`).
    pub client_id: String,
    /// Which token the endpoint's proxy checks: `id_token` (default) or `access_token`.
    #[serde(default = "default_token")]
    pub token: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    pub collections: Vec<Collection>,
    /// Defaults for this organization, like the admin policy's (same structure), never locked.
    #[serde(default)]
    pub branding: Option<Value>,
    #[serde(default)]
    pub settings: BTreeMap<String, Value>,
    #[serde(default)]
    pub parameters: BTreeMap<String, BTreeMap<String, Value>>,
    /// Apps (`collection/app`) furnish installs on first launch, e.g. the VPN client.
    #[serde(default)]
    pub auto_install: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    /// Its git URL, at the endpoint.
    pub url: String,
}

fn default_token() -> String {
    "id_token".into()
}
fn default_scopes() -> Vec<String> {
    ["openid", "profile", "email", "offline_access"].map(String::from).to_vec()
}

/// Signing in is needed (first time, or the refresh token expired).
#[derive(Debug)]
pub struct LoginRequired {
    pub origin: String,
    pub name: String,
}

impl std::fmt::Display for LoginRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sign in to {} first: furnish login {}", self.name, host(&self.origin))
    }
}
impl std::error::Error for LoginRequired {}

// --- Addresses

/// The endpoint an input means, as an origin (`https://furnish.myorg.tld`), or `None` if it is a
/// git URL or a path: `furnish.myorg.tld`, `https://furnish.myorg.tld`,
/// `furnish://add?endpoint=furnish.myorg.tld`.
pub fn origin(input: &str) -> Option<String> {
    let input = input.trim();
    let input = match input.strip_prefix("furnish://add?endpoint=") {
        Some(rest) => percent_decode(rest),
        None => input.to_string(),
    };
    let (scheme, rest) = match input.split_once("://") {
        Some((s @ ("https" | "http"), rest)) => (s, rest),
        Some(_) => return None,
        None if input.contains(':') || input.contains('/') || std::path::Path::new(&input).exists() => return None,
        None => ("https", input.as_str()),
    };
    // An origin only: host[:port], nothing after it but an optional slash.
    let host = rest.strip_suffix('/').unwrap_or(rest);
    let valid = host.contains('.') || host.starts_with("localhost") || host.starts_with("127.0.0.1");
    (valid && !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c)))
        .then(|| format!("{scheme}://{}", host.to_ascii_lowercase()))
}

fn host(origin: &str) -> &str {
    origin.split_once("://").map_or(origin, |(_, h)| h)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// HTTPS with the operating system's certificates, so an organization's own certificate
/// authority (installed on its computers) works too.
fn http() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder().tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build()).build().into()
}

// --- Descriptions (cached in the data directory)

fn dir() -> PathBuf {
    crate::catalog::data_dir().join("endpoints")
}

fn file(origin: &str, ext: &str) -> PathBuf {
    dir().join(format!("{}.{ext}", host(origin).replace(':', "_")))
}

#[derive(Serialize, Deserialize)]
struct Cached {
    origin: String,
    description: Description,
}

/// Fetch an endpoint's description and keep it.
pub fn discover(origin: &str) -> Result<Description> {
    let url = format!("{origin}/.well-known/furnish.json");
    let description: Description = http().get(&url)
        .call()
        .and_then(|mut r| r.body_mut().read_json())
        .with_context(|| format!("{} isn't a furnish address (no {url})", host(origin)))?;
    for c in &description.collections {
        if !c.url.starts_with(&format!("{origin}/")) {
            bail!("{}: collection {} isn't served by the endpoint itself ({})", host(origin), c.name, c.url);
        }
    }
    fs::create_dir_all(dir())?;
    let cached = Cached { origin: origin.to_string(), description: description.clone() };
    fs::write(file(origin, "json"), serde_json::to_string_pretty(&cached)? + "\n")?;
    Ok(description)
}

/// The endpoints this computer knows (added by the user or preset by the policy and discovered).
pub fn known() -> Vec<(String, Description)> {
    let Ok(entries) = fs::read_dir(dir()) else { return vec![] };
    let mut all: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str::<Cached>(&fs::read_to_string(e.path()).ok()?).ok())
        .map(|c| (c.origin, c.description))
        .collect();
    all.sort_by(|a, b| a.0.cmp(&b.0));
    all
}

/// The known endpoint that serves this URL.
pub fn for_url(url: &str) -> Option<(String, Description)> {
    known().into_iter().find(|(origin, _)| url.starts_with(&format!("{origin}/")))
}

// --- Tokens

#[derive(Serialize, Deserialize, Clone)]
struct Tokens {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    /// Seconds since the epoch.
    expires_at: u64,
}

const KEYRING_SERVICE: &str = "io.github.furnish-self-service.furnish";

/// The credential store: the OS's (Keychain, Credential Manager, Secret Service), or, where there
/// is none (servers, containers; or FURNISH_TOKEN_STORE=file), a file only the user can read.
fn load_tokens(origin: &str) -> Option<Tokens> {
    let text = if use_keyring() {
        keyring::Entry::new(KEYRING_SERVICE, origin).ok()?.get_password().ok()?
    } else {
        fs::read_to_string(file(origin, "token")).ok()?
    };
    serde_json::from_str(&text).ok()
}

fn save_tokens(origin: &str, tokens: &Tokens) -> Result<()> {
    let text = serde_json::to_string(tokens)?;
    if use_keyring() {
        keyring::Entry::new(KEYRING_SERVICE, origin)?.set_password(&text).context("saving the sign-in in the credential store")?;
        return Ok(());
    }
    fs::create_dir_all(dir())?;
    let path = file(origin, "token");
    fs::write(&path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn use_keyring() -> bool {
    if std::env::var("FURNISH_TOKEN_STORE").is_ok_and(|v| v == "file") {
        return false;
    }
    keyring::Entry::store_status().is_ok()
}

/// Sign out: forget the tokens.
pub fn logout(origin: &str) {
    if use_keyring()
        && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, origin)
    {
        let _ = entry.delete_credential();
    }
    let _ = fs::remove_file(file(origin, "token"));
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A JWT's `exp` claim, unverified (only to know when to renew; the endpoint verifies).
fn jwt_exp(jwt: &str) -> Option<u64> {
    let payload = B64.decode(jwt.split('.').nth(1)?.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&payload).ok()?["exp"].as_u64()
}

/// A valid token for the endpoint, renewed if it expires within a minute.
pub fn token(origin: &str, description: &Description) -> Result<String> {
    let required = || LoginRequired { origin: origin.into(), name: description.name.clone() };
    let tokens = load_tokens(origin).ok_or_else(required)?;
    let pick = |t: &Tokens| if description.token == "access_token" { t.access_token.clone() } else { t.id_token.clone() };
    if tokens.expires_at > now() + 60 {
        return pick(&tokens).ok_or_else(|| required().into());
    }
    let refresh = tokens.refresh_token.clone().ok_or_else(required)?;
    let config = provider(&description.issuer)?;
    let response = http().post(&config.token_endpoint)
        .send_form([("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", description.client_id.as_str())])
        .and_then(|mut r| r.body_mut().read_json::<TokenResponse>());
    let Ok(response) = response else {
        logout(origin);
        return Err(required().into());
    };
    let renewed = store(origin, description, response, Some(refresh))?;
    pick(&renewed).ok_or_else(|| required().into())
}

// --- Apps installed on first launch (policy `autoInstall`, from the admin or an endpoint)

fn auto_installed_path() -> PathBuf {
    crate::catalog::data_dir().join("auto-installed.json")
}

/// Apps to install automatically now: listed for automatic install and not done before, so an
/// app the user uninstalled stays uninstalled.
pub fn auto_install_pending() -> Vec<String> {
    let done: Vec<String> = fs::read_to_string(auto_installed_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    crate::policy::load().auto_install.into_iter().filter(|a| !done.contains(a)).collect()
}

/// Remember that an app was installed automatically (or was already there).
pub fn auto_install_done(app: &str) -> Result<()> {
    let mut done: Vec<String> = fs::read_to_string(auto_installed_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if !done.iter().any(|a| a == app) {
        done.push(app.to_string());
        fs::create_dir_all(crate::catalog::data_dir())?;
        fs::write(auto_installed_path(), serde_json::to_string_pretty(&done)? + "\n")?;
    }
    Ok(())
}

// --- Signing in (OIDC authorization code + PKCE, loopback redirect)

#[derive(Deserialize)]
struct ProviderConfig {
    authorization_endpoint: String,
    token_endpoint: String,
}

fn provider(issuer: &str) -> Result<ProviderConfig> {
    let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
    http().get(&url).call().and_then(|mut r| r.body_mut().read_json()).with_context(|| format!("reading {url}"))
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

fn store(origin: &str, description: &Description, r: TokenResponse, old_refresh: Option<String>) -> Result<Tokens> {
    let chosen = if description.token == "access_token" { r.access_token.as_deref() } else { r.id_token.as_deref() };
    let expires_at = chosen.and_then(jwt_exp).or(r.expires_in.map(|s| now() + s)).unwrap_or(now() + 300);
    let tokens = Tokens { id_token: r.id_token, access_token: r.access_token, refresh_token: r.refresh_token.or(old_refresh), expires_at };
    save_tokens(origin, &tokens)?;
    Ok(tokens)
}

fn random() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random numbers: {e}"))?;
    Ok(B64.encode(bytes))
}

/// Open a URL or an HTML file in the user's browser (FURNISH_BROWSER: a command to use
/// instead, e.g. in tests).
pub fn open_browser(url: &str) -> Result<()> {
    let mut cmd = if let Ok(browser) = std::env::var("FURNISH_BROWSER") {
        let mut c = std::process::Command::new(browser);
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().context("opening the browser")?;
    Ok(())
}

/// Sign in to the endpoint in the browser; `show` gets the URL in case the browser doesn't open.
pub fn login(origin: &str, description: &Description, show: impl Fn(&str)) -> Result<()> {
    let config = provider(&description.issuer)?;
    let listener = TcpListener::bind("127.0.0.1:0").context("listening for the sign-in")?;
    let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());
    let (verifier, state) = (random()?, random()?);
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    let scope = description.scopes.join(" ");
    let query: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", &description.client_id),
        ("redirect_uri", &redirect),
        ("scope", &scope),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ];
    let encoded: Vec<String> = query.iter().map(|(k, v)| format!("{k}={}", percent_encode(v))).collect();
    let separator = if config.authorization_endpoint.contains('?') { '&' } else { '?' };
    let url = format!("{}{separator}{}", config.authorization_endpoint, encoded.join("&"));
    show(&url);
    if let Err(err) = open_browser(&url) {
        eprintln!("warning: {err:#}");
    }

    // The browser comes back to us with the code (or an error), once. Five minutes at most.
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let (mut stream, request_line) = loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line)?;
                if line.starts_with("GET /callback") {
                    break (stream, line);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => bail!("no sign-in within five minutes"),
            Err(e) => return Err(e.into()),
        }
    };
    let target = request_line.split_whitespace().nth(1).unwrap_or_default();
    let params: BTreeMap<String, String> = target
        .split_once('?')
        .map(|(_, q)| q.split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), percent_decode(v))).collect())
        .unwrap_or_default();
    let page = |message: &str| {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<!doctype html><title>furnish</title><body style=\"font-family:system-ui;margin:3rem\"><h1>{}</h1><p>{message}</p>",
            html_escape(&description.name)
        )
    };
    if params.get("state") != Some(&state) {
        let _ = stream.write_all(page("This sign-in wasn't started by furnish. Please try again.").as_bytes());
        bail!("the sign-in came back with an unexpected state");
    }
    let Some(code) = params.get("code") else {
        let _ = stream.write_all(page("Signing in didn't work. You can close this window.").as_bytes());
        bail!("signing in failed: {}", params.get("error_description").or(params.get("error")).map_or("no code", |s| s));
    };
    let response = http().post(&config.token_endpoint)
        .send_form([
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("client_id", description.client_id.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .and_then(|mut r| r.body_mut().read_json::<TokenResponse>());
    match response {
        Ok(r) => {
            let _ = stream.write_all(page("You're signed in to furnish. You can close this window.").as_bytes());
            store(origin, description, r, None)?;
            Ok(())
        }
        Err(e) => {
            let _ = stream.write_all(page("Signing in didn't work. You can close this window.").as_bytes());
            Err(e).context("getting the sign-in's tokens")
        }
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_counts_as_an_endpoint() {
        assert_eq!(origin("furnish.myorg.tld").as_deref(), Some("https://furnish.myorg.tld"));
        assert_eq!(origin("https://Furnish.MyOrg.tld/").as_deref(), Some("https://furnish.myorg.tld"));
        assert_eq!(origin("furnish://add?endpoint=furnish.myorg.tld").as_deref(), Some("https://furnish.myorg.tld"));
        assert_eq!(origin("http://127.0.0.1:8080").as_deref(), Some("http://127.0.0.1:8080"));
        // Git URLs and paths are collections, not endpoints.
        assert_eq!(origin("https://github.com/acme/apps.git"), None);
        assert_eq!(origin("git@github.com:acme/apps.git"), None);
        assert_eq!(origin("ssh://git@host/apps.git"), None);
        assert_eq!(origin("/srv/apps"), None);
        assert_eq!(origin("https://evil.example/x?y"), None);
    }

    #[test]
    fn decoding_and_expiry() {
        assert_eq!(percent_decode("a%2Fb+c"), "a/b c");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
        let jwt = format!("x.{}.y", B64.encode(br#"{"exp":1234}"#));
        assert_eq!(jwt_exp(&jwt), Some(1234));
    }
}
