use super::*;

#[test]
fn generator_template_requires_exactly_one_content_placeholder() {
    assert!(
        validate_generator_template("Example JSON: {\"ok\": true}\n{content}\nDone", 1).is_ok()
    );
    assert!(validate_generator_template("{content}\n{name}", 1).is_ok());
    assert!(validate_generator_template("no placeholder", 1).is_err());
    assert!(validate_generator_template("{content}\n{content}", 1).is_err());
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

#[test]
fn expectation_include_expands_yaml_list_relative_to_config_file() {
    let root = temp_home("expectation-include");
    fs::create_dir_all(root.join("checks/expects")).unwrap();
    fs::write(
        root.join("checks/expects/project.yml"),
        r#"
- q: "Included?"
  a: "yes"
  cooldown: 7d
  thinking: high
"#,
    )
    .unwrap();
    let mut cache = RepoInspectionCache::new();
    let config = parse_check_config_content_with_root(
        &root,
        Path::new("checks/check.yml"),
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - include: "expects/*.yml"
  - q: "Local?"
    a: "no"
"#,
        &mut cache,
    )
    .unwrap();

    assert_eq!(config.expectations.len(), 2);
    assert_eq!(config.expectations[0].q, "Included?");
    assert_eq!(config.expectations[0].a, "yes");
    assert_eq!(config.expectations[0].cooldown.as_deref(), Some("7d"));
    assert_eq!(config.expectations[0].thinking.as_deref(), Some("high"));
    assert_eq!(config.expectations[1].q, "Local?");
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
