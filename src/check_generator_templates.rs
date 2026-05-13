pub(crate) fn validate_generator_template(template: &str, number: usize) -> Result<(), String> {
    if template.matches("{content}").count() != 1 {
        return Err(format!(
            "expectation {} q_template must contain exactly one {{content}} placeholder",
            number
        ));
    }
    Ok(())
}
