use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

fn write_template(dir: &TempDir, contents: &str) -> PathBuf {
    let path = dir.path().join("template.txt");
    fs::write(&path, contents).unwrap();
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_envsubst"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn scope_substitutes_selected_variables_only() {
    let temp = TempDir::new().unwrap();
    let template = write_template(&temp, "${SCOPED_VALUE}/${UNSCOPED_VALUE}");

    let output = Command::new(env!("CARGO_BIN_EXE_envsubst"))
        .args(["--scope", "SCOPED_VALUE", template.to_str().unwrap()])
        .env("SCOPED_VALUE", "selected")
        .env("UNSCOPED_VALUE", "unselected")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "selected/${UNSCOPED_VALUE}"
    );
}

#[test]
fn scope_is_repeatable_and_supports_the_short_flag() {
    let temp = TempDir::new().unwrap();
    let template = write_template(&temp, "${FIRST_VALUE}/${SECOND_VALUE}/${UNSCOPED_VALUE}");

    let output = Command::new(env!("CARGO_BIN_EXE_envsubst"))
        .args([
            "-s",
            "FIRST_VALUE",
            "--scope",
            "SECOND_VALUE",
            template.to_str().unwrap(),
        ])
        .env("FIRST_VALUE", "first")
        .env("SECOND_VALUE", "second")
        .env("UNSCOPED_VALUE", "unscoped")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "first/second/${UNSCOPED_VALUE}"
    );
}

#[test]
fn fail_on_missing_ignores_out_of_scope_references() {
    let temp = TempDir::new().unwrap();
    let template = write_template(&temp, "${SCOPED_VALUE}/${OUT_OF_SCOPE_MISSING}");

    let output = Command::new(env!("CARGO_BIN_EXE_envsubst"))
        .args([
            "--scope",
            "SCOPED_VALUE",
            "--fail-on-missing",
            template.to_str().unwrap(),
        ])
        .env("SCOPED_VALUE", "selected")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "selected/${OUT_OF_SCOPE_MISSING}"
    );
}

#[test]
fn fail_on_missing_reports_scoped_variables_without_writing_output() {
    let temp = TempDir::new().unwrap();
    let template = write_template(&temp, "${REQUIRED_VALUE}");
    let output_dir = temp.path().join("rendered");

    let output = run(&[
        "--scope",
        "REQUIRED_VALUE",
        "--fail-on-missing",
        "--output",
        output_dir.to_str().unwrap(),
        template.to_str().unwrap(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("$REQUIRED_VALUE (referenced 1x)")
    );
    assert!(!output_dir.exists());
}

#[test]
fn scope_uses_env_file_values_without_consulting_process_environment() {
    let temp = TempDir::new().unwrap();
    let template = write_template(&temp, "${FROM_FILE}/${OUT_OF_SCOPE}");
    let env_file = temp.path().join("values.env");
    fs::write(&env_file, "FROM_FILE=file-value\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_envsubst"))
        .args([
            "--scope",
            "FROM_FILE",
            "--env-file",
            env_file.to_str().unwrap(),
            template.to_str().unwrap(),
        ])
        .env("FROM_FILE", "process-value")
        .env("OUT_OF_SCOPE", "process-only")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "file-value/${OUT_OF_SCOPE}"
    );
}

#[test]
fn scope_accepts_empty_values() {
    let temp = TempDir::new().unwrap();
    let template = write_template(&temp, "before${EMPTY_VALUE}after");

    let output = Command::new(env!("CARGO_BIN_EXE_envsubst"))
        .args([
            "--scope",
            "EMPTY_VALUE",
            "--fail-on-missing",
            template.to_str().unwrap(),
        ])
        .env("EMPTY_VALUE", "")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "beforeafter");
}

#[test]
fn scope_rejects_invalid_variable_names() {
    let temp = TempDir::new().unwrap();
    let template = write_template(&temp, "text");

    let output = run(&["--scope", "invalid-name", template.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("invalid scoped variable 'invalid-name'")
    );
}
