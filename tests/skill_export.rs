#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn command(root: &tempfile::TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_forkstr"));
    command.current_dir(root.path());
    command
}

fn files(root: &Path, directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fs::read_dir(directory)
        .unwrap()
        .flat_map(|entry| {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files(root, &path)
            } else {
                BTreeMap::from([(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                )])
            }
        })
        .collect()
}

#[test]
fn exports_the_bundled_skill_byte_for_byte() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("forkstr-skills");

    let output = command(&root)
        .args(["skill", "export"])
        .arg(&destination)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        files(&destination, &destination),
        files(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("skills")
                .as_path(),
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("skills")
                .as_path()
        )
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Exported Forkstr skill"));
    assert!(stdout.contains("No agent configuration was changed."));
}

#[test]
fn refuses_to_overwrite_an_existing_destination() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("forkstr-skills");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("keep"), "user content").unwrap();

    let output = command(&root)
        .args(["skill", "export"])
        .arg(&destination)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(destination.join("keep")).unwrap(),
        "user content"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Nothing was overwritten"));
}

#[test]
fn requires_an_existing_parent_directory() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("missing/forkstr-skills");

    let output = command(&root)
        .args(["skill", "export"])
        .arg(&destination)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(!destination.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("existing parent directory"));
}

#[test]
fn help_explains_offline_non_configuring_export() {
    let root = tempfile::tempdir().unwrap();

    let output = command(&root)
        .args(["skill", "export", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("offline"));
    assert!(stdout.contains("without configuring an agent"));
}
