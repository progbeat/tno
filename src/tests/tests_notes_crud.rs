use super::*;

#[test]
fn hash_is_ten_base64url_chars() {
    let hash = hash_key("src/lib.rs");
    assert_eq!(hash.len(), 10);
    assert!(hash
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
}

#[test]
fn path_creation_is_deterministic() {
    with_env("deterministic", |_| {
        let config = Config::from_env().unwrap();
        let first = ensure_note(&config, "a/b.rs").unwrap();
        let second = ensure_note(&config, "a/b.rs").unwrap();
        assert_eq!(first.path, second.path);
        assert!(first.path.exists());
    });
}

#[test]
fn write_and_append_preserve_metadata() {
    with_env("write-append", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        append_note(&config, "src/main.rs", "decision").unwrap();
        let note = note_for_key(&config, "src/main.rs").unwrap();
        let content = fs::read_to_string(note.path).unwrap();
        assert!(content.starts_with("<!-- canon key=\"src/main.rs\" hash=\""));
        assert!(content.contains("\nbody\n"));
        assert!(content.contains("decision"));
    });
}

#[test]
fn delete_removes_only_target() {
    with_env("delete", |_| {
        let config = Config::from_env().unwrap();
        let first = ensure_note(&config, "one").unwrap();
        let second = ensure_note(&config, "two").unwrap();
        delete_note(&config, "one").unwrap();
        assert!(!first.path.exists());
        assert!(second.path.exists());
        let index = fs::read_to_string(config.root.join("index.tsv")).unwrap();
        assert!(!index.contains("\tone\n"));
        assert!(index.contains("\ttwo\n"));
    });
}

#[test]
fn delete_verifies_note_before_removing_index_entry() {
    with_env("delete-bad-note", |_| {
        let config = Config::from_env().unwrap();
        let note = ensure_note(&config, "one").unwrap();
        fs::write(&note.path, "<!-- canon key=\"other\" hash=\"bad\" -->\n").unwrap();

        let err = delete_note(&config, "one").unwrap_err();

        assert!(err.contains("belongs to key"));
        assert!(note.path.exists());
        let index = fs::read_to_string(config.root.join("index.tsv")).unwrap();
        assert!(index.contains("\tone\n"));
    });
}

#[test]
fn collect_text_rejects_invalid_start_index() {
    let args = vec![OsString::from("one")];
    let err = collect_text(&args, 2).unwrap_err();
    assert!(err.contains("exceeds argument count"));
}

#[test]
fn note_keys_reject_index_separators() {
    with_env("bad-note-key", |_| {
        let config = Config::from_env().unwrap();
        assert!(write_note(&config, "bad\tkey", "body").is_err());
        assert!(write_note(&config, "bad\nkey", "body").is_err());
    });
}
