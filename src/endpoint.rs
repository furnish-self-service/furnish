//! Endpoints: an organization's furnish address (e.g. `furnish.myorg.tld`) that lists its
//! collections and serves them to its signed-in people over the internet, without a VPN.
//!
//! - `https://<endpoint>/.well-known/furnish.json` describes it ([`Description`]): the
//!   organization's name, its OpenID Connect provider and client ID, its collections' git URLs,
//!   and branding, setting and option defaults and apps to install on first launch. Those
//!   defaults never lock anything, never override the admin policy, and only concern the
//!   endpoint's own collections (see `policy::with_endpoints`).
//! - The endpoint and its identity provider use https (plain http only on this computer), and the
//!   provider's configuration names exactly the issuer the endpoint names.
//! - Users sign in with the organization's SSO in their browser: OIDC authorization code with
//!   PKCE and a loopback redirect (a public client: furnish holds no secret). The tokens are
//!   kept in the OS's credential store, bound to the issuer they came from, and renewed with the
//!   refresh token at that issuer's token endpoint.
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
    /// Where furnish sends reports (see `reports`), on this endpoint, e.g. https://furnish.myorg.tld/events.
    #[serde(default)]
    pub events: Option<String>,
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

/// The endpoint didn't answer with its description (offline, its server down, an HTTP error).
#[derive(Debug)]
pub struct Unreachable(String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// --- Addresses

/// The endpoint an input means, as an origin (`https://furnish.myorg.tld`), or `None` if it is a
/// git URL or a path: `furnish.myorg.tld`, `https://furnish.myorg.tld`,
/// `furnish://add?endpoint=furnish.myorg.tld`. Plain `http` only for this computer itself
/// (`localhost`, `127.0.0.1`, `[::1]`).
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
    let authority = rest.strip_suffix('/').unwrap_or(rest).to_ascii_lowercase();
    let (host, port) = split_authority(&authority)?;
    let named = !host.starts_with(['.', '-']) && !host.ends_with(['.', '-']) && !host.contains("..")
        && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
        && (host.contains('.') || host == "localhost");
    let valid = (named || host == "[::1]") && (port.is_empty() || (port.len() <= 5 && port.chars().all(|c| c.is_ascii_digit())));
    (valid && (scheme == "https" || plain_http_allowed(host))).then(|| format!("{scheme}://{authority}"))
}

/// A furnish link (`furnish://add?endpoint=furnish.myorg.tld`): the furnish address it names, or
/// `None` for anything else, such as a git URL or a path.
pub fn link_origin(link: &str) -> Option<String> {
    let value = link.trim().strip_prefix("furnish://add?endpoint=")?;
    let value = value.split(['&', '#']).next().unwrap_or_default();
    origin(&percent_decode(value))
}

/// Host and port (without the colon; empty if none) of `host[:port]` or `[v6]:port`.
fn split_authority(authority: &str) -> Option<(&str, &str)> {
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let rest = &authority[end + 1..];
        return match rest.strip_prefix(':') {
            Some(port) => Some((&authority[..=end], port)),
            None if rest.is_empty() => Some((authority, "")),
            None => None,
        };
    }
    Some(authority.split_once(':').unwrap_or((authority, "")))
}

/// Plain http only reaches this computer; FURNISH_ALLOW_HTTP (comma-separated host names) adds
/// hosts for tests.
fn plain_http_allowed(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
        || std::env::var("FURNISH_ALLOW_HTTP").is_ok_and(|hosts| hosts.split(',').any(|h| h.trim() == host))
}

/// Whether furnish may send a sign-in to this URL: https, or http to this computer.
pub(crate) fn secure_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    match (scheme, split_authority(&authority)) {
        ("https", Some(_)) => true,
        ("http", Some((host, _))) => plain_http_allowed(host),
        _ => false,
    }
}

/// The host of a URL, e.g. `login.myorg.tld`, to show people where they sign in.
pub fn url_host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or_default().rsplit('@').next().unwrap_or_default().to_ascii_lowercase()
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
pub(crate) fn http() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder().tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build()).build().into()
}

// --- Descriptions (cached in the data directory)

fn dir() -> PathBuf {
    crate::catalog::data_dir().join("endpoints")
}

/// The file for an origin: its scheme, host and port, e.g. `https_furnish.myorg.tld.json`.
fn file(origin: &str, ext: &str) -> PathBuf {
    dir().join(format!("{}.{ext}", stem(origin)))
}

fn stem(origin: &str) -> String {
    origin.replace("://", "_").chars().map(|c| if c.is_ascii_alphanumeric() || ".-".contains(c) { c } else { '_' }).collect()
}

#[derive(Serialize, Deserialize)]
struct Cached {
    origin: String,
    description: Description,
    /// When the endpoint was added (seconds since the epoch), to know which came first.
    #[serde(default)]
    added: u64,
}

/// Fetch an endpoint's description and check it: its collections and reports are on the endpoint
/// itself, and its identity provider is reached over https.
pub fn fetch(origin: &str) -> Result<Description> {
    if self::origin(origin).as_deref() != Some(origin) {
        bail!("'{origin}' isn't a furnish address (https://furnish.myorg.tld)");
    }
    let url = format!("{origin}/.well-known/furnish.json");
    let not_furnish = || format!("{} isn't a furnish address (no {url})", host(origin));
    let mut response = http().get(&url).call().map_err(|e| anyhow::Error::new(e).context(Unreachable(not_furnish())))?;
    let description: Description = response.body_mut().read_json().with_context(not_furnish)?;
    for c in &description.collections {
        if !c.url.starts_with(&format!("{origin}/")) {
            bail!("{}: collection {} isn't served by the endpoint itself ({})", host(origin), c.name, c.url);
        }
        if c.name.is_empty() || c.name.starts_with('.') || c.name.contains(['/', '\\', ':']) {
            bail!("{}: '{}' isn't a valid collection name", host(origin), c.name);
        }
    }
    if !secure_url(&description.issuer) {
        bail!("{}: the identity provider {} isn't an https address", host(origin), description.issuer);
    }
    if !matches!(description.token.as_str(), "id_token" | "access_token") {
        bail!("{}: token must be id_token or access_token, not {}", host(origin), description.token);
    }
    Ok(description)
}

/// Fetch an endpoint's description; keep it if the endpoint is known already or the admin policy
/// names it. Endpoints the user adds are kept once they signed in (see `login`).
pub fn discover(origin: &str) -> Result<Description> {
    let description = fetch(origin)?;
    let named = crate::policy::load_native().endpoints.iter().any(|e| self::origin(e).as_deref() == Some(origin));
    if named || file(origin, "json").exists() {
        remember(origin, &description)?;
    }
    Ok(description)
}

/// Fetch the descriptions of the known endpoints again, the way `discover` does when adding one,
/// so what an organization changes reaches everyone who added its address. Without `usersCanAdd`,
/// only the endpoints the admin policy names. An endpoint that doesn't answer keeps its saved
/// description quietly (a log line); one that answers with a description that fails the checks
/// keeps it too, and is returned as a problem.
pub fn refresh() -> Vec<String> {
    let mut problems = Vec::new();
    for (origin, _) in known() {
        if allowed(&origin).is_err() {
            continue;
        }
        match discover(&origin) {
            Ok(_) => {}
            Err(err) if err.is::<Unreachable>() => eprintln!("note: keeping the saved description of {}: {}", host(&origin), err.root_cause()),
            Err(err) => problems.push(format!("{err:#}; keeping the saved description")),
        }
    }
    for warning in crate::policy::load().warnings {
        eprintln!("warning: {warning}");
    }
    problems
}

fn remember(origin: &str, description: &Description) -> Result<()> {
    private_dir(&dir())?;
    let path = file(origin, "json");
    let old = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Cached>(&t).ok());
    // Another identity provider or client: the sign-in from the old one goes (see `Tokens`).
    if old.as_ref().is_some_and(|c| c.description.issuer != description.issuer || c.description.client_id != description.client_id) {
        logout(origin);
    }
    let added = old.map_or_else(now, |c| c.added);
    let cached = Cached { origin: origin.to_string(), description: description.clone(), added };
    crate::auth::write_private(&path, (serde_json::to_string_pretty(&cached)? + "\n").as_bytes())?;
    Ok(())
}

/// The endpoints this computer knows (added by the user or preset by the policy and discovered),
/// the first added first.
pub fn known() -> Vec<(String, Description)> {
    let Ok(entries) = fs::read_dir(dir()) else { return vec![] };
    let mut all: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let cached = serde_json::from_str::<Cached>(&fs::read_to_string(e.path()).ok()?).ok()?;
            migrate(&e.path(), &cached).then_some(cached)
        })
        .map(|c| (c.added, c.origin, c.description))
        .collect();
    all.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    all.into_iter().map(|(_, origin, d)| (origin, d)).collect()
}

/// Files of earlier versions were named by host only (so http:// and https:// shared one): move
/// the description to its new name and drop the sign-in, which wasn't bound to its identity
/// provider (see `Tokens`). Plain-http addresses other than this computer are dropped.
fn migrate(path: &std::path::Path, cached: &Cached) -> bool {
    let Some(old) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { return false };
    if origin(&cached.origin).as_deref() != Some(cached.origin.as_str()) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(dir().join(format!("{old}.token")));
        return false;
    }
    let new = file(&cached.origin, "json");
    if path == new {
        return true;
    }
    let _ = fs::remove_file(dir().join(format!("{old}.token")));
    if new.exists() {
        let _ = fs::remove_file(path);
        return false; // the new file is listed on its own
    }
    fs::rename(path, &new).is_ok()
}

/// The known endpoint that serves this URL.
pub fn for_url(url: &str) -> Option<(String, Description)> {
    known().into_iter().find(|(origin, _)| url.starts_with(&format!("{origin}/")))
}

/// A directory only the user can open, on Unix.
fn private_dir(path: &std::path::Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

// --- What an endpoint does, for people to confirm before adding it

/// What adding an endpoint changes, shown before signing in: where people sign in, what it
/// installs, whether it turns reports on, and whether it brands the app.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub origin: String,
    pub name: String,
    /// Where people enter their password: the identity provider's sign-in page's host.
    pub sign_in_host: String,
    pub collections: Vec<String>,
    /// Apps it installs right away.
    pub auto_install: Vec<String>,
    /// `errors` or `all` if it turns reports on (about its own collections), else `None`.
    pub reporting: Option<String>,
    /// Whether it gives the app its name, logo and colors.
    pub branding: bool,
}

/// Fetch an endpoint's description and its identity provider's configuration without keeping
/// anything, and say what adding it would change. Fails if the policy doesn't allow adding it.
pub fn preview(origin: &str) -> Result<Preview> {
    let native = allowed(origin)?;
    let description = fetch(origin)?;
    let config = provider(&description.issuer)?;
    let others: Vec<_> = known().into_iter().filter(|(o, _)| o != origin).collect();
    let before = crate::policy::with_known_endpoints(native.clone(), others.clone());
    let mut with = others;
    with.push((origin.to_string(), description.clone()));
    let after = crate::policy::with_known_endpoints(native, with);
    let reporting = after.settings.get("reporting").and_then(|e| e.value.as_str()).filter(|v| *v != "off").map(String::from);
    let reporting = reporting.filter(|_| description.events.is_some() && before.settings.get("reporting") != after.settings.get("reporting"));
    Ok(Preview {
        origin: origin.to_string(),
        name: description.name.clone(),
        sign_in_host: url_host(&config.authorization_endpoint),
        collections: description.collections.iter().map(|c| c.name.clone()).collect(),
        auto_install: after.auto_install.into_iter().filter(|a| !before.auto_install.contains(a)).collect(),
        reporting,
        branding: after.branding != before.branding,
    })
}

/// The admin policy, if it allows using this endpoint: without `usersCanAdd`, only the endpoints
/// it names.
fn allowed(origin: &str) -> Result<crate::policy::Policy> {
    let native = crate::policy::load_native();
    let named = native.endpoints.iter().any(|e| self::origin(e).as_deref() == Some(origin));
    if !native.collections.users_can_add && !named {
        bail!("your organization manages the collections; adding other furnish addresses isn't allowed");
    }
    Ok(native)
}

// --- Tokens

/// A sign-in, bound to the identity provider and client it came from: if the endpoint names
/// another one later, people sign in again instead of furnish sending the refresh token there.
#[derive(Serialize, Deserialize, Clone)]
struct Tokens {
    issuer: String,
    client_id: String,
    token_endpoint: String,
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
    private_dir(&dir())?;
    crate::auth::write_private(&file(origin, "token"), text.as_bytes())?;
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
    // A sign-in at another provider or client than the endpoint names now doesn't count.
    let tokens = load_tokens(origin)
        .filter(|t| t.issuer == description.issuer && t.client_id == description.client_id)
        .ok_or_else(required)?;
    let pick = |t: &Tokens| if description.token == "access_token" { t.access_token.clone() } else { t.id_token.clone() };
    if tokens.expires_at > now() + 60 {
        return pick(&tokens).ok_or_else(|| required().into());
    }
    let refresh = tokens.refresh_token.clone().ok_or_else(required)?;
    if !secure_url(&tokens.token_endpoint) {
        return Err(required().into());
    }
    let mut response = http()
        .post(&tokens.token_endpoint)
        .config()
        .http_status_as_error(false)
        .build()
        .send_form([("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", description.client_id.as_str())])
        .with_context(|| format!("renewing the sign-in to {}", description.name))?;
    let status = response.status().as_u16();
    let body: Value = response.body_mut().read_json().unwrap_or(Value::Null);
    match refresh_outcome(status, body) {
        Refreshed::Renewed(r) => {
            let renewed = store(origin, description, &tokens.token_endpoint, r, Some(refresh))?;
            pick(&renewed).ok_or_else(|| required().into())
        }
        // The provider no longer accepts the refresh token (expired, revoked): sign in again.
        Refreshed::Rejected => {
            logout(origin);
            Err(required().into())
        }
        // Anything else (provider down, a proxy in between): keep the sign-in for the next try.
        Refreshed::Failed(why) => bail!("renewing the sign-in to {} failed: {why}", description.name),
    }
}

enum Refreshed {
    Renewed(TokenResponse),
    Rejected,
    Failed(String),
}

fn refresh_outcome(status: u16, body: Value) -> Refreshed {
    let error = body["error"].as_str().unwrap_or_default();
    match status {
        200..=299 => serde_json::from_value(body).map_or_else(|e| Refreshed::Failed(format!("unexpected answer: {e}")), Refreshed::Renewed),
        400 | 401 if error == "invalid_grant" => Refreshed::Rejected,
        _ => Refreshed::Failed(format!("HTTP {status} {error}").trim_end().to_string()),
    }
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
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    /// RFC 9207: the provider names itself (`iss`) in the redirect back to furnish.
    #[serde(default)]
    authorization_response_iss_parameter_supported: bool,
}

/// The identity provider's configuration. It must name exactly the issuer the endpoint names, and
/// its endpoints must be https.
fn provider(issuer: &str) -> Result<ProviderConfig> {
    if !secure_url(issuer) {
        bail!("the identity provider {issuer} isn't an https address");
    }
    let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
    let config: ProviderConfig = http().get(&url).call().and_then(|mut r| r.body_mut().read_json()).with_context(|| format!("reading {url}"))?;
    check_provider(issuer, &config).with_context(|| format!("checking {url}"))?;
    Ok(config)
}

fn check_provider(issuer: &str, config: &ProviderConfig) -> Result<()> {
    if config.issuer != issuer {
        bail!("it names the issuer {} instead of {issuer}", config.issuer);
    }
    for url in [&config.authorization_endpoint, &config.token_endpoint] {
        if !secure_url(url) {
            bail!("{url} isn't an https address");
        }
    }
    Ok(())
}

/// RFC 9207: the redirect's `iss`, if the provider sends one (and it must if it says it does),
/// is the issuer the endpoint names. Stops a provider from sending another's code to furnish.
fn check_iss(params: &BTreeMap<String, String>, issuer: &str, required: bool) -> Result<()> {
    match params.get("iss") {
        Some(iss) if iss != issuer => bail!("the sign-in came back from {iss} instead of {issuer}"),
        None if required => bail!("the sign-in came back without naming its identity provider"),
        _ => Ok(()),
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

fn store(origin: &str, description: &Description, token_endpoint: &str, r: TokenResponse, old_refresh: Option<String>) -> Result<Tokens> {
    let chosen = if description.token == "access_token" { r.access_token.as_deref() } else { r.id_token.as_deref() };
    let expires_at = chosen.and_then(jwt_exp).or(r.expires_in.map(|s| now() + s)).unwrap_or(now() + 300);
    let tokens = Tokens {
        issuer: description.issuer.clone(),
        client_id: description.client_id.clone(),
        token_endpoint: token_endpoint.to_string(),
        id_token: r.id_token,
        access_token: r.access_token,
        refresh_token: r.refresh_token.or(old_refresh),
        expires_at,
    };
    save_tokens(origin, &tokens)?;
    Ok(tokens)
}

fn random() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random numbers: {e}"))?;
    Ok(B64.encode(bytes))
}

/// Open a web page in the user's browser: https, or http on this computer (FURNISH_BROWSER: a
/// command to use instead, e.g. in tests).
pub fn open_browser(url: &str) -> Result<()> {
    if !secure_url(url) {
        bail!("not opening {url}: only https addresses open in the browser");
    }
    launch(url)
}

/// Open a local HTML file in the user's browser.
pub fn open_file(path: &std::path::Path) -> Result<()> {
    if !path.is_absolute() || !path.is_file() {
        bail!("{} isn't a file", path.display());
    }
    launch(&path.to_string_lossy())
}

fn launch(target: &str) -> Result<()> {
    let mut cmd = if let Ok(browser) = std::env::var("FURNISH_BROWSER") {
        let mut c = std::process::Command::new(browser);
        c.arg(target);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", target]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(target);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(target);
        c
    };
    cmd.spawn().context("opening the browser")?;
    Ok(())
}

/// How long the browser has to come back with the sign-in.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// Sign in to the endpoint in the browser; `show` gets the URL in case the browser doesn't open.
/// Keeps the endpoint's description once signed in (see `discover`).
pub fn login(origin: &str, description: &Description, show: impl Fn(&str)) -> Result<()> {
    allowed(origin)?;
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

    let page = |status: &str, message: &str| {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<!doctype html><title>furnish</title><body style=\"font-family:system-ui;margin:3rem\"><h1>{}</h1><p>{message}</p>",
            html_escape(&description.name)
        )
    };
    // The browser comes back with the code (or an error). Other requests (another program on this
    // computer, a stale tab) get an error page and furnish keeps waiting, five minutes at most.
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    let (mut stream, params) = loop {
        match listener.accept() {
            Ok((mut stream, _)) => match callback(&stream) {
                Some(params) if params.get("state") == Some(&state) => break (stream, params),
                _ => {
                    let _ = stream.write_all(page("400 Bad Request", "This sign-in wasn't started by furnish. Please try again.").as_bytes());
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => bail!("no sign-in within five minutes"),
            Err(e) => return Err(e.into()),
        }
    };
    let failed = page("200 OK", "Signing in didn't work. You can close this window.");
    if let Err(err) = check_iss(&params, &description.issuer, config.authorization_response_iss_parameter_supported) {
        let _ = stream.write_all(failed.as_bytes());
        return Err(err);
    }
    let Some(code) = params.get("code") else {
        let _ = stream.write_all(failed.as_bytes());
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
            let _ = stream.write_all(page("200 OK", "You're signed in to furnish. You can close this window.").as_bytes());
            store(origin, description, &config.token_endpoint, r, None)?;
            remember(origin, description)
        }
        Err(e) => {
            let _ = stream.write_all(failed.as_bytes());
            Err(e).context("getting the sign-in's tokens")
        }
    }
}

/// The query parameters of a `GET /callback?…` request on the loopback listener, or `None` for
/// any other request. Reads two seconds and 8 KiB at most, so no client can hold up the sign-in.
fn callback(stream: &std::net::TcpStream) -> Option<BTreeMap<String, String>> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut line = String::new();
    BufReader::new(std::io::Read::take(stream, 8192)).read_line(&mut line).ok()?;
    parse_callback(&line)
}

fn parse_callback(request_line: &str) -> Option<BTreeMap<String, String>> {
    let mut words = request_line.split_whitespace();
    let (method, target) = (words.next()?, words.next()?);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (method == "GET" && path == "/callback").then(|| {
        query.split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), percent_decode(v))).collect()
    })
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
        assert_eq!(origin("https://furnish.myorg.tld:8443").as_deref(), Some("https://furnish.myorg.tld:8443"));
        assert_eq!(origin("http://127.0.0.1:8080").as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(origin("http://localhost:8080").as_deref(), Some("http://localhost:8080"));
        assert_eq!(origin("http://[::1]:8080").as_deref(), Some("http://[::1]:8080"));
        // Plain http only on this computer.
        assert_eq!(origin("http://furnish.myorg.tld"), None);
        assert_eq!(origin("http://localhost.evil.tld"), None);
        // Git URLs and paths are collections, not endpoints.
        assert_eq!(origin("https://github.com/acme/apps.git"), None);
        assert_eq!(origin("git@github.com:acme/apps.git"), None);
        assert_eq!(origin("ssh://git@host/apps.git"), None);
        assert_eq!(origin("/srv/apps"), None);
        assert_eq!(origin(r"\\attacker.tld\share"), None);
        assert_eq!(origin("https://evil.example/x?y"), None);
        assert_eq!(origin("https://user@furnish.myorg.tld"), None);
        assert_eq!(origin("https://furnish.myorg.tld:http"), None);
        assert_eq!(origin("https://..tld"), None);
    }

    #[test]
    fn links_only_add_furnish_addresses() {
        assert_eq!(link_origin("furnish://add?endpoint=furnish.myorg.tld").as_deref(), Some("https://furnish.myorg.tld"));
        assert_eq!(link_origin("furnish://add?endpoint=https%3A%2F%2Ffurnish.myorg.tld&x=1").as_deref(), Some("https://furnish.myorg.tld"));
        for link in [
            "furnish://add?endpoint=https%3A%2F%2Fgithub.com%2Facme%2Fapps.git",
            "furnish://add?endpoint=git%40github.com%3Aacme%2Fapps.git",
            "furnish://add?endpoint=%5C%5Cattacker.tld%5Cshare",
            "furnish://add?endpoint=file%3A%2F%2F%2Fetc",
            "furnish://add?endpoint=%2Ftmp%2Frepo",
            "furnish://add?endpoint=http%3A%2F%2Ffurnish.myorg.tld",
            "furnish://add?endpoint=%3Cb%3Ex%3C%2Fb%3E.tld",
            "furnish://other?endpoint=furnish.myorg.tld",
            "https://furnish.myorg.tld",
        ] {
            assert_eq!(link_origin(link), None, "{link}");
        }
    }

    #[test]
    fn sign_ins_only_go_over_https() {
        assert!(secure_url("https://login.myorg.tld/realms/x"));
        assert!(secure_url("http://127.0.0.1:8080/x"));
        assert!(secure_url("http://[::1]/x"));
        assert!(!secure_url("http://login.myorg.tld/x"));
        assert!(!secure_url("file:///etc/passwd"));
        assert!(!secure_url("javascript:alert(1)"));
        assert!(!secure_url("smb://host/share"));
        assert!(!secure_url("https://"));
        assert!(!secure_url("http://user@127.0.0.1/x"));
        assert_eq!(url_host("https://login.myorg.tld/realms/x?y"), "login.myorg.tld");
    }

    fn config(issuer: &str, token_endpoint: &str) -> ProviderConfig {
        ProviderConfig {
            issuer: issuer.into(),
            authorization_endpoint: "https://login.acme.tld/auth".into(),
            token_endpoint: token_endpoint.into(),
            authorization_response_iss_parameter_supported: false,
        }
    }

    #[test]
    fn the_provider_must_be_the_one_the_endpoint_names() {
        let issuer = "https://login.acme.tld/realms/acme";
        assert!(check_provider(issuer, &config(issuer, "https://login.acme.tld/token")).is_ok());
        // Another issuer's metadata (a lookalike pointing at its own token endpoint, or a typo).
        assert!(check_provider(issuer, &config("https://login.evil.tld", "https://login.evil.tld/token")).is_err());
        assert!(check_provider(issuer, &config(&format!("{issuer}/"), "https://login.acme.tld/token")).is_err());
        assert!(check_provider(issuer, &config(issuer, "http://login.acme.tld/token")).is_err());
    }

    #[test]
    fn the_redirect_names_the_issuer_if_it_names_one() {
        let issuer = "https://login.acme.tld";
        let with = |iss: &str| BTreeMap::from([("iss".to_string(), iss.to_string())]);
        assert!(check_iss(&BTreeMap::new(), issuer, false).is_ok());
        assert!(check_iss(&BTreeMap::new(), issuer, true).is_err());
        assert!(check_iss(&with(issuer), issuer, true).is_ok());
        assert!(check_iss(&with("https://login.evil.tld"), issuer, false).is_err());
    }

    #[test]
    fn only_get_callback_counts() {
        let params = parse_callback("GET /callback?code=a%2Fb&state=s HTTP/1.1\r\n").unwrap();
        assert_eq!((params["code"].as_str(), params["state"].as_str()), ("a/b", "s"));
        assert!(parse_callback("GET /favicon.ico HTTP/1.1").is_none());
        assert!(parse_callback("POST /callback?code=x HTTP/1.1").is_none());
        assert!(parse_callback("GET /callbackx?code=x HTTP/1.1").is_none());
        assert!(parse_callback("").is_none());
    }

    #[test]
    fn a_silent_client_cant_hold_up_the_sign_in() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _silent = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let started = Instant::now();
        assert!(callback(&stream).is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn only_a_rejected_refresh_token_signs_out() {
        assert!(matches!(refresh_outcome(400, serde_json::json!({"error": "invalid_grant"})), Refreshed::Rejected));
        assert!(matches!(refresh_outcome(401, serde_json::json!({"error": "invalid_grant"})), Refreshed::Rejected));
        assert!(matches!(refresh_outcome(503, Value::Null), Refreshed::Failed(_)));
        assert!(matches!(refresh_outcome(400, serde_json::json!({"error": "invalid_request"})), Refreshed::Failed(_)));
        assert!(matches!(refresh_outcome(200, serde_json::json!({"id_token": "x"})), Refreshed::Renewed(_)));
    }

    #[test]
    fn files_are_named_by_the_whole_origin() {
        assert_ne!(stem("https://furnish.myorg.tld"), stem("http://furnish.myorg.tld"));
        assert_eq!(stem("https://furnish.myorg.tld:8443"), "https_furnish.myorg.tld_8443");
        assert_eq!(stem("http://[::1]:80"), "http____1__80");
        assert!(!stem("https://../../x").contains('/'));
    }

    #[test]
    fn decoding_and_expiry() {
        assert_eq!(percent_decode("a%2Fb+c"), "a/b c");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
        let jwt = format!("x.{}.y", B64.encode(br#"{"exp":1234}"#));
        assert_eq!(jwt_exp(&jwt), Some(1234));
    }
}
