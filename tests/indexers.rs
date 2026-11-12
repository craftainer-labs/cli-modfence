//! `--run-indexers` (NC6d, NFR-0002): which indexers are planned, how they are
//! found and started, and that nothing runs without the flag. The indexers are
//! fake executables on `PATH` that record how they were called.

#[cfg(windows)]
mod windows {
    use std::process::Command;

    #[test]
    fn run_indexers_is_not_supported_and_warns() {
        let dir = std::env::temp_dir().join(format!("modfence-win-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_modfence"))
            .args(["check", "--run-indexers"])
            .current_dir(&dir)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(err.contains("--run-indexers is not supported"), "{err}");
        assert_eq!(output.status.code(), Some(0));
    }
}

#[cfg(unix)]
mod unix {
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use modfence::extract::Lang;
    use modfence::scip::run::{command_line, plan, project_name, resolve, spawn};
    use protobuf::{EnumOrUnknown, Message, MessageField};
    use scip::types::{Document, Index, Metadata, Occurrence, PositionEncoding};

    const LIB: &str = "def run():\n    pass\n";
    const APP: &str = "from lib import run\n\n\ndef main():\n    run()\n";
    const RUN: &str = "scip-python python lib 0.1 `lib`/run().";
    const CONFIG: &str = "version = 1\n\n[[rule]]\nid = \"no-calls\"\nfrom = { path = \"app.py\" }\nto = { path = \"lib.py\" }\neffect = \"forbid\"\nkinds = [\"calls\"]\n";
    const FAKE: &str = r#"#!/bin/sh
{ printf 'CALL %s\n' "$0"; for a in "$@"; do printf 'ARG %s\n' "$a"; done; } >> "$MODFENCE_RECORD"
out=
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then out="$2"; fi
  shift
done
name=$(basename "$0")
if [ -n "$out" ] && [ -f "$MODFENCE_INDEXES/$name.scip" ]; then
  cp "$MODFENCE_INDEXES/$name.scip" "$out"
fi
exit "${MODFENCE_EXIT:-0}"
"#;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("modfence-idx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base.canonicalize().unwrap()
    }

    fn write(path: &Path, text: &[u8], mode: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// An index whose `lib.py` defines `run` and whose `app.py` calls it.
    fn resolving_index() -> Vec<u8> {
        let mut definition = Occurrence::new();
        definition.range = vec![0, 4, 7];
        RUN.clone_into(&mut definition.symbol);
        definition.symbol_roles = 1;
        let mut call = Occurrence::new();
        call.range = vec![4, 4, 7];
        RUN.clone_into(&mut call.symbol);
        let document = |path: &str, occurrence: Occurrence| {
            let mut document = Document::new();
            path.clone_into(&mut document.relative_path);
            document.position_encoding =
                EnumOrUnknown::new(PositionEncoding::UTF8CodeUnitOffsetFromLineStart);
            document.occurrences = vec![occurrence];
            document
        };
        let mut index = Index::new();
        index.metadata = MessageField::some(Metadata::new());
        index.documents = vec![document("lib.py", definition), document("app.py", call)];
        index.write_to_bytes().unwrap()
    }

    /// A project, a directory of fake indexers and one of the indexes they
    /// write; `fakes` names the executables to create.
    struct Setup {
        project: PathBuf,
        bin: PathBuf,
        indexes: PathBuf,
        record: PathBuf,
    }

    fn setup(name: &str, fakes: &[&str]) -> Setup {
        let base = scratch(name);
        let project = base.join("project");
        write(&project.join("modfence.toml"), CONFIG.as_bytes(), 0o644);
        write(&project.join("lib.py"), LIB.as_bytes(), 0o644);
        write(&project.join("app.py"), APP.as_bytes(), 0o644);
        let (bin, indexes) = (base.join("bin"), base.join("indexes"));
        for fake in fakes {
            write(&bin.join(fake), FAKE.as_bytes(), 0o755);
        }
        write(&indexes.join("scip-python.scip"), &resolving_index(), 0o644);
        let record = base.join("record");
        Setup {
            project,
            bin,
            indexes,
            record,
        }
    }

    impl Setup {
        /// Runs the binary in the project with only the fakes (and the system
        /// tools they use) on `PATH`.
        fn run(&self, args: &[&str]) -> Output {
            self.run_with(&format!("{}:/usr/bin:/bin", self.bin.display()), &[], args)
        }

        fn run_with(&self, path: &str, env: &[(&str, &str)], args: &[&str]) -> Output {
            let mut command = Command::new(env!("CARGO_BIN_EXE_modfence"));
            command
                .args(args)
                .current_dir(&self.project)
                .env_clear()
                .env("PATH", path)
                .env("MODFENCE_RECORD", &self.record)
                .env("MODFENCE_INDEXES", &self.indexes);
            for (key, value) in env {
                command.env(key, value);
            }
            command.output().unwrap()
        }

        /// The calls the fakes recorded: the program, then its arguments.
        fn calls(&self) -> Vec<(String, Vec<String>)> {
            let text = std::fs::read_to_string(&self.record).unwrap_or_default();
            let mut calls: Vec<(String, Vec<String>)> = Vec::new();
            for line in text.lines() {
                if let Some(program) = line.strip_prefix("CALL ") {
                    calls.push((program.to_owned(), Vec::new()));
                } else if let Some(argument) = line.strip_prefix("ARG ") {
                    calls.last_mut().unwrap().1.push(argument.to_owned());
                }
            }
            calls
        }

        /// Every file below the project, relative to it.
        fn files(&self) -> BTreeSet<String> {
            fn walk(dir: &Path, root: &Path, found: &mut BTreeSet<String>) {
                for entry in std::fs::read_dir(dir).unwrap().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path, root, found);
                    } else {
                        let relative = path.strip_prefix(root).unwrap();
                        found.insert(relative.to_string_lossy().into_owned());
                    }
                }
            }
            let mut found = BTreeSet::new();
            walk(&self.project, &self.project, &mut found);
            found
        }
    }

    fn err(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn code(output: &Output) -> i32 {
        output.status.code().unwrap()
    }

    #[test]
    fn nothing_is_started_without_the_flag() {
        let setup = setup("none", &["scip-python", "scip-typescript", "rust-analyzer"]);
        let index = setup.indexes.join("scip-python.scip");
        let index = index.to_str().unwrap();
        let runs: [&[&str]; 7] = [
            &["check"],
            &["graph"],
            &["graph", "--format", "json"],
            &["trace", "path=lib.py"],
            &["check", "--precise"],
            &["check", "--scip", index],
            &["graph", "--precise", "--format", "dot"],
        ];
        for args in runs {
            let output = setup.run(args);
            assert!(code(&output) <= 1, "{args:?}: {}", err(&output));
        }
        assert_eq!(setup.calls(), Vec::<(String, Vec<String>)>::new());
        assert!(!setup.record.exists());
    }

    #[test]
    fn only_the_fakes_for_the_languages_analysed_run() {
        let setup = setup("run", &["scip-python", "scip-typescript", "rust-analyzer"]);
        write(&setup.project.join("util.ts"), b"export function f() {}\n", 0o644);
        write(&setup.indexes.join("scip-typescript.scip"), &Index::new().write_to_bytes().unwrap(), 0o644);
        let before = setup.files();
        let output = setup.run(&["check", "--run-indexers"]);
        let stderr = err(&output);
        // The python index resolves the call, so the rule fires.
        assert_eq!(code(&output), 1, "{stderr}");
        assert!(!stderr.contains("were not checked"), "{stderr}");
        let project = setup.project.display();
        let out = format!("{project}/.modfence/scip");
        let name = setup.project.file_name().unwrap().to_str().unwrap();
        let python = format!(
            "{} index . --project-name={name} --output {out}/python.scip",
            setup.bin.join("scip-python").display()
        );
        let typescript = format!(
            "{} index --infer-tsconfig --output {out}/typescript.scip",
            setup.bin.join("scip-typescript").display()
        );
        let ran: Vec<String> = setup
            .calls()
            .into_iter()
            .map(|(program, args)| format!("{program} {}", args.join(" ")))
            .collect();
        assert_eq!(ran, [python.clone(), typescript.clone()]);
        // What was printed is what ran.
        assert!(stderr.contains(&format!("note: running {python}\n")), "{stderr}");
        assert!(stderr.contains(&format!("note: running {typescript}\n")), "{stderr}");
        // modfence wrote only below `.modfence/`.
        let added: Vec<String> = setup.files().difference(&before).cloned().collect();
        let expected = [".modfence/scip/python.scip", ".modfence/scip/typescript.scip"];
        assert_eq!(added, expected);
    }

    #[test]
    fn missing_indexers_are_a_warning_and_nothing_is_downloaded() {
        let setup = setup("missing", &[]);
        let before = setup.files();
        let output = setup.run(&["check", "--run-indexers"]);
        let stderr = err(&output);
        assert_eq!(code(&output), 0, "{stderr}");
        assert!(stderr.contains("scip-python not found on PATH; py stays on tree-sitter"));
        assert!(stderr.contains("no indexer produced an index"), "{stderr}");
        assert!(stderr.contains("call edges were not checked"), "{stderr}");
        let added: Vec<String> = setup.files().difference(&before).cloned().collect();
        assert_eq!(added, Vec::<String>::new());
    }

    #[test]
    fn a_failing_indexer_leaves_its_language_on_tree_sitter() {
        let setup = setup("failing", &["scip-python"]);
        let failed = setup.run_with(
            &format!("{}:/usr/bin:/bin", setup.bin.display()),
            &[("MODFENCE_EXIT", "3")],
            &["check", "--run-indexers"],
        );
        assert_eq!(code(&failed), 0, "{}", err(&failed));
        assert!(err(&failed).contains("scip-python: exit status: 3, or no index"));
        // Exits 0 but writes nothing: the same warning.
        std::fs::remove_file(setup.indexes.join("scip-python.scip")).unwrap();
        let empty = setup.run(&["check", "--run-indexers"]);
        assert_eq!(code(&empty), 0, "{}", err(&empty));
        assert!(err(&empty).contains("scip-python: exit status: 0, or no index"));
    }

    #[test]
    fn a_bad_index_from_an_indexer_is_skipped_and_an_old_one_is_not_read() {
        let setup = setup("bad", &["scip-python"]);
        write(&setup.indexes.join("scip-python.scip"), &[0xff; 8], 0o644);
        let output = setup.run(&["check", "--run-indexers"]);
        assert_eq!(code(&output), 0, "{}", err(&output));
        assert!(err(&output).contains("python.scip: not a SCIP index"), "{}", err(&output));
        assert!(err(&output).contains("skipped"));
        // An index from an earlier run is removed before the indexer runs.
        let old = setup.project.join(".modfence/scip/python.scip");
        write(&old, &[0xff; 8], 0o644);
        std::fs::remove_file(setup.indexes.join("scip-python.scip")).unwrap();
        let output = setup.run_with(
            &format!("{}:/usr/bin:/bin", setup.bin.display()),
            &[("MODFENCE_EXIT", "3")],
            &["check", "--run-indexers"],
        );
        assert_eq!(code(&output), 0, "{}", err(&output));
        assert!(!err(&output).contains("not a SCIP index"), "{}", err(&output));
        assert!(!old.exists());
    }

    #[test]
    fn path_is_searched_by_modfence_in_absolute_entries_only() {
        let setup = setup("relative", &[]);
        // A program in the checkout, named by a relative `PATH` entry.
        write(&setup.project.join("bin/scip-python"), FAKE.as_bytes(), 0o755);
        let output = setup.run_with("bin:/usr/bin:/bin", &[], &["check", "--run-indexers"]);
        assert_eq!(code(&output), 0, "{}", err(&output));
        assert!(err(&output).contains("scip-python not found on PATH"), "{}", err(&output));
        assert_eq!(setup.calls(), Vec::<(String, Vec<String>)>::new());
        // One that cannot be started is a warning too.
        write(&setup.bin.join("scip-python"), b"#!/nonexistent/interpreter\n", 0o755);
        let output = setup.run(&["check", "--run-indexers"]);
        assert_eq!(code(&output), 0, "{}", err(&output));
        assert!(err(&output).contains("scip-python cannot start:"), "{}", err(&output));
    }

    #[test]
    fn arguments_reach_the_indexer_untouched_and_no_shell_is_involved() {
        let base = scratch("shell");
        let project = base.join("we ird;touch pwned;'q");
        write(&project.join("modfence.toml"), CONFIG.as_bytes(), 0o644);
        write(&project.join("lib.py"), LIB.as_bytes(), 0o644);
        write(&project.join("app.py"), APP.as_bytes(), 0o644);
        write(&base.join("bin/scip-python"), FAKE.as_bytes(), 0o755);
        let setup = Setup {
            project: project.clone(),
            bin: base.join("bin"),
            indexes: base.join("indexes"),
            record: base.join("record"),
        };
        let output = setup.run(&["check", "--run-indexers"]);
        assert_eq!(code(&output), 0, "{}", err(&output));
        let calls = setup.calls();
        let output_arg = format!("{}/.modfence/scip/python.scip", project.display());
        assert_eq!(calls[0].1[2], "--project-name=we_ird_touch_pwned__q");
        assert_eq!(calls[0].1[4], output_arg);
        assert!(!project.join("pwned").exists());
        let quoted = format!("'{}'", output_arg.replace('\'', "'\\''"));
        assert!(err(&output).contains(&quoted), "{}", err(&output));
    }

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn plans_follow_adr_0003() {
        let all: BTreeSet<Lang> = [Lang::Rs, Lang::Py, Lang::Ts].into();
        let (root, out) = (Path::new("/r"), Path::new("/r/.modfence/scip"));
        let plans = plan(&all, root, out, "n", false);
        assert_eq!(plans[0].program, "scip-python");
        let python = ["index", ".", "--project-name=n", "--output"];
        assert_eq!(plans[0].args, os(&[&python[..], &["/r/.modfence/scip/python.scip"]].concat()));
        assert_eq!(plans[1].program, "scip-typescript");
        let typescript = ["index", "--infer-tsconfig", "--output"];
        let output = ["/r/.modfence/scip/typescript.scip"];
        assert_eq!(plans[1].args, os(&[&typescript[..], &output].concat()));
        assert_eq!(plans[2].program, "rust-analyzer");
        let rust = ["scip", "/r", "--output", "/r/.modfence/scip/rust.scip"];
        assert_eq!(plans[2].args, os(&rust));
        let only_ts: BTreeSet<Lang> = [Lang::Ts].into();
        let with_config = plan(&only_ts, root, out, "n", true);
        assert_eq!(with_config[0].args, os(&["index", "--output", "/r/.modfence/scip/typescript.scip"]));
        assert_eq!(plan(&BTreeSet::new(), root, out, "n", true).len(), 0);
    }

    #[test]
    fn the_project_name_is_safe_to_print() {
        let base = scratch("name");
        let named = |dir: &str, pyproject: Option<&str>| {
            let root = base.join(dir);
            std::fs::create_dir_all(&root).unwrap();
            if let Some(text) = pyproject {
                std::fs::write(root.join("pyproject.toml"), text).unwrap();
            }
            project_name(&root)
        };
        assert_eq!(named("a", Some("[project]\nname = \"my-pkg_1.0\"\n")), "my-pkg_1.0");
        assert_eq!(named("b", None), "b");
        assert_eq!(named("c d", None), "c_d");
        assert_eq!(named("e", Some("[project]\nname = \"bad name\\n\"\n")), "e");
        assert_eq!(named("f", Some("[project]\nname = \"-x\"\n")), "f");
        assert_eq!(named("g", Some("not toml [")), "g");
        assert_eq!(named("h", Some("[project]\nname = 3\n")), "h");
        assert_eq!(named("-i", None), "project");
        assert_eq!(project_name(Path::new("/")), "project");
    }

    #[test]
    fn resolution_takes_the_first_executable_file_in_an_absolute_entry() {
        let base = scratch("resolve");
        write(&base.join("a/prog"), b"#!/bin/sh\n", 0o644);
        write(&base.join("b/prog"), b"#!/bin/sh\n", 0o755);
        write(&base.join("c/prog"), b"#!/bin/sh\n", 0o755);
        std::fs::create_dir_all(base.join("d/prog")).unwrap();
        let join = |dirs: &[&str]| {
            let entries = dirs.iter().map(|d| base.join(d));
            std::env::join_paths(entries).unwrap()
        };
        assert_eq!(resolve("prog", &join(&["a", "b", "c"])), Some(base.join("b/prog")));
        assert_eq!(resolve("prog", &join(&["d", "c"])), Some(base.join("c/prog")));
        assert_eq!(resolve("prog", &join(&["a", "d", "nowhere"])), None);
        let relative = std::env::join_paths(["", "relative/dir", "."]).unwrap();
        assert_eq!(resolve("prog", &relative), None);
    }

    #[test]
    fn command_lines_quote_what_a_terminal_would_misread() {
        let args: Vec<OsString> = ["plain-1.2", "a b", "it's", "", "x\ny", "$(x);"]
            .iter()
            .map(OsString::from)
            .collect();
        let line = command_line(Path::new("/bin/prog"), &args);
        let expected = r"/bin/prog plain-1.2 'a b' 'it'\''s' '' 'x\ny' '$(x);'";
        assert_eq!(line, expected);
    }

    #[test]
    fn spawn_waits_with_the_environment_and_reports_failures() {
        let sh = Path::new("/bin/sh");
        let args: Vec<OsString> = ["-c", "exit 3"].iter().map(OsString::from).collect();
        let status = spawn(sh, &args, Path::new("/")).unwrap();
        assert_eq!(status.code(), Some(3));
        assert!(spawn(Path::new("/nonexistent/program"), &[], Path::new("/")).is_err());
    }
}
