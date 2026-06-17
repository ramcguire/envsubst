mod env_context;

use env_context::EnvContext;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut opts = getopts::Options::new();
    opts.optopt(
        "o",
        "output",
        "write substituted files to DIR, mirroring input structure",
        "DIR",
    );
    opts.optmulti(
        "e",
        "env-file",
        "load variables from GLOB (.env syntax); disables real env lookup",
        "GLOB",
    );
    opts.optflag(
        "v",
        "verbose",
        "print file paths and unresolved variables to stderr",
    );
    opts.optflag(
        "f",
        "fail-on-missing",
        "exit 1 if any variables remain unresolved after substitution",
    );
    opts.optflag("h", "help", "show this help");

    let matches = opts.parse(&args[1..]).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    if matches.opt_present("h") || matches.free.is_empty() {
        let brief = format!("Usage: {} [options] PATTERN [PATTERN...]", args[0]);
        print!("{}", opts.usage(&brief));
        process::exit(if matches.opt_present("h") { 0 } else { 1 });
    }

    let output_dir = matches.opt_str("o").map(PathBuf::from);
    let env_file_globs = matches.opt_strs("e");
    let verbose = matches.opt_present("v");
    let fail_on_missing = matches.opt_present("f");
    let patterns = &matches.free;

    // Collect preloaded vars from all matching .env files.
    // If any --env-file is given, real env is not consulted.
    let (preloaded, use_real_env) = if env_file_globs.is_empty() {
        (HashMap::new(), true)
    } else {
        let mut map = HashMap::new();
        for glob_pat in &env_file_globs {
            let entries = glob::glob(glob_pat).unwrap_or_else(|e| {
                eprintln!("error: invalid env-file glob '{glob_pat}': {e}");
                process::exit(1);
            });
            for entry in entries {
                let path = entry.unwrap_or_else(|e| {
                    eprintln!("warning: skipping unreadable env-file path: {e}");
                    PathBuf::new()
                });
                if path.as_os_str().is_empty() || path.is_dir() {
                    continue;
                }
                match parse_env_file(&path) {
                    Ok(vars) => {
                        if verbose {
                            eprintln!("loaded env-file: {}", path.display());
                        }
                        map.extend(vars);
                    }
                    Err(e) => {
                        eprintln!("error: cannot read env-file '{}': {e}", path.display());
                        process::exit(1);
                    }
                }
            }
        }
        (map, false)
    };

    let ctx = EnvContext::new(preloaded, use_real_env);

    let mut file_count = 0u32;
    for pattern in patterns {
        let base = glob_base(pattern);
        let entries = glob::glob(pattern).unwrap_or_else(|e| {
            eprintln!("error: invalid glob pattern '{pattern}': {e}");
            process::exit(1);
        });
        for entry in entries {
            let path = entry.unwrap_or_else(|e| {
                eprintln!("warning: skipping unreadable path: {e}");
                process::exit(1);
            });
            if path.is_dir() {
                continue;
            }
            process_file(&path, &base, output_dir.as_deref(), &ctx, verbose);
            file_count += 1;
        }
    }

    if file_count == 0 {
        eprintln!("error: no files matched the provided patterns");
        process::exit(1);
    }

    let missing = ctx.missing_vars();
    if !missing.is_empty() {
        if verbose {
            let mut pairs: Vec<_> = missing.iter().collect();
            pairs.sort_by_key(|(k, _)| k.as_str());
            eprintln!("\nunresolved variables ({}):", pairs.len());
            for (key, count) in &pairs {
                eprintln!("  ${key} (referenced {count}x)");
            }
        }
        if fail_on_missing {
            process::exit(1);
        }
    }
}

fn process_file(
    path: &Path,
    base: &Path,
    output_dir: Option<&Path>,
    ctx: &EnvContext,
    verbose: bool,
) {
    let contents = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("warning: skipping '{}': {e}", path.display());
            return;
        }
    };

    let result = shellexpand::env_with_context(&contents, |k| ctx.lookup(k)).unwrap();

    match output_dir {
        Some(out_dir) => {
            let out_path = output_path(path, base, out_dir);
            if let Some(parent) = out_path.parent()
                && let Err(e) = fs::create_dir_all(parent)
            {
                eprintln!("error: cannot create directory '{}': {e}", parent.display());
                process::exit(1);
            }
            if let Err(e) = fs::write(&out_path, result.as_bytes()) {
                eprintln!("error: cannot write '{}': {e}", out_path.display());
                process::exit(1);
            }
            if verbose {
                eprintln!("{} -> {}", path.display(), out_path.display());
            }
        }
        None => {
            if verbose {
                eprintln!("# {}", path.display());
            }
            print!("{result}");
        }
    }
}

fn output_path(file: &Path, base: &Path, out_dir: &Path) -> PathBuf {
    let rel = file
        .strip_prefix(base)
        .unwrap_or_else(|_| file.file_name().map(Path::new).unwrap_or(file));
    out_dir.join(rel)
}

/// Extract the non-wildcard directory prefix of a glob pattern.
///
/// `"templates/**/*.yaml"` → `"templates"`
/// `"*.yaml"`              → `"."`
/// `"/etc/conf/*.conf"`    → `"/etc/conf"`
fn glob_base(pattern: &str) -> PathBuf {
    let wildcard = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
    let prefix = &pattern[..wildcard];
    let trimmed = prefix.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return PathBuf::from(".");
    }
    let path = Path::new(trimmed);
    if prefix.ends_with('/') || prefix.ends_with('\\') {
        path.to_path_buf()
    } else {
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf()
    }
}

/// Parse a .env file into a map of key → value.
///
/// Handles `KEY=VALUE`, `export KEY=VALUE`, quoted values, `#` comments.
fn parse_env_file(path: &Path) -> Result<HashMap<String, String>, std::io::Error> {
    let content = fs::read_to_string(path)?;
    let vars = content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let l = l.strip_prefix("export ").unwrap_or(l).trim();
            let (key, val) = l.split_once('=')?;
            let key = key.trim().to_string();
            let val = val.trim();
            let val = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(val)
                .to_string();
            Some((key, val))
        })
        .collect();
    Ok(vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // glob_base

    #[test]
    fn glob_base_double_star() {
        assert_eq!(glob_base("templates/**/*.yaml"), PathBuf::from("templates"));
    }

    #[test]
    fn glob_base_no_directory() {
        assert_eq!(glob_base("*.yaml"), PathBuf::from("."));
    }

    #[test]
    fn glob_base_absolute() {
        assert_eq!(glob_base("/etc/conf/*.conf"), PathBuf::from("/etc/conf"));
    }

    #[test]
    fn glob_base_mid_component() {
        assert_eq!(glob_base("conf/file-*.yaml"), PathBuf::from("conf"));
    }

    #[test]
    fn glob_base_no_wildcard() {
        assert_eq!(glob_base("conf/file.yaml"), PathBuf::from("conf"));
    }

    #[test]
    fn glob_base_question_mark() {
        assert_eq!(glob_base("conf/file-?.yaml"), PathBuf::from("conf"));
    }

    #[test]
    fn glob_base_bracket() {
        assert_eq!(glob_base("conf/[abc]*.yaml"), PathBuf::from("conf"));
    }

    // parse_env_file

    #[test]
    fn parse_basic() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(
            tmp,
            "# comment\nFOO=bar\nexport BAZ=qux\nQUOTED=\"hello world\"\n"
        )
        .unwrap();
        let vars = parse_env_file(tmp.path()).unwrap();
        assert_eq!(vars["FOO"], "bar");
        assert_eq!(vars["BAZ"], "qux");
        assert_eq!(vars["QUOTED"], "hello world");
        assert!(!vars.contains_key("# comment"));
    }

    #[test]
    fn parse_single_quoted() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "KEY='single quoted'").unwrap();
        assert_eq!(parse_env_file(tmp.path()).unwrap()["KEY"], "single quoted");
    }

    #[test]
    fn parse_empty_value() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "EMPTY=").unwrap();
        assert_eq!(parse_env_file(tmp.path()).unwrap()["EMPTY"], "");
    }

    #[test]
    fn parse_empty_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        assert!(parse_env_file(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn parse_only_comments() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "# comment\n  # indented\n").unwrap();
        assert!(parse_env_file(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn parse_no_equals_skipped() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "NOEQUALS\nGOOD=value").unwrap();
        let vars = parse_env_file(tmp.path()).unwrap();
        assert!(!vars.contains_key("NOEQUALS"));
        assert_eq!(vars["GOOD"], "value");
    }

    #[test]
    fn parse_embedded_equals_in_value() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "URL=https://example.com?a=1&b=2").unwrap();
        assert_eq!(
            parse_env_file(tmp.path()).unwrap()["URL"],
            "https://example.com?a=1&b=2"
        );
    }

    // Integration: process_file using testdata fixtures

    #[test]
    fn integration_template_to_output_dir() {
        let out_dir = tempfile::TempDir::new().unwrap();
        let base = PathBuf::from("xtask/testdata/integration");
        let file = PathBuf::from("xtask/testdata/integration/template.yaml");

        temp_env::with_vars(
            [
                ("EV_GREETING", Some("hello")),
                ("EV_SERVICE", Some("my-svc")),
            ],
            || {
                let ctx = EnvContext::new(HashMap::new(), true);
                process_file(&file, &base, Some(out_dir.path()), &ctx, false);
                let out = fs::read_to_string(out_dir.path().join("template.yaml")).unwrap();
                assert!(out.contains("greeting: hello"));
                assert!(out.contains("service: my-svc"));
                assert!(
                    out.contains("${EV_UNDEFINED_12345}"),
                    "unset var left as-is"
                );
            },
        );
    }

    #[test]
    fn integration_nested_structure_preserved() {
        let out_dir = tempfile::TempDir::new().unwrap();
        let base = PathBuf::from("xtask/testdata/integration");
        let file = PathBuf::from("xtask/testdata/integration/nested/service.conf");

        temp_env::with_vars(
            [
                ("EV_HOST", Some("localhost")),
                ("EV_PORT", Some("8080")),
                ("EV_DEBUG", Some("true")),
            ],
            || {
                let ctx = EnvContext::new(HashMap::new(), true);
                process_file(&file, &base, Some(out_dir.path()), &ctx, false);
                let out = fs::read_to_string(out_dir.path().join("nested/service.conf")).unwrap();
                assert!(out.contains("host = localhost"));
                assert!(out.contains("port = 8080"));
                assert!(out.contains("debug = true"));
            },
        );
    }

    #[test]
    fn integration_env_file_mode_uses_preloaded() {
        let out_dir = tempfile::TempDir::new().unwrap();
        let mut env_file = tempfile::NamedTempFile::new().unwrap();
        writeln!(env_file, "EV_HOST=db.example.com").unwrap();
        writeln!(env_file, "EV_PORT=5432").unwrap();
        writeln!(env_file, "EV_DEBUG=false").unwrap();

        // Real env is absent — preloaded from file should be used
        temp_env::with_vars(
            [
                ("EV_HOST", None::<&str>),
                ("EV_PORT", None::<&str>),
                ("EV_DEBUG", None::<&str>),
            ],
            || {
                let preloaded = parse_env_file(env_file.path()).unwrap();
                let ctx = EnvContext::new(preloaded, false);
                let base = PathBuf::from("xtask/testdata/integration");
                let file = PathBuf::from("xtask/testdata/integration/nested/service.conf");
                process_file(&file, &base, Some(out_dir.path()), &ctx, false);
                let out = fs::read_to_string(out_dir.path().join("nested/service.conf")).unwrap();
                assert!(out.contains("host = db.example.com"));
                assert!(out.contains("port = 5432"));
                assert!(out.contains("debug = false"));
            },
        );
    }

    #[test]
    fn integration_env_file_mode_ignores_real_env() {
        let out_dir = tempfile::TempDir::new().unwrap();
        let base = PathBuf::from("xtask/testdata/integration");
        let file = PathBuf::from("xtask/testdata/integration/nested/service.conf");

        // Real env has different values — file mode must not see them
        temp_env::with_vars(
            [
                ("EV_HOST", Some("real-host")),
                ("EV_PORT", Some("9999")),
                ("EV_DEBUG", Some("real-debug")),
            ],
            || {
                let preloaded = HashMap::from([
                    ("EV_HOST".to_string(), "file-host".to_string()),
                    ("EV_PORT".to_string(), "1111".to_string()),
                    ("EV_DEBUG".to_string(), "file-debug".to_string()),
                ]);
                let ctx = EnvContext::new(preloaded, false);
                process_file(&file, &base, Some(out_dir.path()), &ctx, false);
                let out = fs::read_to_string(out_dir.path().join("nested/service.conf")).unwrap();
                assert!(out.contains("host = file-host"), "real env must be ignored");
                assert!(out.contains("port = 1111"));
            },
        );
    }

    #[test]
    fn integration_missing_vars_tracked() {
        let out_dir = tempfile::TempDir::new().unwrap();
        let base = PathBuf::from("xtask/testdata/integration");
        let file = PathBuf::from("xtask/testdata/integration/template.yaml");

        temp_env::with_vars(
            [("EV_GREETING", Some("hey")), ("EV_SERVICE", Some("svc"))],
            || {
                let ctx = EnvContext::new(HashMap::new(), true);
                process_file(&file, &base, Some(out_dir.path()), &ctx, false);
                assert!(ctx.missing_vars().contains_key("EV_UNDEFINED_12345"));
            },
        );
    }

    #[test]
    fn integration_stdout_when_no_output_dir() {
        // process_file with no output_dir should not panic and not write files
        let base = PathBuf::from("xtask/testdata/integration");
        let file = PathBuf::from("xtask/testdata/integration/template.yaml");

        temp_env::with_vars(
            [("EV_GREETING", Some("hi")), ("EV_SERVICE", Some("s"))],
            || {
                let ctx = EnvContext::new(HashMap::new(), true);
                // Just verify it doesn't panic; output goes to stdout
                process_file(&file, &base, None, &ctx, false);
            },
        );
    }

    // fail-on-missing: verify missing_vars() is populated (the condition that triggers exit 1)

    #[test]
    fn fail_on_missing_real_env_has_unresolved_vars() {
        let out_dir = tempfile::TempDir::new().unwrap();
        let base = PathBuf::from("xtask/testdata/integration");
        let file = PathBuf::from("xtask/testdata/integration/template.yaml");

        // EV_GREETING and EV_SERVICE are set; EV_UNDEFINED_12345 is not
        temp_env::with_vars(
            [("EV_GREETING", Some("hi")), ("EV_SERVICE", Some("svc"))],
            || {
                let ctx = EnvContext::new(HashMap::new(), true);
                process_file(&file, &base, Some(out_dir.path()), &ctx, false);
                let missing = ctx.missing_vars();
                assert!(
                    missing.contains_key("EV_UNDEFINED_12345"),
                    "unresolved var must appear in missing_vars — fail_on_missing would exit 1"
                );
                assert_eq!(
                    missing.len(),
                    1,
                    "only the truly absent var should be missing"
                );
            },
        );
    }

    #[test]
    fn fail_on_missing_env_file_mode_has_unresolved_vars() {
        let out_dir = tempfile::TempDir::new().unwrap();

        // Provide EV_GREETING and EV_SERVICE via .env file, leave EV_UNDEFINED_12345 absent
        let mut env_file = tempfile::NamedTempFile::new().unwrap();
        writeln!(env_file, "EV_GREETING=hello").unwrap();
        writeln!(env_file, "EV_SERVICE=svc").unwrap();

        let preloaded = parse_env_file(env_file.path()).unwrap();
        let ctx = EnvContext::new(preloaded, false);

        let base = PathBuf::from("xtask/testdata/integration");
        let file = PathBuf::from("xtask/testdata/integration/template.yaml");
        process_file(&file, &base, Some(out_dir.path()), &ctx, false);

        let missing = ctx.missing_vars();
        assert!(
            missing.contains_key("EV_UNDEFINED_12345"),
            "absent var must appear in missing_vars in env-file mode — fail_on_missing would exit 1"
        );
        assert_eq!(missing.len(), 1);
    }

    #[test]
    fn fail_on_missing_all_vars_resolved_means_empty_missing() {
        let out_dir = tempfile::TempDir::new().unwrap();
        let base = PathBuf::from("xtask/testdata/integration");
        let file = PathBuf::from("xtask/testdata/integration/nested/service.conf");

        // All three vars in service.conf are provided
        temp_env::with_vars(
            [
                ("EV_HOST", Some("h")),
                ("EV_PORT", Some("80")),
                ("EV_DEBUG", Some("false")),
            ],
            || {
                let ctx = EnvContext::new(HashMap::new(), true);
                process_file(&file, &base, Some(out_dir.path()), &ctx, false);
                assert!(
                    ctx.missing_vars().is_empty(),
                    "no missing vars — fail_on_missing would not exit"
                );
            },
        );
    }
}
