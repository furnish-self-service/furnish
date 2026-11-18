//! Content-addressed installer files for Furnish/Installer. Discovery and previews
//! only look up files; installation checks the pin even when reusing a cached file.
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use ureq::http::Uri;

struct Installer {
    uri: Uri,
    digest: String,
    extension: String,
}

impl Installer {
    fn parse(url: &str, sha256: &str) -> Result<Self> {
        ensure!(
            sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "installer sha256 must be exactly 64 hexadecimal digits"
        );
        ensure!(!url.contains('#') && !url.chars().any(char::is_control), "invalid installer URL");
        let uri: Uri = url.parse().context("invalid installer URL")?;
        let authority = uri.authority().filter(|a| !a.host().is_empty()).context("installer URL needs a host")?;
        ensure!(!authority.as_str().contains('@'), "installer URL must not contain credentials");
        ensure!(
            uri.scheme_str() == Some("https") || (uri.scheme_str() == Some("http") && matches!(authority.host(), "127.0.0.1" | "[::1]")),
            "installer URL must use HTTPS (HTTP is allowed on numeric loopback only)"
        );
        let extension = Path::new(uri.path().rsplit('/').next().unwrap_or_default())
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        ensure!(
            matches!(extension.as_str(), "pkg" | "dmg" | "msi" | "exe" | "deb" | "rpm"),
            "installer URL path must end in .pkg, .dmg, .msi, .exe, .deb or .rpm"
        );
        Ok(Self {
            uri,
            digest: sha256.to_ascii_lowercase(),
            extension,
        })
    }

    fn resolve(&self, data: &Path, system: &Path, fetch: bool) -> Result<PathBuf> {
        let name = format!("{}.{}", self.digest, self.extension);
        let dir = std::path::absolute(data)?.join("installer-downloads");
        let dest = dir.join(&name);
        let shared = std::path::absolute(system)?.join("installer-downloads");
        let cached = |root: &Path| -> Result<Option<PathBuf>> {
            let file = root.join(&name);
            if present(root, true)? && present(&file, false)? {
                let readable = fs::File::open(&file)?;
                if !fetch || hash_copy(readable, io::sink())? == self.digest {
                    return Ok(Some(file));
                }
            }
            Ok(None)
        };
        if let Some(file) = cached(&dir)? {
            return Ok(file);
        }
        // Users can reuse a service-owned pin if they can read it. An inaccessible
        // service cache leaves the user's own cache usable. The service itself uses
        // the system directory as its data directory and never looks up a user's pin.
        if dir != shared && let Ok(Some(file)) = cached(&shared) {
            return Ok(file);
        }
        if !fetch {
            return Ok(dest);
        }
        fs::create_dir_all(&dir)?;
        ensure!(present(&dir, true)?, "installer cache directory disappeared");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // The service exposes pins for users to inspect; only its account can write.
            fs::set_permissions(&dir, fs::Permissions::from_mode(if dir == shared { 0o755 } else { 0o700 }))?;
        }

        let https = self.uri.scheme_str() == Some("https");
        let mut response = crate::endpoint::http()
            .get(self.uri.clone())
            .config()
            .https_only(https)
            .max_redirects(if https { 10 } else { 0 })
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .build()
            .call()
            .context("downloading the pinned installer")?;
        ensure!(
            response.status().is_success(),
            "installer download returned HTTP {}",
            response.status()
        );
        // Publish a complete file in one rename. Concurrent attempts with this pin
        // can only publish the same bytes, and a failed transfer leaves no partial installer.
        let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
        let actual = hash_copy(response.body_mut().as_reader(), tmp.as_file_mut()).context("reading the installer download")?;
        ensure!(
            actual == self.digest,
            "installer SHA-256 mismatch: expected {}, got {actual}",
            self.digest
        );
        #[cfg(unix)]
        if dir == shared {
            use std::os::unix::fs::PermissionsExt;
            tmp.as_file().set_permissions(fs::Permissions::from_mode(0o644))?;
        }
        tmp.persist(&dest).context("publishing the verified installer")?;
        Ok(dest)
    }
}

fn present(path: &Path, directory: bool) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).with_context(|| format!("checking {}", path.display())),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        ensure!(
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
            "installer cache path is a reparse point: {}",
            path.display()
        );
    }
    ensure!(
        if directory { metadata.is_dir() } else { metadata.is_file() },
        "unexpected file type in installer cache: {}",
        path.display()
    );
    Ok(true)
}

fn hash_copy(mut reader: impl Read, mut writer: impl Write) -> Result<String> {
    let mut digest = Sha256::new();
    let mut buf = [0; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(digest.finalize().iter().map(|b| format!("{b:02x}")).collect());
        }
        digest.update(&buf[..n]);
        writer.write_all(&buf[..n])?;
    }
}

pub(crate) fn validate(url: &str, sha256: &str) -> Result<()> {
    Installer::parse(url, sha256).map(|_| ())
}

pub(crate) fn fetch(url: &str, sha256: &str) -> Result<PathBuf> {
    Installer::parse(url, sha256)?.resolve(&crate::catalog::data_dir(), &crate::catalog::system_dir(), true)
}

/// Some package managers need installer metadata to identify what to remove.
/// Verify that metadata before an uninstall status check can conclude the app is absent.
pub(crate) fn prepare_removal_identity(properties: &serde_json::Value) -> Result<()> {
    let Some(url) = properties["url"].as_str() else { return Ok(()) };
    let pin = properties["sha256"].as_str().context("installer URL needs a sha256 pin")?;
    let installer = Installer::parse(url, pin)?;
    let needs_file = match installer.extension.as_str() {
        "pkg" => properties["packageId"].as_str().unwrap_or_default().is_empty(),
        "msi" => properties["productCode"].as_str().unwrap_or_default().is_empty(),
        "deb" | "rpm" => true,
        // The DMG app name and the EXE display name must be declared by the collection.
        _ => false,
    };
    if needs_file {
        installer.resolve(&crate::catalog::data_dir(), &crate::catalog::system_dir(), true)?;
    }
    Ok(())
}

/// Used by the bundled Unix and Windows DSC resources, including when running under the GUI.
pub(crate) fn command(op: &str) -> Result<String> {
    let url = std::env::var("url").context("installer URL is missing")?;
    let digest = std::env::var("sha256").context("installer sha256 is missing")?;
    let path = match op {
        "fetch" => fetch(&url, &digest)?,
        "lookup" => Installer::parse(&url, &digest)?.resolve(&crate::catalog::data_dir(), &crate::catalog::system_dir(), false)?,
        _ => bail!("expected installer-download lookup or fetch"),
    };
    Ok(path.to_str().context("installer cache path is not UTF-8")?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        thread,
    };

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// One local connection, closed after returning the supplied HTTP response.
    fn serve(reply: &'static str) -> (String, thread::JoinHandle<()>) {
        serve_together(reply, 1)
    }

    fn serve_together(reply: &'static str, requests: usize) -> (String, thread::JoinHandle<()>) {
        let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        server.set_nonblocking(true).unwrap();
        let url = format!("http://{}/products/tool.MSI?v=4", server.local_addr().unwrap());
        let worker = thread::spawn(move || {
            // Wait for all requests before responding, so concurrent callers exercise publication.
            let until = std::time::Instant::now() + Duration::from_secs(5);
            let mut waiting = Vec::new();
            while waiting.len() < requests {
                match server.accept() {
                    Ok((conn, _)) => {
                        conn.set_nonblocking(false).unwrap();
                        conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        for line in BufReader::new(&conn).lines() {
                            if line.unwrap().is_empty() {
                                break;
                            }
                        }
                        waiting.push(conn);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock && std::time::Instant::now() < until => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("waiting for a fixture request: {e}"),
                }
            }
            for mut conn in waiting {
                conn.write_all(reply.as_bytes()).unwrap();
            }
        });
        (url, worker)
    }

    #[test]
    fn validates_url_and_pin_before_any_io() {
        for url in ["https://example.org/dist/a.pkg", "http://[::1]:9000/demo.DEB?a=b"] {
            assert!(validate(url, &ABC.to_uppercase()).is_ok());
        }
        for url in [
            "file:///tmp/a.msi",
            "http://example.org/a.pkg",
            "https://a:b@example.org/a.exe",
            "https://example.org/a.txt?name=a.msi",
            "https://example.org/a.msi#fragment",
            "https://example.org/a.msi/",
            "https://:443/a.msi",
        ] {
            assert!(validate(url, ABC).is_err(), "{url}");
        }
        for checksum in ["", "abcd", &"z".repeat(64)] {
            assert!(validate("https://example.org/a.msi", checksum).is_err());
        }
    }

    #[test]
    fn a_verified_download_is_reused_offline_and_visible_across_accounts() {
        let (url, server) = serve("HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc");
        let download = Installer::parse(&url, &ABC.to_uppercase()).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let system = tmp.path().join("system");
        let user = tmp.path().join("user");
        let cold = download.resolve(&system, &system, false).unwrap();
        assert_eq!(cold.file_name().unwrap().to_str().unwrap(), format!("{ABC}.msi"));
        assert!(!cold.exists() && !system.exists());
        assert_eq!(download.resolve(&system, &system, true).unwrap(), cold);
        server.join().unwrap();
        assert_eq!(fs::read(&cold).unwrap(), b"abc");
        assert_eq!(download.resolve(&user, &system, false).unwrap(), cold);
        assert_eq!(download.resolve(&user, &system, true).unwrap(), cold);
        assert!(!user.exists());
        fs::write(&cold, b"changed after publication").unwrap();
        assert!(download.resolve(&user, &system, true).is_err());
    }

    #[test]
    fn bad_bytes_interrupted_transfers_and_loopback_redirects_publish_nothing() {
        for (reply, why) in [
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad",
                "SHA-256 mismatch",
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nab",
                "reading the installer download",
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: http://example.invalid/abc.msi\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "302",
            ),
        ] {
            let (url, server) = serve(reply);
            let tmp = tempfile::tempdir().unwrap();
            let pinned = Installer::parse(&url, ABC).unwrap();
            let error = pinned.resolve(tmp.path(), tmp.path(), true).unwrap_err();
            assert!(format!("{error:#}").contains(why), "{error:#}");
            server.join().unwrap();
            assert_eq!(fs::read_dir(tmp.path().join("installer-downloads")).unwrap().count(), 0);
        }
    }

    #[test]
    fn declared_removal_identities_need_no_download() {
        for (ext, key, value) in [
            ("msi", "productCode", "{00000000-0000-0000-0000-000000000000}"),
            ("pkg", "packageId", "org.example.vendor"),
            ("dmg", "app", "Vendor.app"),
            ("exe", "displayName", "Vendor"),
        ] {
            let props = serde_json::json!({"url": format!("https://unused.invalid/vendor.{ext}"), "sha256": ABC, key: value});
            prepare_removal_identity(&props).unwrap();
        }
    }

    #[test]
    fn parallel_verified_publications_share_one_complete_installer() {
        let (url, server) = serve_together("HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc", 2);
        let root = tempfile::tempdir().unwrap();
        let pinned = Installer::parse(&url, ABC).unwrap();
        let resolve = || pinned.resolve(root.path(), root.path(), true).unwrap();
        thread::scope(|s| {
            let first = s.spawn(resolve);
            let second = s.spawn(resolve);
            assert_eq!(first.join().unwrap(), second.join().unwrap());
        });
        server.join().unwrap();
        let path = resolve();
        assert_eq!(fs::read(path).unwrap(), b"abc");
    }
}
