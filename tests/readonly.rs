//! NFR-0002, "Read-only run": the tool works on a read-only checkout. A copy
//! of a fixture loses its write permission, and `check`, `graph` and `trace`
//! must give the same result on it as on a writable copy and write nothing.
//!
//! Unix only, because the test relies on permission bits. Root ignores them,
//! so the test skips (with a message) when it finds it can write anyway.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Adds (`writable`) or removes the write bits of everything under `path`.
fn chmod(path: &Path, writable: bool) {
    let mode = std::fs::metadata(path).unwrap().permissions().mode();
    let mode = if writable {
        mode | 0o200
    } else {
        mode & !0o222
    };
    if writable {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path).unwrap() {
            chmod(&entry.unwrap().path(), writable);
        }
    }
    if !writable {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

/// Every path under `path` with its size and, for a file, its modified time.
fn snapshot(path: &Path, into: &mut Vec<String>) {
    let meta = std::fs::metadata(path).unwrap();
    let modified = if meta.is_file() {
        meta.modified().ok()
    } else {
        None
    };
    into.push(format!("{} {} {modified:?}", path.display(), meta.len()));
    if meta.is_dir() {
        let mut names: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        names.sort();
        for name in names {
            snapshot(&name, into);
        }
    }
}

/// Makes the scratch tree writable again so it can be removed.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.exists() {
            chmod(&self.0, true);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn run(args: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modfence"))
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn a_read_only_checkout_gives_the_same_result_and_is_not_written() {
    let root = std::env::temp_dir().join(format!("modfence-readonly-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let scratch = Scratch(root.clone());
    let commands: [&[&str]; 6] = [
        &["check"],
        &["check", "--format", "sarif"],
        &["graph"],
        &["graph", "--format", "json"],
        &["trace", "kind=external"],
        &["trace", "kind=external", "--format", "dot"],
    ];
    for fixture in ["app", "clean"] {
        let writable = root.join("rw").join(fixture);
        let locked = root.join("ro").join(fixture);
        copy(&fixtures().join(fixture), &writable);
        copy(&fixtures().join(fixture), &locked);
        chmod(&locked, false);
        // Root (and a filesystem that ignores the bits) can write anyway.
        let probe = locked.join("probe");
        if std::fs::File::create(&probe).is_ok() {
            std::fs::remove_file(&probe).unwrap();
            eprintln!("skipped: permission bits are not enforced here (running as root?)");
            return;
        }
        let mut before = Vec::new();
        snapshot(&locked, &mut before);
        for args in commands {
            let expected = run(args, &writable);
            let actual = run(args, &locked);
            assert!(
                expected.status.code().is_some_and(|code| code <= 1),
                "{fixture} {args:?}: {}",
                String::from_utf8_lossy(&expected.stderr)
            );
            assert_eq!(
                actual.status.code(),
                expected.status.code(),
                "{fixture} {args:?}"
            );
            assert_eq!(actual.stdout, expected.stdout, "{fixture} {args:?}");
            assert_eq!(actual.stderr, expected.stderr, "{fixture} {args:?}");
            // Not vacuous: there is a result to compare.
            assert!(!expected.stdout.is_empty(), "{fixture} {args:?}");
        }
        // Nothing was created, changed or removed, `.modfence/` included.
        let mut after = Vec::new();
        snapshot(&locked, &mut after);
        assert_eq!(after, before, "{fixture}");
        assert!(!locked.join(".modfence").exists(), "{fixture}");
    }
    drop(scratch);
}
