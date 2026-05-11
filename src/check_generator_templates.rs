pub(crate) fn validate_generator_template(template: &str, number: usize) -> Result<(), String> {
    if template.matches("{content}").count() != 1 {
        return Err(format!(
            "expectation {} q_template must contain exactly one {{content}} placeholder",
            number
        ));
    }
    if template_contains_unknown_placeholder(template) {
        return Err(format!(
            "expectation {} q_template must not contain placeholders other than {{content}}",
            number
        ));
    }
    Ok(())
}

pub(crate) fn template_contains_unknown_placeholder(template: &str) -> bool {
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else {
            return false;
        };
        if &rest[..end] == "content" {
            rest = &rest[end + 1..];
            continue;
        }
        return true;
    }
    false
}
