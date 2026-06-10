use std::process::Command;

fn cite_cli_bin() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target/debug/cite-cli");
    if !p.exists() {
        p.set_file_name("cite-cli");
        let mut release = p.clone();
        release.pop();
        release.push("release/cite-cli");
        if release.exists() {
            return release;
        }
    }
    p
}

fn run(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(cite_cli_bin())
        .args(args)
        .output()
        .expect("Failed to execute cite-cli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();
    (stdout, stderr, success)
}

#[test]
fn test_init_creates_project() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("test-project");

    let (_, stderr, ok) = run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "test-project",
    ]);
    assert!(ok, "init should succeed: {stderr}");
    assert!(project_dir.join("cite.toml").exists());
    assert!(project_dir.join("metadata.yml").exists());
    assert!(project_dir.join("content").is_dir());
    assert!(project_dir.join("assets/audio").is_dir());
    assert!(project_dir.join("assets/images").is_dir());
    assert!(project_dir.join("build").is_dir());
    assert!(project_dir.join(".gitignore").exists());
}

#[test]
fn test_init_fails_on_existing_nonempty() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("existing");

    // First init succeeds
    let (_, _, ok) = run(&["init", "--path", project_dir.to_str().unwrap(), "existing"]);
    assert!(ok);

    // Second init should fail
    let (_, stderr, ok) = run(&["init", "--path", project_dir.to_str().unwrap(), "existing"]);
    assert!(!ok, "second init on same dir should fail");
    assert!(stderr.contains("not empty") || stderr.contains("already exists"));
}

#[test]
fn test_validate_empty_project() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("test-validate");

    run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "test-validate",
    ]);
    let (_, stderr, ok) = run(&["validate", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "validate should pass on empty project: {stderr}");
}

#[test]
fn test_full_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("full-workflow");

    // Init
    let (_, _, ok) = run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "full-workflow",
    ]);
    assert!(ok);

    // Create content
    let content_file = project_dir.join("content/article.md");
    std::fs::write(&content_file, "# Test Article\nContent here.").unwrap();

    // Write metadata
    let meta = r#"
artists:
  - slug: alice
    name: "Alice"
news:
  - slug: test-article
    title: "Test Article"
    file: content/article.md
    category: tech
    artists: [alice]
podcasts: []
newsletters: []
timelines: []
"#;
    std::fs::write(project_dir.join("metadata.yml"), meta).unwrap();

    // Validate
    let (_, stderr, ok) = run(&["validate", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "validate should pass: {stderr}");

    // Lint
    let (_, stderr, ok) = run(&["lint", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "lint should pass: {stderr}");

    // Build
    let (_, stderr, ok) = run(&["build", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "build should succeed: {stderr}");
    assert!(project_dir.join("build/content.json").exists());

    // Build (cached) - should be no-op
    let (_, stderr, ok) = run(&["build", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "cached build should succeed: {stderr}");

    // Force rebuild
    let (_, stderr, ok) = run(&["build", "--path", project_dir.to_str().unwrap(), "--force"]);
    assert!(ok, "force rebuild should succeed: {stderr}");

    // Status
    let (_, stderr, ok) = run(&["status", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "status should succeed: {stderr}");

    // Doctor
    let (_, stderr, ok) = run(&["doctor", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "doctor should succeed: {stderr}");

    // Clean
    let (_, stderr, ok) = run(&["clean", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "clean should succeed: {stderr}");
    assert!(!project_dir.join("build").exists());
}

#[test]
fn test_validate_catches_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("bad-ref");

    run(&["init", "--path", project_dir.to_str().unwrap(), "bad-ref"]);

    let meta = r#"
artists: []
news:
  - slug: broken
    title: "Broken"
    file: content/nonexistent.md
podcasts: []
newsletters: []
timelines: []
"#;
    std::fs::write(project_dir.join("metadata.yml"), meta).unwrap();

    let (_, stderr, ok) = run(&["validate", "--path", project_dir.to_str().unwrap()]);
    assert!(!ok, "validate should fail for missing file");
    assert!(stderr.contains("does not exist"));
}

#[test]
fn test_validate_catches_bad_cross_ref() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("bad-ref");

    run(&["init", "--path", project_dir.to_str().unwrap(), "bad-ref"]);
    std::fs::write(project_dir.join("content/a.md"), "# A").unwrap();

    let meta = r#"
artists: []
news:
  - slug: a
    title: "A"
    file: content/a.md
    artists: [nonexistent-artist]
podcasts: []
newsletters: []
timelines: []
"#;
    std::fs::write(project_dir.join("metadata.yml"), meta).unwrap();

    let (_, stderr, ok) = run(&["validate", "--path", project_dir.to_str().unwrap()]);
    assert!(!ok, "validate should fail for bad cross-ref");
    assert!(stderr.contains("unknown artist"));
}

#[test]
fn test_help() {
    let (stdout, _, ok) = run(&["--help"]);
    assert!(ok);
    assert!(stdout.contains("cite-cli"));
}

#[test]
fn test_verbose_flag() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("verbose-test");
    let (_, _, ok) = run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "verbose-test",
    ]);
    assert!(ok);
    let (_, _stderr, ok) = run(&[
        "validate",
        "--path",
        project_dir.to_str().unwrap(),
        "--verbose",
    ]);
    assert!(ok);
}

#[test]
fn test_doctor_on_nonexistent_project() {
    let dir = tempfile::tempdir().unwrap();
    let (_, stderr, ok) = run(&["doctor", "--path", dir.path().to_str().unwrap()]);
    assert!(ok);
    assert!(stderr.contains("cite.toml not found"));
}

#[test]
fn test_status_after_init() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("status-test");
    run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "status-test",
    ]);
    let (_, stderr, ok) = run(&["status", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
    assert!(stderr.contains("status-test"));
    assert!(stderr.contains("Artists:  0"));
}

#[test]
fn test_validate_missing_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("no-meta");
    run(&["init", "--path", project_dir.to_str().unwrap(), "no-meta"]);
    // Remove metadata.yml
    std::fs::remove_file(project_dir.join("metadata.yml")).unwrap();
    let (_, stderr, ok) = run(&["validate", "--path", project_dir.to_str().unwrap()]);
    assert!(!ok);
    assert!(stderr.contains("not found"));
}

#[test]
fn test_clean_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("clean-test");
    run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "clean-test",
    ]);
    let (_, _, ok) = run(&["clean", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
    // Second clean should also succeed
    let (_, _, ok) = run(&["clean", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
}

#[test]
fn test_build_then_clean() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("build-clean");
    run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "build-clean",
    ]);
    std::fs::write(project_dir.join("content/a.md"), "# Hello").unwrap();
    let meta = r#"
artists: []
news:
  - slug: test
    title: "Test"
    file: content/a.md
podcasts: []
newsletters: []
timelines: []
"#;
    std::fs::write(project_dir.join("metadata.yml"), meta).unwrap();
    // Build
    let (_, _, ok) = run(&["build", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
    assert!(project_dir.join("build/content.json").exists());
    // Clean
    let (_, _, ok) = run(&["clean", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
    assert!(!project_dir.join("build").exists());
}

#[test]
fn test_validate_empty_content_file() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("empty-content");
    run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "empty-content",
    ]);
    std::fs::write(project_dir.join("content/empty.md"), "").unwrap();
    let meta = r#"
artists: []
news:
  - slug: empty
    title: "Empty"
    file: content/empty.md
podcasts: []
newsletters: []
timelines: []
"#;
    std::fs::write(project_dir.join("metadata.yml"), meta).unwrap();
    let (_, _, ok) = run(&["validate", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "empty file should not fail validation");
    let (_, stderr, ok) = run(&["lint", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
    assert!(stderr.contains("very short"));
}

#[test]
fn test_complex_workflow_with_cross_refs() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("complex");
    run(&["init", "--path", project_dir.to_str().unwrap(), "complex"]);
    std::fs::write(project_dir.join("content/ai.md"), "# AI Article").unwrap();
    std::fs::write(project_dir.join("content/ml.md"), "# ML Article").unwrap();
    let meta = r#"
artists:
  - slug: alice
    name: "Alice"
  - slug: bob
    name: "Bob"
news:
  - slug: ai-article
    title: "AI Article"
    file: content/ai.md
    category: tech
    artists: [alice, bob]
  - slug: ml-article
    title: "ML Article"
    file: content/ml.md
    category: tech
    artists: [alice]
podcasts:
  - slug: ai-podcast
    title: "AI Podcast"
    file: content/ai.md
    duration_seconds: 1800
newsletters:
  - slug: weekly
    title: "Weekly"
    issue_number: 1
    published_date: "2026-06-10"
    included_news: [ai-article]
timelines:
  - slug: ai-timeline
    title: "AI Timeline"
    entries:
      - date: "2026-01-01"
        title: "Start"
        summary: "The beginning"
"#;
    std::fs::write(project_dir.join("metadata.yml"), meta).unwrap();

    let (_, stderr, ok) = run(&["validate", "--path", project_dir.to_str().unwrap()]);
    assert!(ok, "validate should pass: {stderr}");
    let (_, _, ok) = run(&["lint", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
    let (_, _, ok) = run(&["build", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
    let (_, stderr, ok) = run(&["status", "--path", project_dir.to_str().unwrap()]);
    assert!(ok);
    assert!(stderr.contains("Artists:  2"));
    assert!(stderr.contains("News:     2"));
    assert!(stderr.contains("Podcasts: 1"));
    assert!(stderr.contains("Newsletters: 1"));
    assert!(stderr.contains("Timelines: 1"));
    assert!(stderr.contains("✔ (exists)"));

    // Read built content.json and verify
    let content = std::fs::read_to_string(project_dir.join("build/content.json")).unwrap();
    let bundle: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(bundle["project"], "complex");
    assert_eq!(bundle["artists"].as_array().unwrap().len(), 2);
    assert_eq!(bundle["news"].as_array().unwrap().len(), 2);
    assert_eq!(bundle["podcasts"].as_array().unwrap().len(), 1);
    assert_eq!(bundle["newsletters"].as_array().unwrap().len(), 1);
    assert_eq!(bundle["timelines"].as_array().unwrap().len(), 1);
}

#[test]
fn test_deploy_fails_without_backend() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("no-backend");
    run(&[
        "init",
        "--path",
        project_dir.to_str().unwrap(),
        "no-backend",
    ]);
    let (_, stderr, ok) = run(&["deploy", "--path", project_dir.to_str().unwrap()]);
    assert!(!ok);
    assert!(stderr.contains("No build artifact") || stderr.contains("No [backend]"));
}

#[test]
fn test_deploy_fails_without_build() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("no-build");
    run(&["init", "--path", project_dir.to_str().unwrap(), "no-build"]);

    // Write a cite.toml with a backend section
    let manifest = format!(
        "[project]\nname = \"no-build\"\nversion = \"0.1.0\"\n\n[build]\ncompiler_version = \"0\"\nincremental = true\noutput_format = \"json\"\n\n[backend]\nstaging_url = \"https://test.supabase.co\"\nstaging_service_key = \"test-key\"\n"
    );
    std::fs::write(project_dir.join("cite.toml"), manifest).unwrap();

    let (_, stderr, ok) = run(&["deploy", "--path", project_dir.to_str().unwrap()]);
    assert!(!ok);
    assert!(stderr.contains("No build artifact"));
}
