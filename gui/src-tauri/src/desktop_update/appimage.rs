//! Locate an AppImage through its live FUSE mount, even when another file has the same name.
//! No installed AppImage is run to discover its build.
use crate::build_info::{self, BuildInfo};
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn number(text: &str, field: &str) -> Option<u64> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| *name == field)?
        .1
        .trim()
        .parse()
        .ok()
}

pub(super) fn outer_image(proc: &Path, this: &Path, binary: &Path) -> Option<PathBuf> {
    // The Type 2 runtime hands AppRun fd 1023 referring to the mounted root.
    let root = fs::read_link(this.join("fd/1023")).ok()?;
    if binary.strip_prefix(&root).ok()? != Path::new("usr/bin/furnish-gui") {
        return None;
    }
    let id = number(&fs::read_to_string(this.join("fdinfo/1023")).ok()?, "mnt_id")?;
    let mounts = fs::read_to_string(this.join("mountinfo")).ok()?;
    let (fields, fsinfo) = mounts
        .lines()
        .filter_map(|l| l.split_once(" - "))
        .find(|(left, _)| left.split_ascii_whitespace().next().and_then(|n| n.parse::<u64>().ok()) == Some(id))?;
    let connection: u64 = fields
        .split_ascii_whitespace()
        .nth(2)?
        .strip_prefix("0:")?
        .parse()
        .ok()?;
    let mut device = fsinfo.split_ascii_whitespace();
    if !device.next()?.starts_with("fuse.") {
        return None;
    }
    // FUSE mountinfo uses octal escapes and generally reports just the image's basename.
    let name = device
        .next()?
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\");
    let name = Path::new(&name).file_name()?;
    for pid in fs::read_dir(proc).ok()?.flatten() {
        if !pid.file_name().as_encoded_bytes().iter().all(u8::is_ascii_digit) {
            continue;
        }
        let directory = pid.path();
        let Ok(exe) = fs::read_link(directory.join("exe")) else {
            continue;
        };
        if !exe.is_absolute() || exe.file_name() != Some(name) {
            continue;
        }
        let Ok(fds) = fs::read_dir(directory.join("fdinfo")) else {
            continue;
        };
        // Only the daemon serving our kernel connection identifies the outer AppImage. A
        // different AppImage with the same name is common. No process scan is done on polls.
        if fds
            .flatten()
            .any(|fd| fs::read_to_string(fd.path()).is_ok_and(|s| number(&s, "fuse_connection") == Some(connection)))
        {
            return Some(exe);
        }
    }
    None
}

/// In a Type 2 AppImage the SquashFS immediately follows the runtime's ELF section table.
/// Furnish packages target little-endian Linux. Verify the boundary before giving it to unsquashfs.
fn squashfs_offset(image: &Path) -> Option<u64> {
    let mut reader = fs::File::open(image).ok()?;
    let mut header = [0; 64];
    reader.read_exact(&mut header).ok()?;
    if &header[..4] != b"\x7fELF" || header[5] != 1 || &header[8..11] != b"AI\x02" {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([header[i], header[i + 1]]) as u64;
    let offset = match header[4] {
        1 => (u32::from_le_bytes(header[32..36].try_into().ok()?) as u64).checked_add(u16_at(46) * u16_at(48))?,
        2 => u64::from_le_bytes(header[40..48].try_into().ok()?).checked_add(u16_at(58) * u16_at(60))?,
        _ => return None,
    };
    reader.seek(SeekFrom::Start(offset)).ok()?;
    let mut magic = [0; 4];
    reader.read_exact(&mut magic).ok()?;
    (&magic == b"hsqs").then_some(offset)
}

struct Timed {
    pipe: UnixStream,
    until: Instant,
}
impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self
            .until
            .checked_duration_since(Instant::now())
            .filter(|t| !t.is_zero())
            .ok_or(io::ErrorKind::TimedOut)?;
        self.pipe.set_read_timeout(Some(left))?;
        self.pipe.read(buf)
    }
}

pub(super) fn read_build(image: &Path, tool: &Path, target: &str) -> Option<BuildInfo> {
    let offset = squashfs_offset(image)?;
    let (reader, writer) = UnixStream::pair().ok()?;
    let mut child = Command::new(tool)
        .env_clear()
        .env("LC_ALL", "C")
        .args([
            "-cat",
            "-processors",
            "1",
            "-data-queue",
            "8",
            "-frag-queue",
            "8",
            "-offset",
        ])
        .arg(offset.to_string())
        .arg(image)
        .arg("usr/bin/furnish-gui")
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stream = Timed {
        pipe: reader,
        until: Instant::now() + Duration::from_secs(10),
    };
    let info = build_info::read_records(stream.take(128 * 1024 * 1024), target);
    // Once the marker is read, or the work limit is reached, no more data is needed.
    let _ = child.kill();
    let _ = child.wait();
    info.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn outer_path_belongs_to_the_daemon_of_this_mount() {
        let temp = tempfile::tempdir().unwrap();
        let proc = temp.path();
        let this = proc.join("self");
        let mount = Path::new("/tmp/.mount_fixture");
        fs::create_dir_all(this.join("fd")).unwrap();
        fs::create_dir(this.join("fdinfo")).unwrap();
        symlink(mount, this.join("fd/1023")).unwrap();
        fs::write(this.join("fdinfo/1023"), "mnt_id:\t77\n").unwrap();
        fs::write(
            this.join("mountinfo"),
            r"77 1 0:444 / /tmp/.mount_fixture ro - fuse.AppImage Furnish\040Beta.AppImage ro",
        )
        .unwrap();
        for (pid, path, connection) in [
            ("100", "/somewhere/Furnish Beta.AppImage", 123),
            ("200", "/opt/Furnish Beta.AppImage", 444),
        ] {
            let process = proc.join(pid);
            fs::create_dir_all(process.join("fdinfo")).unwrap();
            symlink(path, process.join("exe")).unwrap();
            fs::write(process.join("fdinfo/7"), format!("fuse_connection:\t{connection}\n")).unwrap();
        }
        let binary = mount.join("usr/bin/furnish-gui");
        assert_eq!(
            outer_image(proc, &this, &binary),
            Some("/opt/Furnish Beta.AppImage".into())
        );
        assert_eq!(outer_image(proc, &this, &mount.join("usr/bin/other")), None);
        fs::remove_file(proc.join("200/fdinfo/7")).unwrap();
        assert_eq!(outer_image(proc, &this, &binary), None);
    }

    #[test]
    fn portable_probe_requires_a_type_two_image_with_a_valid_squashfs_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("portable");
        let mut header = vec![0u8; 96];
        header[..6].copy_from_slice(b"\x7fELF\x02\x01");
        header[8..11].copy_from_slice(b"AI\x02");
        header[40..48].copy_from_slice(&64u64.to_le_bytes());
        header[58..60].copy_from_slice(&8u16.to_le_bytes());
        header[60..62].copy_from_slice(&2u16.to_le_bytes());
        fs::write(&image, &header).unwrap();
        assert_eq!(squashfs_offset(&image), None);
        header[80..84].copy_from_slice(b"hsqs");
        fs::write(&image, &header).unwrap();
        assert_eq!(squashfs_offset(&image), Some(80));
        assert_eq!(read_build(&image, &temp.path().join("missing-tool"), "any"), None);
        header[8] = 0;
        fs::write(&image, &header).unwrap();
        assert_eq!(squashfs_offset(&image), None);
    }
}
