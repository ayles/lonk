#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct SourceFile(PathBuf);

impl SourceFile {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "lonk-cli-test-{}-{}.lonk",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(source.as_bytes()).unwrap();
        Self(path)
    }

    fn run(&self, command: &str, rest: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_lonk"))
            .arg(command)
            .arg(&self.0)
            .args(rest)
            .output()
            .unwrap()
    }
}

impl Drop for SourceFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn run(args: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lonk"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).unwrap()
}

#[test]
fn check_reports_a_type_without_evaluating_even_the_selected_value() {
    let source = SourceFile::new("{good = 42; bad = 1 / 0}");
    let checked = source.run("check", &["bad"]);
    assert!(checked.status.success(), "{}", stderr(&checked));
    assert_eq!(stdout(&checked), "int\n");
    assert!(checked.stderr.is_empty());
    assert!(source.run("check", &[]).status.success());

    let evaluated = source.run("eval", &["bad"]);
    assert_eq!(evaluated.status.code(), Some(1));
    assert!(stderr(&evaluated).contains("division"));
    assert!(evaluated.stdout.is_empty());
}

#[test]
fn check_rejects_static_errors_in_unselected_fields() {
    let source = SourceFile::new("{good = 42; bad = 1 + true}");
    let output = source.run("check", &["good"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains(&format!("{}:1:", source.0.display())));
}

#[test]
fn eval_selects_nested_fields_and_array_elements_lazily() {
    let source = SourceFile::new("{packages = [{name = \"hello\"}]; bad = 1 / 0}");
    let output = source.run("eval", &["packages.0.name"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "\"hello\"\n");
    let checked = source.run("check", &["packages.0.name"]);
    assert!(checked.status.success(), "{}", stderr(&checked));
    assert_eq!(stdout(&checked), "str\n");

    for command in ["eval", "check"] {
        let missing = source.run(command, &["missing"]);
        assert_eq!(missing.status.code(), Some(1));
        assert!(stderr(&missing).contains(&format!("{}:1:", source.0.display())));
    }
    let scalar = SourceFile::new("1 + 2 * 3");
    assert_eq!(stdout(&scalar.run("eval", &[])), "7\n");
}

#[test]
fn syntax_and_runtime_errors_include_filename_line_and_column() {
    let source = SourceFile::new("let x = 1;\nx / 0\n");
    let output = source.run("eval", &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains(&format!("{}:2:1:", source.0.display())));

    fs::write(&source.0, "let x = 1;\nlet = broken\n").unwrap();
    for command in ["eval", "check", "fmt", "ast"] {
        let output = source.run(command, &[]);
        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).contains(&format!("{}:2:", source.0.display())));
    }
}

#[test]
fn fmt_is_syntax_only_idempotent_and_does_not_modify_input() {
    let original = "// keep me\n{value=unknown;bad=1/0}";
    let source = SourceFile::new(original);
    let formatted = source.run("fmt", &[]);
    assert!(formatted.status.success(), "{}", stderr(&formatted));
    assert!(stdout(&formatted).contains("// keep me"));
    assert_eq!(fs::read_to_string(&source.0).unwrap(), original);

    let check = source.run("fmt", &["--check"]);
    assert_eq!(check.status.code(), Some(1));
    assert!(check.stdout.is_empty());
    assert!(stderr(&check).contains("formatting differs"));

    fs::write(&source.0, &formatted.stdout).unwrap();
    assert_eq!(source.run("fmt", &[]).stdout, formatted.stdout);
    let check = run(&[
        OsStr::new("fmt"),
        OsStr::new("--check"),
        source.0.as_os_str(),
    ]);
    assert!(check.status.success(), "{}", stderr(&check));
    assert!(check.stdout.is_empty());
    assert!(check.stderr.is_empty());
}

#[test]
fn usage_rejects_unknown_flags_extra_positionals_and_empty_path_segments() {
    let source = SourceFile::new("{a = 1}");
    let file = source.0.as_os_str();
    let cases: Vec<Vec<&OsStr>> = vec![
        vec![],
        vec!["wat".as_ref()],
        vec!["--help".as_ref(), "extra".as_ref()],
        vec!["--version".as_ref(), "extra".as_ref()],
        vec!["eval".as_ref()],
        vec!["check".as_ref()],
        vec!["fmt".as_ref()],
        vec!["ast".as_ref()],
        vec!["eval".as_ref(), file, "a".as_ref(), "extra".as_ref()],
        vec!["check".as_ref(), file, "a".as_ref(), "extra".as_ref()],
        vec!["fmt".as_ref(), file, "extra".as_ref()],
        vec!["ast".as_ref(), file, "extra".as_ref()],
        vec!["eval".as_ref(), file, "--check".as_ref()],
        vec!["check".as_ref(), file, "--check".as_ref()],
        vec!["fmt".as_ref(), file, "--check".as_ref(), "--check".as_ref()],
        vec!["fmt".as_ref(), file, "--wat".as_ref()],
        vec!["eval".as_ref(), file, "-x".as_ref()],
        vec!["eval".as_ref(), file, "a..b".as_ref()],
        vec!["eval".as_ref(), file, ".a".as_ref()],
        vec!["eval".as_ref(), file, "a.".as_ref()],
        vec!["eval".as_ref(), file, "".as_ref()],
    ];
    for args in cases {
        let output = run(&args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(stderr(&output).contains("lonk --help"), "{args:?}");
    }
}

#[test]
fn help_version_and_option_terminator_work() {
    for args in [["--help"].as_slice(), &["-h"], &["eval", "--help"]] {
        let output = run(&args.iter().map(OsStr::new).collect::<Vec<_>>());
        assert!(output.status.success());
        assert!(stdout(&output).contains("lonk eval FILE [PATH]"));
    }
    for option in ["--version", "-V"] {
        let output = run(&[option.as_ref()]);
        assert!(output.status.success());
        assert_eq!(
            stdout(&output),
            concat!("lonk ", env!("CARGO_PKG_VERSION"), "\n")
        );
    }
    let source = SourceFile::new("{\"-field\" = 42}");
    let output = run(&[
        "eval".as_ref(),
        "--".as_ref(),
        source.0.as_os_str(),
        "-field".as_ref(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "42\n");
}

#[test]
fn missing_file_is_a_readable_error_without_a_panic() {
    let source = SourceFile::new("1");
    fs::remove_file(&source.0).unwrap();
    let output = source.run("eval", &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains(&source.0.display().to_string()));
    assert!(!stderr(&output).contains("panicked"));
}

#[cfg(unix)]
#[test]
fn non_utf8_selection_is_a_usage_error_without_a_panic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let source = SourceFile::new("1");
    let path = OsString::from_vec(vec![0xff]);
    let output = run(&["check".as_ref(), source.0.as_os_str(), &path]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("PATH must be valid UTF-8"));
}
