use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

pub(crate) fn ensure_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| format!("failed to create {}: {}", path.display(), err))
}

pub(crate) fn for_each_nonempty_line(
    path: &Path,
    mut visit: impl FnMut(usize, String) -> Result<(), String>,
) -> Result<(), String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("failed to open {}: {}", path.display(), err)),
    };
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

pub(crate) fn crossed_size_compaction_bucket(
    previous_size: u64,
    current_size: u64,
    min_size: u64,
) -> bool {
    size_compaction_bucket(current_size, min_size) > size_compaction_bucket(previous_size, min_size)
}

fn size_compaction_bucket(size: u64, min_size: u64) -> u32 {
    if min_size == 0 || size < min_size {
        return 0;
    }
    let units = size / min_size;
    u64::BITS - units.leading_zeros()
}
