use crate::fs_util::ensure_dir;
use crate::hash::hash_key;
use crate::notes_header::{
    header, initial_content, normalize_body, validate_note_key, verify_note_key,
    verify_note_key_from_first_line,
};
use crate::notes_index::{remove_index, upsert_index, write_file_atomically};
use crate::notes_restore::{
    error_with_restore_context, restore_deleted_note_after_index_failure,
    restore_note_after_index_failure,
};
use crate::output::write_stdout;
use crate::time::unix_timestamp;
use crate::types::{Config, Note};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

const NOTE_LOG_MARKER: &str = "<!-- canon log v1 -->";

#[derive(Deserialize, Serialize)]
#[serde(tag = "op")]
enum NoteRecord {
    #[serde(rename = "write")]
    Write { text: String },
    #[serde(rename = "append")]
    Append { timestamp: u64, text: String },
}

pub(crate) fn ensure_note(config: &Config, key: &str) -> Result<Note, String> {
    ensure_dir(&config.root)?;
    let note = note_for_key(config, key)?;
    if note.path.exists() {
        verify_note_key(&note.path, key)?;
        return Ok(note);
    } else {
        let content = initial_content(key, &note.hash);
        write_file_atomically(&note.path, content.as_bytes())?;
    }
    if let Err(index_err) = upsert_index(config, &note.hash, key) {
        return Err(error_with_restore_context(
            index_err,
            restore_note_after_index_failure(&note.path, None),
        ));
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
    write_stdout(&materialize_note_content(&note, &content)?)
}

pub(crate) fn write_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    append_note_record(
        config,
        key,
        NoteRecord::Write {
            text: normalize_body(text),
        },
    )
}

pub(crate) fn append_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    append_note_record(
        config,
        key,
        NoteRecord::Append {
            timestamp: unix_timestamp()?,
            text: normalize_body(text),
        },
    )
}

fn append_note_record(config: &Config, key: &str, record: NoteRecord) -> Result<(), String> {
    ensure_dir(&config.root)?;
    let note = note_for_key(config, key)?;
    let record_content = encode_note_record(&record)?;
    if note.path.exists() {
        verify_note_key(&note.path, key)?;
        upsert_index(config, &note.hash, key)?;
        append_to_file(&note.path, record_content.as_bytes())?;
        return Ok(());
    }

    let mut content = initial_content(key, &note.hash);
    content.push_str(&record_content);
    write_file_atomically(&note.path, content.as_bytes())?;
    if let Err(index_err) = upsert_index(config, &note.hash, key) {
        return Err(error_with_restore_context(
            index_err,
            restore_note_after_index_failure(&note.path, None),
        ));
    }
    Ok(())
}

fn encode_note_record(record: &NoteRecord) -> Result<String, String> {
    let json = serde_json::to_string(record)
        .map_err(|err| format!("failed to encode note record: {}", err))?;
    Ok(format!("\n{}\n{}\n", NOTE_LOG_MARKER, json))
}

pub(crate) fn materialize_note_content(note: &Note, content: &str) -> Result<String, String> {
    let Some((log_start, records)) = find_note_log(note, content)? else {
        return Ok(content.to_string());
    };

    let mut output = content[..log_start].to_string();
    for record in records {
        match record {
            NoteRecord::Write { text } => {
                output = format!(
                    "{}{}\n",
                    header(&note.key, &note.hash),
                    normalize_body(&text)
                );
            }
            NoteRecord::Append { timestamp, text } => {
                if output.is_empty() {
                    output = initial_content(&note.key, &note.hash);
                }
                if !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&format!(
                    "\n## {}\n\n{}\n",
                    timestamp,
                    normalize_body(&text)
                ));
            }
        }
    }
    Ok(output)
}

fn find_note_log(note: &Note, content: &str) -> Result<Option<(usize, Vec<NoteRecord>)>, String> {
    let separator = format!("\n{}\n", NOTE_LOG_MARKER);
    let mut offset = 0;
    while let Some(relative_start) = content[offset..].find(&separator) {
        let separator_start = offset + relative_start;
        let log_start = separator_start + separator.len();
        if let Some(records) = parse_note_log_records(note, &content[log_start..])? {
            return Ok(Some((separator_start, records)));
        }
        offset = log_start;
    }
    Ok(None)
}

fn parse_note_log_records(note: &Note, text: &str) -> Result<Option<Vec<NoteRecord>>, String> {
    let mut records = Vec::new();
    for line in text.lines() {
        if line == NOTE_LOG_MARKER || line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(record) => records.push(record),
            Err(_) if records.is_empty() => return Ok(None),
            Err(err) => {
                return Err(format!(
                    "malformed note log record in {}: {}",
                    note.path.display(),
                    err
                ));
            }
        }
    }
    Ok((!records.is_empty()).then_some(records))
}

fn append_to_file(path: &Path, content: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    file.write_all(content)
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))
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
