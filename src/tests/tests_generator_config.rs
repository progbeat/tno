use super::*;

#[test]
fn generator_template_rejects_every_non_content_brace_pair() {
    assert!(
        validate_generator_template("Example JSON: {\"ok\": true}\n{content}\nDone", 1).is_err()
    );
    assert!(validate_generator_template("{content}\n{name}", 1).is_err());
    assert!(validate_generator_template("{content}\n{}", 1).is_err());
    assert!(validate_generator_template("{content}\n{ name }", 1).is_err());
    assert!(validate_generator_template("{content}\n{!}", 1).is_err());
    assert!(validate_generator_template("{content}\n{foo bar}", 1).is_err());
}

#[test]
fn generator_specs_must_have_h1_matching_normalized_filename() {
    let root = temp_home("generator-h1-title");
    fs::create_dir_all(root.join("specs")).unwrap();
    fs::write(root.join("specs/cache-policy.md"), "# `Cache` Policy\n").unwrap();
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - path: "specs/*.md"
    q_template: "{content}"
    a: "yes"
"#;
    let mut cache = RepoInspectionCache::new();
    let config =
        parse_check_config_content_with_root(&root, Path::new("check.yml"), yaml, &mut cache)
            .unwrap();
    assert_eq!(config.expectations.len(), 1);

    fs::write(root.join("specs/wrong.md"), "# Right\n").unwrap();
    let mut cache = RepoInspectionCache::new();
    let error =
        parse_check_config_content_with_root(&root, Path::new("check.yml"), yaml, &mut cache)
            .unwrap_err();
    assert!(error.contains("H1 title normalizes to right.md"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn generator_spec_title_normalization_matches_filename_rule() {
    assert_eq!(
        normalize_spec_title("`Check` [Output](docs/check-output.md)"),
        Some("check-output".to_string())
    );
    assert_eq!(
        normalize_spec_title("API (v2): Über Cache"),
        Some("api-v2-über-cache".to_string())
    );
}

#[test]
fn generator_specs_must_have_h1_title() {
    let root = temp_home("generator-missing-h1");
    fs::create_dir_all(root.join("specs")).unwrap();
    fs::write(root.join("specs/cache.md"), "No title\n").unwrap();
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - path: "specs/*.md"
    q_template: "{content}"
    a: "yes"
"#;
    let mut cache = RepoInspectionCache::new();
    let error =
        parse_check_config_content_with_root(&root, Path::new("check.yml"), yaml, &mut cache)
            .unwrap_err();
    assert!(error.contains("must contain an H1 title"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn generator_paths_support_multiple_filename_wildcards() {
    let root = temp_home("generator-multi-wildcard");
    fs::create_dir_all(root.join("specs")).unwrap();
    fs::write(root.join("specs/cache.policy.md"), "cache").unwrap();
    fs::write(root.join("specs/cache.notes.txt"), "notes").unwrap();
    fs::write(root.join("specs/log.policy.md"), "log").unwrap();

    assert_eq!(
        expand_filesystem_generator_paths(&root, "specs/c*.*.md").unwrap(),
        vec!["specs/cache.policy.md".to_string()]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_inspection_cache_reuses_generator_path_expansion() {
    let root = temp_home("generator-path-cache");
    fs::create_dir_all(root.join("specs")).unwrap();
    fs::write(root.join("specs/a.md"), "# A\n").unwrap();
    let mut cache = RepoInspectionCache::new();

    let first = cache
        .generator_paths(&root, Path::new("check.yml"), "specs/*.md", false)
        .unwrap();
    fs::write(root.join("specs/b.md"), "# B\n").unwrap();
    let second = cache
        .generator_paths(&root, Path::new("check.yml"), "specs/*.md", false)
        .unwrap();

    assert_eq!(first, vec!["specs/a.md".to_string()]);
    assert_eq!(second, first);
    let _ = fs::remove_dir_all(root);
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn generator_paths_ignore_unmatched_non_utf8_names() {
    use std::os::unix::ffi::OsStringExt;

    let root = temp_home("generator-non-utf8");
    fs::create_dir_all(root.join("specs")).unwrap();
    fs::write(root.join("specs/a.md"), "ok").unwrap();
    fs::write(
        root.join("specs").join(std::ffi::OsString::from_vec(vec![
            b'n', b'o', b't', b'e', b'-', 0xff, b'.', b't', b'x', b't',
        ])),
        "ignored",
    )
    .unwrap();
    assert_eq!(
        expand_filesystem_generator_paths(&root, "specs/*.md").unwrap(),
        vec!["specs/a.md".to_string()]
    );
    fs::write(
        root.join("specs").join(std::ffi::OsString::from_vec(vec![
            b's', b'p', b'e', b'c', b'-', 0xff, b'.', b'm', b'd',
        ])),
        "matched",
    )
    .unwrap();
    assert!(expand_filesystem_generator_paths(&root, "specs/*.md").is_err());
    let _ = fs::remove_dir_all(root);
}
