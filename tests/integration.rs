use regex::Regex;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct ProjectHarness {
    _dir: tempfile::TempDir,
    project: PathBuf,
}

impl ProjectHarness {
    fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join(name);
        Self::ok(&["init", "--path", project.to_str().unwrap(), name]);
        Self { _dir: dir, project }
    }

    fn binary() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("target/debug/cite-cli");
        if p.exists() {
            return p;
        }
        p.set_file_name("cite-cli");
        let mut release = p.clone();
        release.pop();
        release.push("release/cite-cli");
        if release.exists() {
            return release;
        }
        p
    }

    fn output(args: &[&str]) -> (String, String, bool) {
        let output = Command::new(Self::binary())
            .args(args)
            .output()
            .expect("Failed to run cite-cli");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (stdout, stderr, output.status.success())
    }

    fn ok(args: &[&str]) {
        let (_, stderr, ok) = Self::output(args);
        assert!(ok, "cite-cli {} failed: {stderr}", args.join(" "));
    }

    fn run(&self, args: &[&str]) -> (String, String, bool) {
        let mut full = args.to_vec();
        full.extend_from_slice(&["--path", self.project.to_str().unwrap()]);
        Self::output(&full)
    }

    fn run_ok(&self, args: &[&str]) -> String {
        let (_, stderr, ok) = self.run(args);
        assert!(ok, "cite-cli {} failed: {stderr}", args.join(" "));
        stderr
    }

    fn write_metadata(&self, yaml: &str) {
        fs::write(self.project.join("metadata.yml"), yaml).unwrap();
    }

    fn write_content(&self, relative: &str, text: &str) {
        let path = self.project.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn read_bundle(&self) -> serde_json::Value {
        let content = fs::read_to_string(self.project.join("build/content.json")).unwrap();
        serde_json::from_str(&content).unwrap()
    }
}

// ── init ────────────────────────────────────────────────────────

#[test]
fn init_creates_project_structure() {
    let h = ProjectHarness::new("my-project");

    assert!(h.project.join("cite.toml").exists(), "cite.toml");
    assert!(h.project.join("metadata.yml").exists(), "metadata.yml");
    assert!(h.project.join("content").is_dir(), "content/");
    assert!(h.project.join("assets/audio").is_dir(), "assets/audio/");
    assert!(h.project.join("assets/image").is_dir(), "assets/image/");
    assert!(!h.project.join("build").exists());
    assert!(h.project.join(".gitignore").exists(), ".gitignore");
}

#[test]
fn init_is_idempotent_on_existing_project() {
    let h = ProjectHarness::new("existing");
    let (_, stderr, ok) =
        ProjectHarness::output(&["init", "--path", h.project.to_str().unwrap(), "existing"]);
    assert!(ok);
    assert!(stderr.contains("ready"));
    assert!(stderr.contains("Skipped"));
}

// ── doctor ────────────────────────────────────────────────────

#[test]
fn doctor_passes_on_empty_project() {
    let h = ProjectHarness::new("empty");
    h.run_ok(&["doctor"]);
}

#[test]
fn doctor_catches_missing_file() {
    let h = ProjectHarness::new("missing-file");
    h.write_metadata(
        r#"
podcasts:
  - title: "Broken"
    file: content/nonexistent.md
"#,
    );
    let (_, stderr, ok) = h.run(&["doctor"]);
    assert!(!ok);
    assert!(stderr.contains("does not exist"));
}

#[test]
fn doctor_catches_missing_metadata() {
    let h = ProjectHarness::new("no-meta");
    fs::remove_file(h.project.join("metadata.yml")).unwrap();
    let (_, stderr, ok) = h.run(&["doctor"]);
    assert!(!ok);
    assert!(stderr.contains("not found"));
}

// ── lint ────────────────────────────────────────────────────────

#[test]
fn lint_warns_on_short_content() {
    let h = ProjectHarness::new("short-content");
    h.write_content("content/a.md", "Hi");
    h.write_metadata(
        r#"
podcasts:
  - title: "A"
    file: content/a.md
"#,
    );
    let (_, stderr, ok) = h.run(&["lint"]);
    assert!(ok);
    assert!(stderr.contains("very short"));
}

// ── build ───────────────────────────────────────────────────────

#[test]
fn build_produces_valid_content_json() {
    let h = ProjectHarness::new("build-test");
    h.write_content("content/article.md", "# Hello World");
    h.write_metadata(
        r#"
podcasts:
  - title: "My Podcast"
    file: content/article.md
    category: tech
"#,
    );

    // Set artist_id in cite.toml
    let mut toml = fs::read_to_string(h.project.join("cite.toml")).unwrap();
    let re = Regex::new(r#"(?m)^(artist_id\s*=\s*)"[^"]*""#).unwrap();
    toml = re.replace(&toml, r#"$1"alice_uuid""#).to_string();
    fs::write(h.project.join("cite.toml"), toml).unwrap();
    h.run_ok(&["build"]);

    let bundle = h.read_bundle();
    assert!(h.project.join("build").exists());
    assert_eq!(bundle["project"], "build-test");
    assert_eq!(bundle["compiler_version"], 1.0);
    assert_eq!(bundle["artist_id"], "alice_uuid");
    assert_eq!(bundle["podcasts"].as_array().unwrap().len(), 1);
    let pod = &bundle["podcasts"][0];
    assert!(!pod["id"].as_str().unwrap().is_empty());
    assert_eq!(pod["title"], "My Podcast");
    assert_eq!(pod["content"], "# Hello World");
}

#[test]
fn build_generates_timelines_from_bib_citations() {
    let h = ProjectHarness::new("bib-timeline-test");
    h.write_content("content/release.md", "# v1.0 Released");
    h.write_content(
        "content/papers.bib",
        r#"
@article{paper2023,
  title = {Breakthrough in Materials},
  author = {Smith, J.},
  year = {2023},
  month = jun,
  abstract = {A major breakthrough.},
}
@inproceedings{paper2024,
  title = {Follow-up Study},
  author = {Smith, J. and Doe, A.},
  year = {2024},
  month = jan,
  abstract = {Extended results.},
}
"#,
    );
    h.write_metadata(
        r#"
podcasts:
  - title: "Release 1"
    file: content/release.md
    citation: content/papers.bib
"#,
    );

    h.run_ok(&["build"]);
    let bundle = h.read_bundle();

    let timelines = bundle["timelines"].as_array().unwrap();
    assert_eq!(
        timelines.len(),
        1,
        "should generate one timeline from citation"
    );

    let tl = &timelines[0];
    assert!(!tl["id"].as_str().unwrap().is_empty());

    let entries = tl["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["date"], "2023-06");
    assert!(
        entries[0]["title"]
            .as_str()
            .unwrap()
            .contains("Breakthrough")
    );
    assert_eq!(entries[1]["date"], "2024-01");
}

#[test]
fn build_is_idempotent_with_cache() {
    let h = ProjectHarness::new("cached-build");
    h.write_content("content/article.md", "# Same");
    h.write_metadata(
        r#"
podcasts:
  - title: "A"
    file: content/article.md
"#,
    );

    h.run_ok(&["build"]);
    let first = h.read_bundle();

    let (_, stderr, ok) = h.run(&["build"]);
    assert!(ok, "cached build: {stderr}");

    h.run_ok(&["build", "--force"]);

    let second = h.read_bundle();
    // Compare everything except the auto-generated UUID (which changes each run)
    let first_content = &first["podcasts"][0]["content"];
    let second_content = &second["podcasts"][0]["content"];
    assert_eq!(
        first_content, second_content,
        "force rebuild content should match"
    );
    assert_eq!(first["project"], second["project"]);
    assert_eq!(first["artist_id"], second["artist_id"]);
}

#[test]
fn build_does_not_resolve_wiki_links() {
    let h = ProjectHarness::new("no-wiki-test");
    h.write_content("content/main.md", "See [[other-page]] for details");
    h.write_metadata(
        r#"
podcasts:
  - title: "Main"
    file: content/main.md
"#,
    );

    h.run_ok(&["build"]);

    let bundle = h.read_bundle();
    let content = bundle["podcasts"][0]["content"].as_str().unwrap();
    // Wiki links are not resolved — they pass through as-is
    assert!(
        content.contains("[[other-page]]"),
        "wiki-link should remain as-is, got: {content}"
    );
}

#[test]
fn build_empty_project_succeeds() {
    let h = ProjectHarness::new("empty-build");
    h.run_ok(&["build"]);
    let bundle = h.read_bundle();
    let pods = bundle["podcasts"].as_array().unwrap();
    assert_eq!(pods.len(), 0, "template has no default podcast");
}

#[test]
fn build_force_rebuilds() {
    let h = ProjectHarness::new("force-build");
    h.write_content("content/a.md", "v1");
    h.write_metadata(
        r#"
podcasts:
  - title: "A"
    file: content/a.md
"#,
    );
    h.run_ok(&["build"]);
    let first = h.read_bundle();

    h.run_ok(&["build", "--force"]);
    let second = h.read_bundle();
    assert_eq!(
        first["podcasts"][0]["content"],
        second["podcasts"][0]["content"]
    );
    assert_eq!(second["compiler_version"], 1.0);
}

// ── status ──────────────────────────────────────────────────────

#[test]
fn status_shows_project_info() {
    let h = ProjectHarness::new("status-test");
    h.write_content("content/a.md", "# Content");
    h.write_metadata(
        r#"
podcasts:
  - title: "A"
    file: content/a.md
"#,
    );

    let stderr = h.run_ok(&["status"]);
    assert!(stderr.contains("status-test"));
    assert!(stderr.contains("Artist ID"));

    h.run_ok(&["build"]);

    let stderr = h.run_ok(&["status"]);
    assert!(stderr.contains("exists"));
}

// ── doctor ──────────────────────────────────────────────────────

#[test]
fn doctor_detects_missing_project() {
    let (_, stderr, ok) =
        ProjectHarness::output(&["doctor", "--path", "/tmp/nonexistent-project-test-12345"]);
    assert!(ok);
    assert!(stderr.contains("cite.toml not found"));
}

#[test]
fn doctor_shows_project_health() {
    let h = ProjectHarness::new("doctor-test");
    let stderr = h.run_ok(&["doctor"]);
    assert!(stderr.contains("cite.toml found"));
    assert!(stderr.contains("metadata.yml found"));
}

// ── clean ───────────────────────────────────────────────────────

#[test]
fn clean_removes_artifacts_and_is_idempotent() {
    let h = ProjectHarness::new("clean-test");
    h.write_content("content/a.md", "# A");
    h.write_metadata(
        r#"
podcasts:
  - title: "A"
    file: content/a.md
"#,
    );
    h.run_ok(&["build"]);
    assert!(h.project.join("build").exists());

    h.run_ok(&["clean"]);
    assert!(!h.project.join("build").exists(), "build/ removed");
    assert!(
        !h.project.join(".cite-cache.json").exists(),
        "cache removed"
    );

    h.run_ok(&["clean"]);
}

// ── deploy ──────────────────────────────────────────────────────

#[test]
fn deploy_fails_without_backend() {
    let h = ProjectHarness::new("no-backend");
    let (_, stderr, ok) = h.run(&["deploy"]);
    assert!(!ok);
    assert!(stderr.contains("No [backend]") || stderr.contains("No build artifact"));
}

#[test]
fn deploy_fails_without_build() {
    let h = ProjectHarness::new("no-build");
    fs::write(
        h.project.join("cite.toml"),
        "[project]\nname = \"no-build\"\nversion = \"0.1.0\"\nlanguage = \"en\"\nmetadata_file = \"metadata.yml\"\nartist_id = \"\"\n\n[build]\ncompiler_version = 0.0\nincremental = true\noutput_format = \"json\"\n\n[backend]\nstaging_url = \"https://test.supabase.co\"\nstaging_service_key = \"test-key\"\n",
    ).unwrap();
    let (_, stderr, ok) = h.run(&["deploy"]);
    assert!(!ok);
    assert!(stderr.contains("No build artifact"));
}

// ── rollback ────────────────────────────────────────────────────

#[test]
fn rollback_fails_without_backend() {
    let h = ProjectHarness::new("no-backend-rb");
    let (_, stderr, ok) = h.run(&["rollback", "some-id"]);
    assert!(!ok);
    assert!(stderr.contains("No [backend]"));
}

// ── e2e ─────────────────────────────────────────────────────────

#[test]
fn full_workflow_end_to_end() {
    let h = ProjectHarness::new("e2e");

    h.write_content("content/ai.md", "# AI Article\nSome content about AI.");
    h.write_content("content/ml.md", "# ML Article\nSome content about ML.");
    h.write_metadata(
        r#"
podcasts:
  - title: "AI Article"
    file: content/ai.md
    category: tech
  - title: "ML Article"
    file: content/ml.md
    category: tech
"#,
    );

    h.run_ok(&["doctor"]);
    h.run_ok(&["lint"]);
    h.run_ok(&["build"]);

    let stderr = h.run_ok(&["status"]);
    assert!(stderr.contains("Artist ID"));
    assert!(stderr.contains("Podcasts: 2"));
    let bundle = h.read_bundle();
    assert_eq!(bundle["project"], "e2e");
    assert_eq!(bundle["podcasts"].as_array().unwrap().len(), 2);
    assert!(
        bundle["podcasts"][0]["content"]
            .as_str()
            .unwrap()
            .contains("AI")
    );
    assert!(
        bundle["podcasts"][1]["content"]
            .as_str()
            .unwrap()
            .contains("ML")
    );

    h.run_ok(&["clean"]);
    assert!(!h.project.join("build").exists());
}

// ── cli basics ──────────────────────────────────────────────────

#[test]
fn help_prints_usage() {
    let (stdout, _, ok) = ProjectHarness::output(&["--help"]);
    assert!(ok);
    assert!(stdout.contains("cite-cli"));
    assert!(stdout.contains("rollback"));
    assert!(stdout.contains("deploy"));
}

#[test]
fn verbose_flag_works() {
    let h = ProjectHarness::new("verbose-test");
    h.run_ok(&["doctor", "--verbose"]);
}
