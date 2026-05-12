use crate::*;

pub(crate) fn upsert_index(config: &Config, hash: &str, key: &str) -> Result<(), String> {
    ensure_dir(&config.root)?;
    let _lock = lock_index(config)?;
    let path = config.root.join("index.tsv");
    let mut entries = read_index(&path)?;
    entries.retain(|(_, existing_key)| existing_key != key);
    entries.push((hash.to_string(), key.to_string()));
    write_index(&path, &entries)
}

pub(crate) fn remove_index(config: &Config, _hash: &str, key: &str) -> Result<(), String> {
    ensure_dir(&config.root)?;
    let _lock = lock_index(config)?;
    let path = config.root.join("index.tsv");
    let mut entries = read_index(&path)?;
    entries.retain(|(_, existing_key)| existing_key != key);
    write_index(&path, &entries)
}

pub(crate) struct IndexLock {
    path: PathBuf,
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn stale_index_lock_age(age: Duration) -> bool {
    age >= Duration::from_secs(INDEX_LOCK_STALE_AFTER_SECS)
}

pub(crate) fn index_lock_is_stale(path: &Path) -> Result<bool, String> {
    let metadata = fs::metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    let modified = metadata
        .modified()
        .map_err(|err| format!("failed to inspect mtime for {}: {}", path.display(), err))?;
    let age = modified
        .elapsed()
        .map_err(|err| format!("failed to inspect age for {}: {}", path.display(), err))?;
    Ok(stale_index_lock_age(age))
}

pub(crate) fn create_index_lock(path: &Path) -> Result<(), io::Error> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
}

pub(crate) fn remove_stale_index_lock(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "failed to remove stale lock {}: {}",
            path.display(),
            err
        )),
    }
}

pub(crate) fn lock_index(config: &Config) -> Result<IndexLock, String> {
    ensure_dir(&config.root)?;
    let path = config.root.join("index.tsv.lock");
    match create_index_lock(&path) {
        Ok(()) => Ok(IndexLock { path }),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            if !index_lock_is_stale(&path)? {
                return Err(format!(
                    "failed to lock {}: lock is already held",
                    path.display()
                ));
            }
            remove_stale_index_lock(&path)?;
            create_index_lock(&path)
                .map_err(|err| format!("failed to lock {}: {}", path.display(), err))?;
            Ok(IndexLock { path })
        }
        Err(err) => Err(format!("failed to lock {}: {}", path.display(), err)),
    }
}

pub(crate) fn read_index(path: &Path) -> Result<Vec<(String, String)>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for_each_nonempty_line(path, |line_number, line| {
        let Some((hash, key)) = line.split_once('\t') else {
            return Err(format!(
                "malformed index line {} in {}",
                line_number,
                path.display()
            ));
        };
        validate_index_entry(hash, key).map_err(|err| {
            format!(
                "malformed index line {} in {}: {}",
                line_number,
                path.display(),
                err
            )
        })?;
        entries.push((hash.to_string(), key.to_string()));
        Ok(())
    })?;
    Ok(entries)
}

pub(crate) fn validate_index_entry(hash: &str, key: &str) -> Result<(), String> {
    if hash.is_empty() {
        return Err("hash must not be empty".to_string());
    }
    if hash.contains('\t') || hash.contains('\n') || hash.contains('\r') {
        return Err("hash must not contain tabs or newlines".to_string());
    }
    validate_note_key(key)
}

pub(crate) fn write_index(path: &Path, entries: &[(String, String)]) -> Result<(), String> {
    let mut content = String::new();
    for (hash, key) in entries {
        validate_index_entry(hash, key)
            .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
        content.push_str(hash);
        content.push('\t');
        content.push_str(key);
        content.push('\n');
    }
    write_file_atomically(path, content.as_bytes())
}

pub(crate) fn write_file_atomically(path: &Path, content: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid file path: {}", path.display()))?;
    let temp_path = path.with_file_name(format!(".{}.{}.tmp", file_name, process::id()));
    fs::write(&temp_path, content)
        .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
    replace_file_with_temp(&temp_path, path)
}
