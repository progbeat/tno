use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub(crate) fn ensure_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| format!("failed to create {}: {}", path.display(), err))
}

pub(crate) fn for_each_nonempty_line(
    path: &Path,
    mut visit: impl FnMut(usize, String) -> Result<(), String>,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line_number = index + 1;
        let line = line.map_err(|err| {
            format!(
                "failed to read {} line {}: {}",
                path.display(),
                line_number,
                err
            )
        })?;
        if !line.trim().is_empty() {
            visit(line_number, line)?;
        }
    }
    Ok(())
}

pub(crate) fn replace_file_with_temp(temp_path: &Path, path: &Path) -> Result<(), String> {
    fs::rename(temp_path, path).map_err(|err| {
        let _ = fs::remove_file(temp_path);
        format!(
            "failed to replace {} with {}: {}",
            path.display(),
            temp_path.display(),
            err
        )
    })
}
