use crate::*;

pub(crate) fn history_path(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<PathBuf, String> {
    Ok(resolve_git_path(root, GIT_CANON_CACHE_DIR)?
        .join(&expectation.id)
        .join(history_file_name()))
}

pub(crate) fn history_file_name() -> &'static str {
    "history.jsonl"
}

#[cfg(test)]
pub(crate) fn read_history_records(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Vec<CheckRecord>, String> {
    let path = history_path(root, expectation)?;
    read_history_records_from_path(&path)
}

pub(crate) fn read_history_records_from_path(path: &Path) -> Result<Vec<CheckRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|err| {
            format!(
                "failed to read {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<CheckRecord>(&line).map_err(|err| {
            format!(
                "invalid history JSON in {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

#[derive(Default)]
pub(crate) struct HistoryCache {
    pub(crate) paths: BTreeMap<(PathBuf, String), PathBuf>,
    pub(crate) records: BTreeMap<PathBuf, Vec<CheckRecord>>,
}

impl HistoryCache {
    pub(crate) fn new() -> HistoryCache {
        HistoryCache::default()
    }

    pub(crate) fn read_records(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Vec<CheckRecord>, String> {
        let path = self.path(root, expectation)?;
        if let Some(records) = self.records.get(&path) {
            return Ok(records.clone());
        }
        let records = read_history_records_from_path(&path)?;
        self.records.insert(path, records.clone());
        Ok(records)
    }

    pub(crate) fn path(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), expectation.id.clone());
        if let Some(path) = self.paths.get(&key) {
            return Ok(path.clone());
        }
        let path = history_path(root, expectation)?;
        self.paths.insert(key, path.clone());
        Ok(path)
    }
}
