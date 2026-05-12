use crate::*;

pub(crate) fn ensure_note(config: &Config, key: &str) -> Result<Note, String> {
    ensure_dir(&config.root)?;
    let note = note_for_key(config, key)?;
    let created = !note.path.exists();
    if note.path.exists() {
        verify_note_key(&note.path, key)?;
    } else {
        let content = initial_content(key, &note.hash);
        write_file_atomically(&note.path, content.as_bytes())?;
    }
    if let Err(index_err) = upsert_index(config, &note.hash, key) {
        if created {
            return Err(error_with_restore_context(
                index_err,
                restore_note_after_index_failure(&note.path, None),
            ));
        }
        return Err(index_err);
    }
    Ok(note)
}

pub(crate) fn note_for_key(config: &Config, key: &str) -> Result<Note, String> {
    validate_note_key(key)?;
    let hash = hash_key(key);
    let path = config.root.join(format!("{}.md", hash));
    Ok(Note {
        key: key.to_string(),
        hash,
        path,
    })
}

pub(crate) fn read_note(config: &Config, key: &str) -> Result<(), String> {
    let note = note_for_key(config, key)?;
    if !note.path.exists() {
        return Err(format!("canon not found for key: {}", key));
    }
    let content = fs::read_to_string(&note.path)
        .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?;
    verify_note_key_from_first_line(&note.path, content.lines().next().unwrap_or(""), key)?;
    print!("{}", content);
    Ok(())
}

pub(crate) fn write_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    change_note(config, key, |note, _original| {
        Ok(format!(
            "{}{}\n",
            header(&note.key, &note.hash),
            normalize_body(text)
        ))
    })
}

pub(crate) fn append_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    change_note(config, key, |note, original| {
        let timestamp = unix_timestamp()?;
        let section = format!("\n## {}\n\n{}\n", timestamp, normalize_body(text));
        let mut content = match original {
            Some(bytes) => String::from_utf8(bytes.to_vec())
                .map_err(|_| format!("{} must be valid UTF-8", note.path.display()))?,
            None => initial_content(key, &note.hash),
        };
        content.push_str(&section);
        Ok(content)
    })
}

fn change_note(
    config: &Config,
    key: &str,
    build_content: impl FnOnce(&Note, Option<&[u8]>) -> Result<String, String>,
) -> Result<(), String> {
    ensure_dir(&config.root)?;
    let note = note_for_key(config, key)?;
    let original = if note.path.exists() {
        verify_note_key(&note.path, key)?;
        Some(
            fs::read(&note.path)
                .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?,
        )
    } else {
        None
    };
    let content = build_content(&note, original.as_deref())?;
    write_file_atomically(&note.path, content.as_bytes())?;
    if let Err(index_err) = upsert_index(config, &note.hash, key) {
        return Err(error_with_restore_context(
            index_err,
            restore_note_after_index_failure(&note.path, original.as_deref()),
        ));
    }
    Ok(())
}

pub(crate) fn delete_note(config: &Config, key: &str) -> Result<(), String> {
    let note = note_for_key(config, key)?;
    if note.path.exists() {
        verify_note_key(&note.path, key)?;
        let original = fs::read(&note.path)
            .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?;
        fs::remove_file(&note.path)
            .map_err(|err| format!("failed to delete {}: {}", note.path.display(), err))?;
        if let Err(index_err) = remove_index(config, &note.hash, key) {
            return Err(error_with_restore_context(
                index_err,
                restore_deleted_note_after_index_failure(&note.path, &original),
            ));
        }
    } else {
        remove_index(config, &note.hash, key)?;
    }
    Ok(())
}
