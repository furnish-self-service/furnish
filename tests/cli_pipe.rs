use std::{fs, io, process::Command};

#[test]
fn closing_stdout_preserves_the_command_result() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let data = root.join("data");
    fs::create_dir(&data).unwrap();
    fs::write(data.join("settings.json"), r#"{"selfUpdate":"off"}"#).unwrap();
    let policy = root.join("policy.json");
    fs::write(&policy, "{}").unwrap();

    let run = |subcommand: &str| {
        // Close the reader before starting Furnish so even the first line hits
        // a broken pipe, regardless of pipe capacity and process scheduling.
        let (reader, writer) = io::pipe().unwrap();
        drop(reader);
        Command::new(env!("CARGO_BIN_EXE_furnish"))
            .arg(subcommand)
            .current_dir(root)
            .env("FURNISH_DATA_DIR", &data)
            .env("FURNISH_SYSTEM_DIR", root.join("system"))
            .env("FURNISH_POLICY_FILE", &policy)
            .stdout(writer)
            .output()
            .unwrap()
    };

    for cmd in ["list", "reports", "settings"] {
        let out = run(cmd);
        assert!(out.status.success(), "{cmd} exited {}: {}", out.status, String::from_utf8_lossy(&out.stderr));
        assert!(out.stderr.is_empty(), "{cmd}: {}", String::from_utf8_lossy(&out.stderr));
    }

    // Checking a collection with no apps prints findings before returning an error.
    let out = run("check");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("the collection has errors"));
}
