use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("test")             => cmd_test(),
        Some("lint")             => cmd_lint(),
        Some("fmt")              => cmd_fmt(),
        Some("docker-build")     => cmd_docker_build(&args.next().unwrap_or_else(|| "dev".into())),
        Some("docker-build-all") => cmd_docker_build_all(),
        Some("docker-test")      => {
            let image = args.next().unwrap_or_else(|| "envsubst:dev".into());
            if !run_docker_tests(&image) {
                std::process::exit(1);
            }
        }
        Some("ci") => cmd_ci(),
        Some(cmd) => {
            eprintln!("error: unknown command: {cmd}");
            std::process::exit(2);
        }
        None => {
            eprintln!("Usage: cargo xtask <command>");
            eprintln!("  test                  run unit and integration tests");
            eprintln!("  lint                  clippy + formatting check");
            eprintln!("  fmt                   auto-format source files");
            eprintln!("  docker-build [TAG]    build default image variant (default tag: dev)");
            eprintln!("  docker-build-all      build all four image variants");
            eprintln!("  docker-test [IMAGE]   run Docker image integration tests");
            eprintln!("  ci                    test + lint + docker-build + docker-test");
            std::process::exit(2);
        }
    }
}

// ── Orchestration commands ────────────────────────────────────────────────────

fn cmd_test() {
    exec(Command::new("cargo").arg("test"));
}

fn cmd_lint() {
    exec(Command::new("cargo").args(["clippy", "--", "-D", "warnings"]));
    exec(Command::new("cargo").args(["fmt", "--check"]));
}

fn cmd_fmt() {
    exec(Command::new("cargo").arg("fmt"));
}

fn cmd_docker_build(tag: &str) {
    docker_build_variant("envsubst", tag, "static-debian12", "latest");
}

fn cmd_docker_build_all() {
    for (variant, distroless_tag, image_tag) in VARIANTS {
        println!("building envsubst:{image_tag}");
        docker_build_variant("envsubst", image_tag, variant, distroless_tag);
    }
}

fn cmd_ci() {
    cmd_test();
    cmd_lint();
    cmd_docker_build("dev");
    if !run_docker_tests("envsubst:dev") {
        std::process::exit(1);
    }
}

const VARIANTS: &[(&str, &str, &str)] = &[
    ("static-debian12", "latest",  "dev"),
    ("static-debian12", "nonroot", "dev-nonroot"),
    ("static-debian13", "latest",  "dev-debian13"),
    ("static-debian13", "nonroot", "dev-debian13-nonroot"),
];

fn docker_build_variant(image: &str, image_tag: &str, variant: &str, distroless_tag: &str) {
    exec(Command::new("docker").args([
        "buildx", "build",
        "--build-arg", &format!("DISTROLESS_VARIANT={variant}"),
        "--build-arg", &format!("DISTROLESS_TAG={distroless_tag}"),
        "--tag",       &format!("{image}:{image_tag}"),
        "--load",
        ".",
    ]));
}

fn exec(cmd: &mut Command) {
    let status = cmd.status().unwrap_or_else(|e| panic!("failed to run {:?}: {e}", cmd.get_program()));
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

// ── Docker image integration tests ───────────────────────────────────────────

struct Runner {
    pass: u32,
    fail: u32,
    errors: Vec<String>,
    workdir: TempDir,
}

struct Out {
    exit: i32,
    stdout: String,
    stderr: String,
}

impl Runner {
    fn new() -> Self {
        Self {
            pass: 0,
            fail: 0,
            errors: Vec::new(),
            workdir: TempDir::new().expect("temp dir"),
        }
    }

    fn run(&self, args: &[&str]) -> Out {
        let output = Command::new("docker")
            .args(["run", "--rm"])
            .args(args)
            .output()
            .expect("failed to invoke docker — is it installed and running?");
        Out {
            exit: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn mktmpl(&self, rel: &str, content: &str) {
        let p = self.p(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn mkdir(&self, rel: &str) -> PathBuf {
        let p = self.p(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn p(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.workdir.path().to_path_buf(), |acc, c| acc.join(c))
    }

    fn ok(&mut self, name: &str) {
        println!("  PASS: {name}");
        self.pass += 1;
    }

    fn fail(&mut self, name: &str, msg: &str) {
        println!("  FAIL: {name} — {msg}");
        self.fail += 1;
        self.errors.push(name.to_string());
    }

    fn assert_exit(&mut self, name: &str, out: &Out, expected: i32) {
        if out.exit == expected {
            self.ok(name);
        } else {
            self.fail(name, &format!("expected exit {expected}, got {}", out.exit));
            if !out.stdout.trim().is_empty() { println!("        stdout: {}", out.stdout.trim()); }
            if !out.stderr.trim().is_empty() { println!("        stderr: {}", out.stderr.trim()); }
        }
    }

    fn assert_nonzero(&mut self, name: &str, out: &Out) {
        if out.exit != 0 { self.ok(name); }
        else { self.fail(name, "expected non-zero exit, got 0"); }
    }

    fn assert_contains(&mut self, name: &str, text: &str, substr: &str) {
        if text.contains(substr) { self.ok(name); }
        else {
            self.fail(name, &format!("did not find {substr:?}"));
            println!("        text: {}", text.trim());
        }
    }

    fn assert_not_contains(&mut self, name: &str, text: &str, substr: &str) {
        if !text.contains(substr) { self.ok(name); }
        else {
            self.fail(name, &format!("unexpectedly found {substr:?}"));
            println!("        text: {}", text.trim());
        }
    }

    fn assert_file(&mut self, name: &str, path: &Path, substr: &str) {
        match fs::read_to_string(path) {
            Ok(s) if s.contains(substr) => self.ok(name),
            Ok(s) => {
                self.fail(name, &format!("did not find {substr:?} in output file"));
                println!("        file: {}", s.trim());
            }
            Err(e) => self.fail(name, &format!("could not read output file: {e}")),
        }
    }

    fn summary(self) -> bool {
        println!();
        println!("Results: {} passed, {} failed", self.pass, self.fail);
        if !self.errors.is_empty() {
            println!();
            println!("Failed:");
            for e in &self.errors { println!("  - {e}"); }
        }
        self.fail == 0
    }
}

fn dp(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn section(title: &str) {
    println!();
    println!("=== {title} ===");
}

fn run_docker_tests(image: &str) -> bool {
    println!("Testing image: {image}");
    let mut r = Runner::new();

    section("Smoke");

    let o = r.run(&[image, "--help"]);
    r.assert_exit("help: exits 0", &o, 0);
    r.assert_contains("help: shows PATTERN", &o.stdout, "PATTERN");

    let o = r.run(&[image]);
    r.assert_nonzero("no-args: exits non-zero", &o);

    section("Stdout substitution");

    r.mktmpl("s1/a.txt", "hello ${GREETING}");
    let d = r.mkdir("s1");
    let o = r.run(&[
        "-e", "GREETING=world",
        "-v", &format!("{}:/in:ro", dp(&d)),
        image, "/in/a.txt",
    ]);
    r.assert_exit("basic: exits 0", &o, 0);
    r.assert_contains("basic: substituted", &o.stdout, "hello world");

    r.mktmpl("s2/a.txt", "val: ${__ABSENT_XYZ__}");
    let d = r.mkdir("s2");
    let o = r.run(&["-v", &format!("{}:/in:ro", dp(&d)), image, "/in/a.txt"]);
    r.assert_exit("missing-literal: exits 0", &o, 0);
    r.assert_contains("missing-literal: left as-is", &o.stdout, "${__ABSENT_XYZ__}");

    r.mktmpl("s3/a.txt", "${__NODEFAULT__:-fallback}");
    let d = r.mkdir("s3");
    let o = r.run(&["-v", &format!("{}:/in:ro", dp(&d)), image, "/in/a.txt"]);
    r.assert_exit("default-expansion: exits 0", &o, 0);
    r.assert_contains("default-expansion: applied", &o.stdout, "fallback");
    r.assert_not_contains("default-expansion: no literal", &o.stdout, "${__NODEFAULT__");

    section("Output directory");

    r.mktmpl("o1/in/app.yaml", "db: ${DB_HOST}");
    let (in_d, out_d) = (r.mkdir("o1/in"), r.mkdir("o1/out"));
    let o = r.run(&[
        "-e", "DB_HOST=postgres",
        "-v", &format!("{}:/in:ro", dp(&in_d)),
        "-v", &format!("{}:/out", dp(&out_d)),
        image, "/in/app.yaml", "--output", "/out",
    ]);
    r.assert_exit("output-dir: exits 0", &o, 0);
    r.assert_file("output-dir: file written", &out_d.join("app.yaml"), "db: postgres");

    // Nested directory structure is mirrored: glob base is /in, so
    // /in/root.conf → /out/root.conf and /in/nested/inner.conf → /out/nested/inner.conf.
    r.mktmpl("o2/in/root.conf", "a=${VAL}");
    r.mktmpl("o2/in/nested/inner.conf", "b=${VAL}");
    let (in_d, out_d) = (r.mkdir("o2/in"), r.mkdir("o2/out"));
    let o = r.run(&[
        "-e", "VAL=x",
        "-v", &format!("{}:/in:ro", dp(&in_d)),
        "-v", &format!("{}:/out", dp(&out_d)),
        image, "/in/**/*.conf", "--output", "/out",
    ]);
    r.assert_exit("nested: exits 0", &o, 0);
    r.assert_file("nested: root file", &out_d.join("root.conf"), "a=x");
    r.assert_file("nested: inner file", &out_d.join("nested").join("inner.conf"), "b=x");

    section("Env-file mode");

    r.mktmpl("e1/template.txt", "service=${SVC_NAME}");
    r.mktmpl("e1/vars.env", "SVC_NAME=myapp\n");
    let d = r.mkdir("e1");
    let o = r.run(&[
        "-v", &format!("{}:/in:ro", dp(&d)),
        image, "/in/template.txt", "--env-file", "/in/vars.env",
    ]);
    r.assert_exit("env-file: exits 0", &o, 0);
    r.assert_contains("env-file: substituted", &o.stdout, "service=myapp");

    r.mktmpl("e2/template.txt", "val=${REAL_SYS_VAR}");
    r.mktmpl("e2/empty.env", "");
    let d = r.mkdir("e2");
    let o = r.run(&[
        "-e", "REAL_SYS_VAR=should_not_appear",
        "-v", &format!("{}:/in:ro", dp(&d)),
        image, "/in/template.txt", "--env-file", "/in/empty.env",
    ]);
    r.assert_exit("env-file-isolation: exits 0", &o, 0);
    r.assert_not_contains("env-file-isolation: sys env ignored", &o.stdout, "should_not_appear");
    r.assert_contains("env-file-isolation: left as literal", &o.stdout, "${REAL_SYS_VAR}");

    section("Fail on missing");

    r.mktmpl("f1/tmpl.txt", "${__DEFINITELY_MISSING__}");
    let d = r.mkdir("f1");
    let o = r.run(&[
        "-v", &format!("{}:/in:ro", dp(&d)),
        image, "/in/tmpl.txt", "--fail-on-missing",
    ]);
    r.assert_nonzero("fail-on-missing: exits non-zero for unresolved", &o);

    r.mktmpl("f2/tmpl.txt", "${ALL_PRESENT}");
    let d = r.mkdir("f2");
    let o = r.run(&[
        "-e", "ALL_PRESENT=yes",
        "-v", &format!("{}:/in:ro", dp(&d)),
        image, "/in/tmpl.txt", "--fail-on-missing",
    ]);
    r.assert_exit("fail-on-missing: exits 0 when all resolved", &o, 0);

    section("Verbose");

    r.mktmpl("v1/tmpl.txt", "${__VERBOSE_MISS__}");
    let d = r.mkdir("v1");
    let o = r.run(&[
        "-v", &format!("{}:/in:ro", dp(&d)),
        image, "/in/tmpl.txt", "--verbose",
    ]);
    r.assert_contains("verbose: unresolved var in stderr", &o.stderr, "__VERBOSE_MISS__");

    r.mktmpl("v2/tmpl.txt", "x=${X}");
    let d = r.mkdir("v2");
    let o = r.run(&[
        "-e", "X=1",
        "-v", &format!("{}:/in:ro", dp(&d)),
        image, "/in/tmpl.txt", "--verbose",
    ]);
    r.assert_contains("verbose: file path in stderr", &o.stderr, "tmpl.txt");

    r.summary()
}
