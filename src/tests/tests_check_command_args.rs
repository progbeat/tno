use super::*;

#[test]
fn check_command_accepts_custom_config_option() {
    let parsed = parse_check_command_args(&[
        "--config".into(),
        "alt.yml".into(),
        "--all".into(),
        "2".into(),
    ])
    .unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("alt.yml"));
    assert_eq!(
        parsed.option_args,
        vec![OsString::from("--all"), OsString::from("2")]
    );

    let parsed = parse_check_command_args(&["-c".into(), "old.yml".into()]).unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("old.yml"));

    let parsed = parse_check_command_args(&["--config=old.yml".into()]).unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("old.yml"));

    assert!(parse_check_command_args(&["-c".into()]).is_err());
    assert!(
        parse_check_command_args(&["-c".into(), "a.yml".into(), "--config=b.yml".into()]).is_err()
    );
    assert!(parse_check_command_args(&["-c".into(), "../outside.yml".into()]).is_err());
    assert!(parse_check_command_args(&["-c".into(), "/tmp/outside.yml".into()]).is_err());
}

#[test]
fn check_command_accepts_query_mode() {
    let parsed = parse_check_command_args(&["-q".into(), "Question?".into()]).unwrap();
    assert_eq!(parsed.query.as_deref(), Some("Question?"));
    assert!(parsed.query_scope.is_empty());
    assert_eq!(parsed.config_path, PathBuf::from(CHECK_PATH));
    assert!(parsed.option_args.is_empty());

    let parsed = parse_check_command_args(&[
        "--config".into(),
        "alt.yml".into(),
        "-q".into(),
        "Question?".into(),
    ])
    .unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("alt.yml"));
    assert_eq!(parsed.query.as_deref(), Some("Question?"));
    assert!(parsed.query_scope.is_empty());

    assert!(parse_check_command_args(&["-q".into()]).is_err());
    assert!(parse_check_command_args(&[
        "-q".into(),
        "Question?".into(),
        "-q".into(),
        "Again?".into()
    ])
    .is_err());
    assert!(parse_check_command_args(&["-q".into(), "Question?".into(), "1".into()]).is_err());
    assert!(
        parse_check_command_args(&["-q".into(), "Question?".into(), "--ignore-cache".into()])
            .is_err()
    );
    assert!(parse_check_command_args(&["-q".into(), "Question?".into(), "--all".into()]).is_err());
}

#[test]
fn check_command_accepts_query_scope_option() {
    let parsed = parse_check_command_args(&[
        "-s".into(),
        "./src".into(),
        "--scope".into(),
        "tests".into(),
        "-q".into(),
        "Question?".into(),
        "--scope=README.md".into(),
    ])
    .unwrap();

    assert_eq!(parsed.query.as_deref(), Some("Question?"));
    assert_eq!(
        parsed.query_scope,
        vec![
            "src".to_string(),
            "tests".to_string(),
            "README.md".to_string()
        ]
    );
    assert!(parsed.option_args.is_empty());

    assert!(parse_check_command_args(&["-q".into(), "Question?".into(), "-s".into()]).is_err());
    assert!(
        parse_check_command_args(&["-q".into(), "Question?".into(), "--scope=".into()]).is_err()
    );
    assert!(
        parse_check_command_args(&["-q".into(), "Question?".into(), "--scope".into()]).is_err()
    );
    assert!(parse_check_command_args(&[
        "-q".into(),
        "Question?".into(),
        "-s".into(),
        "../src".into()
    ])
    .is_err());
    assert!(parse_check_command_args(&["-s".into(), "src".into()]).is_err());
}
