// The evaluator protocol intentionally makes the top-level key order part of
// the response format so logs, stdout, and human review stay predictable.
// serde_json validates field names and types below, but it does not expose
// source object order through the normal typed-deserialization path, so this
// small scanner only checks the top-level object envelope before serde parses
// the actual field values.
pub(crate) fn validate_evaluator_response_key_order(text: &str) -> Result<(), String> {
    let keys = top_level_json_object_keys(text)?;
    if keys == ["answer", "evidence", "scope"] {
        Ok(())
    } else {
        Err(format!(
            "evaluator JSON response must contain keys in order answer, evidence, scope; got {}",
            keys.join(", ")
        ))
    }
}

pub(crate) fn top_level_json_object_keys(text: &str) -> Result<Vec<String>, String> {
    let bytes = text.as_bytes();
    let mut index = skip_json_ws(bytes, 0);
    if bytes.get(index) != Some(&b'{') {
        return Err("evaluator response must be a JSON object".to_string());
    }
    index += 1;
    let mut keys = Vec::new();
    loop {
        index = skip_json_ws(bytes, index);
        if bytes.get(index) == Some(&b'}') {
            index += 1;
            break;
        }
        let (key, next) = parse_json_string_at(bytes, index)?;
        keys.push(key);
        index = skip_json_ws(bytes, next);
        if bytes.get(index) != Some(&b':') {
            return Err("evaluator JSON object key must be followed by ':'".to_string());
        }
        index = skip_json_value(bytes, index + 1)?;
        index = skip_json_ws(bytes, index);
        match bytes.get(index) {
            Some(b',') => index += 1,
            Some(b'}') => {
                index += 1;
                break;
            }
            _ => return Err("evaluator JSON object contains trailing content".to_string()),
        }
    }
    if skip_json_ws(bytes, index) != bytes.len() {
        return Err("evaluator response must not contain surrounding prose".to_string());
    }
    Ok(keys)
}

pub(crate) fn skip_json_ws(bytes: &[u8], mut index: usize) -> usize {
    while matches!(bytes.get(index), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        index += 1;
    }
    index
}

pub(crate) fn parse_json_string_at(
    bytes: &[u8],
    mut index: usize,
) -> Result<(String, usize), String> {
    if bytes.get(index) != Some(&b'"') {
        return Err("expected JSON string key".to_string());
    }
    let start = index;
    index += 1;
    let mut escaped = false;
    while let Some(byte) = bytes.get(index).copied() {
        index += 1;
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'"' => {
                let raw = std::str::from_utf8(&bytes[start..index])
                    .map_err(|_| "JSON string key must be valid UTF-8".to_string())?;
                let decoded = serde_json::from_str::<String>(raw)
                    .map_err(|err| format!("invalid JSON string key: {}", err))?;
                return Ok((decoded, index));
            }
            b'\\' => escaped = true,
            0x00..=0x1f => {
                return Err("JSON string key contains an unescaped control character".to_string())
            }
            _ => {}
        }
    }
    Err("unterminated JSON string".to_string())
}

pub(crate) fn skip_json_value(bytes: &[u8], mut index: usize) -> Result<usize, String> {
    index = skip_json_ws(bytes, index);
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut saw_scalar = false;
    while let Some(byte) = bytes.get(index).copied() {
        if in_string {
            index += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
                saw_scalar = true;
            }
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                index += 1;
            }
            b'{' | b'[' => {
                stack.push(byte);
                index += 1;
            }
            b'}' => {
                if stack.last() == Some(&b'{') {
                    stack.pop();
                    index += 1;
                    if stack.is_empty() {
                        saw_scalar = true;
                    }
                } else if stack.is_empty() && saw_scalar {
                    return Ok(index);
                } else {
                    return Err("unbalanced JSON object".to_string());
                }
            }
            b']' => {
                if stack.last() == Some(&b'[') {
                    stack.pop();
                    index += 1;
                    if stack.is_empty() {
                        saw_scalar = true;
                    }
                } else {
                    return Err("unbalanced JSON array".to_string());
                }
            }
            b',' if stack.is_empty() && saw_scalar => return Ok(index),
            b' ' | b'\n' | b'\r' | b'\t' if stack.is_empty() && saw_scalar => return Ok(index),
            _ => {
                saw_scalar = true;
                index += 1;
            }
        }
        if stack.is_empty() && saw_scalar && !in_string {
            let next = skip_json_ws(bytes, index);
            if matches!(bytes.get(next), Some(b',' | b'}')) {
                return Ok(next);
            }
        }
    }
    if saw_scalar && stack.is_empty() && !in_string {
        Ok(index)
    } else {
        Err("unterminated JSON value".to_string())
    }
}
