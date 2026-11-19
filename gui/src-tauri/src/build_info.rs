//! Running desktop identity, shared by About, headless inspection and package replacement checks.
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub version: String,
    pub revision: Option<String>,
    pub target: String,
    pub source_date_epoch: Option<u64>,
}

macro_rules! marker {
    () => {
        "\0FURNISH-DESKTOP-BUILD-v1\x1e"
    };
}
const MARKER: &[u8] = marker!().as_bytes();
const CHUNK: usize = 64 * 1024;
const MAX_JSON: usize = 1024;
const _: () = assert!(
    env!("FURNISH_DESKTOP_BUILD_JSON").len() <= MAX_JSON,
    "desktop build identity exceeds its record size"
);
static RECORD: &str = concat!(marker!(), env!("FURNISH_DESKTOP_BUILD_JSON"), "\0");

pub fn current() -> BuildInfo {
    // Keep the complete, searchable record in optimized/stripped desktop executables.
    let record = std::hint::black_box(RECORD.as_bytes());
    serde_json::from_slice(&record[MARKER.len()..record.len() - 1])
        .expect("embedded desktop build identity")
}

pub fn read_binary(path: &Path) -> io::Result<Option<BuildInfo>> {
    read_records(File::open(path)?, &current().target)
}

pub(super) fn read_records(mut input: impl Read, target: &str) -> io::Result<Option<BuildInfo>> {
    let finder = memchr::memmem::Finder::new(MARKER);
    // Overlap accommodates a complete record straddling two reads. A fat Mach-O may
    // contain multiple valid identities. Prefer the slice for the running architecture.
    let overlap = MARKER.len() + MAX_JSON;
    let mut buf = vec![0u8; CHUNK + overlap];
    let mut kept = 0;
    let mut other_target = None;
    loop {
        let read = input.read(&mut buf[kept..kept + CHUNK])?;
        if read == 0 {
            return Ok(other_target);
        }
        let bytes = &buf[..kept + read];
        for pos in finder.find_iter(bytes) {
            let json = &bytes[pos + MARKER.len()..];
            let Some(end) = memchr::memchr(0, &json[..json.len().min(MAX_JSON + 1)]) else {
                continue;
            };
            let Ok(info) = serde_json::from_slice::<BuildInfo>(&json[..end]) else {
                continue;
            };
            if furnish::selfupdate::semver(&info.version).is_none()
                || info.target.is_empty()
                || !info
                    .target
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
                || info.revision.as_deref().is_some_and(|r| {
                    !matches!(r.len(), 40 | 64) || !r.bytes().all(|b| b.is_ascii_hexdigit())
                })
            {
                continue;
            }
            if info.target == target {
                return Ok(Some(info));
            }
            other_target.get_or_insert(info);
        }
        let end = bytes.len();
        kept = end.min(overlap);
        buf.copy_within(end - kept..end, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(value: &BuildInfo) -> Vec<u8> {
        [MARKER, &serde_json::to_vec(value).unwrap(), &[0]].concat()
    }

    #[test]
    fn running_executable_contains_the_same_build_record() {
        assert_eq!(
            read_binary(&std::env::current_exe().unwrap()).unwrap(),
            Some(current())
        );
    }

    #[test]
    fn skips_invalid_markers_and_reads_split_records_and_fat_binaries() {
        let native = current();
        let foreign = BuildInfo {
            target: "other-target".into(),
            ..native.clone()
        };
        let mut bytes = record(&foreign);
        bytes.extend_from_slice(MARKER);
        bytes.extend_from_slice(b"bad json\0");
        bytes.resize(CHUNK - MARKER.len() / 2, 42);
        bytes.extend(record(&native));
        assert_eq!(
            read_records(bytes.as_slice(), &native.target).unwrap(),
            Some(native)
        );
        assert_eq!(read_records(&b"ordinary executable"[..], "").unwrap(), None);
    }
}
