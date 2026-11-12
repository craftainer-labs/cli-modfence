//! NFR-0002: the binary works in a sandbox with no network. Every command and
//! output format runs on the fixture repositories inside a new network
//! namespace (`unshare`), and must give the same result as outside it.
//!
//! The sandbox is required on Linux, where CI runs; set `MODFENCE_SKIP_SANDBOX`
//! to skip these tests on a machine that cannot create one.

#[cfg(target_os = "linux")]
mod linux {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    /// The command prefix that gives a process a network namespace of its
    /// own: `unshare` with a user namespace when unprivileged users may create
    /// one, plain `unshare` as root, or through passwordless `sudo` (the
    /// devcontainer's `vscode` user has it).
    fn sandbox() -> Option<&'static [&'static str]> {
        let candidates: [&[&str]; 3] = [
            &["unshare", "-rn"],
            &["unshare", "-n"],
            &["sudo", "-n", "unshare", "-n"],
        ];
        candidates.into_iter().find(|prefix| {
            Command::new(prefix[0])
                .args(&prefix[1..])
                .arg("true")
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli")
    }

    fn outside(args: &[String]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_modfence"))
            .args(args)
            .output()
            .unwrap()
    }

    /// The modfence binary under `prefix`, in a neutral working directory. The
    /// coverage profile location is passed through `env`, because `sudo`
    /// would drop it and the profile would land in the working directory.
    fn sandboxed(prefix: &[&str]) -> Command {
        let mut command = Command::new(prefix[0]);
        command.args(&prefix[1..]).current_dir(std::env::temp_dir());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            let mut setting = std::ffi::OsString::from("LLVM_PROFILE_FILE=");
            setting.push(profile);
            command.arg("env").arg(setting);
        }
        command.arg(env!("CARGO_BIN_EXE_modfence"));
        command
    }

    fn inside(prefix: &[&str], args: &[String]) -> Output {
        sandboxed(prefix).args(args).output().unwrap()
    }

    /// Every command with every output format, on one fixture.
    fn commands(fixture: &str) -> Vec<Vec<String>> {
        let config = fixtures().join(fixture).join("modfence.toml");
        let config = config.to_str().unwrap().to_owned();
        let mut all: Vec<Vec<&str>> = Vec::new();
        for format in ["text", "sarif", "github"] {
            all.push(vec!["check", "--format", format]);
        }
        for format in ["text", "json", "dot", "mermaid", "graphml", "html"] {
            all.push(vec!["graph", "--format", format]);
            all.push(vec!["trace", "--format", format, "kind=external"]);
        }
        all.push(vec!["graph", "--precise"]);
        all.into_iter()
            .map(|args| {
                let mut owned: Vec<String> = args.into_iter().map(str::to_owned).collect();
                owned.extend(["--config".to_owned(), config.clone()]);
                owned
            })
            .collect()
    }

    #[test]
    fn every_command_gives_the_same_result_without_a_network() {
        let Some(prefix) = sandbox() else {
            assert!(
                std::env::var_os("MODFENCE_SKIP_SANDBOX").is_some(),
                "no network namespace could be created (tried unshare -rn, unshare -n and \
                 sudo -n unshare -n); set MODFENCE_SKIP_SANDBOX to skip this test"
            );
            return;
        };
        // The sandbox really has no network: only the loopback device is left.
        let listed = Command::new(prefix[0])
            .args(&prefix[1..])
            .args(["cat", "/proc/net/dev"])
            .output()
            .unwrap();
        let listing = String::from_utf8_lossy(&listed.stdout);
        let interfaces: Vec<&str> = listing
            .lines()
            .skip(2)
            .filter_map(|line| line.split(':').next())
            .map(str::trim)
            .collect();
        assert!(
            interfaces.iter().all(|name| *name == "lo"),
            "{interfaces:?}"
        );
        for fixture in ["app", "clean"] {
            for args in commands(fixture) {
                let normal = outside(&args);
                let sandboxed = inside(prefix, &args);
                assert_eq!(sandboxed.status.code(), normal.status.code(), "{args:?}");
                assert_eq!(sandboxed.stdout, normal.stdout, "{args:?}");
                assert!(
                    normal.status.code().is_some_and(|code| code <= 1),
                    "{args:?}"
                );
            }
        }
    }

    #[test]
    fn reading_an_index_needs_no_network_either() {
        let Some(prefix) = sandbox() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("modfence-offline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
        std::fs::write(dir.join("empty.scip"), b"").unwrap();
        let args: Vec<String> = ["check", "--scip", "empty.scip"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let mut command = sandboxed(prefix);
        command.args(&args).current_dir(&dir);
        let output = command.output().unwrap();
        let err = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(0), "{err}");
        assert!(err.contains("no usable SCIP documents"), "{err}");
    }
}
