use crate::*;

pub(crate) fn validate_generator_spec_title(
    path: &str,
    content: &str,
    number: usize,
) -> Result<(), String> {
    let title = spec_h1_title(content).ok_or_else(|| {
        format!(
            "expectation {} generated spec {} must contain an H1 title",
            number, path
        )
    })?;
    let normalized = normalize_spec_title(title).ok_or_else(|| {
        format!(
            "expectation {} generated spec {} H1 title must normalize to a filename",
            number, path
        )
    })?;
    let expected_file_name = format!("{}.md", normalized);
    let actual_file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "expectation {} generated spec path is invalid: {}",
                number, path
            )
        })?;
    if actual_file_name != expected_file_name {
        return Err(format!(
            "expectation {} generated spec {} H1 title normalizes to {}, but filename is {}",
            number, path, expected_file_name, actual_file_name
        ));
    }
    Ok(())
}

pub(crate) fn spec_h1_title(content: &str) -> Option<&str> {
    content.lines().find_map(|line| {
        let trimmed = line.trim_start();
        let title = trimmed.strip_prefix("# ")?;
        let title = title.trim();
        if title.is_empty() {
            None
        } else {
            Some(title)
        }
    })
}

pub(crate) fn normalize_spec_title(title: &str) -> Option<String> {
    let title = strip_markdown_inline_markup(title);
    let mut normalized = String::new();
    let mut pending_separator = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_separator && !normalized.is_empty() {
                normalized.push('-');
            }
            normalized.push(ch.to_ascii_lowercase());
            pending_separator = false;
        } else if ch.is_ascii() {
            pending_separator = true;
        } else {
            for lower in ch.to_lowercase() {
                if pending_separator && !normalized.is_empty() {
                    normalized.push('-');
                }
                normalized.push(lower);
                pending_separator = false;
            }
        }
    }
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(crate) fn strip_markdown_inline_markup(title: &str) -> String {
    let chars = title.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '[' {
            if let Some((label_end, link_end)) = markdown_link_bounds(&chars, index) {
                output.extend(chars[index + 1..label_end].iter().copied());
                index = link_end + 1;
                continue;
            }
        }
        if matches!(ch, '`' | '*' | '_') {
            index += 1;
            continue;
        }
        output.push(ch);
        index += 1;
    }
    output
}

pub(crate) fn markdown_link_bounds(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let label_end = chars
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, ch)| (*ch == ']').then_some(index))?;
    if chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let link_end = chars
        .iter()
        .enumerate()
        .skip(label_end + 2)
        .find_map(|(index, ch)| (*ch == ')').then_some(index))?;
    Some((label_end, link_end))
}
