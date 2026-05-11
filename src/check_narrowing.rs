use crate::*;

pub(crate) fn scope_narrowing_log_fields(
    number: usize,
    original_scope: &[String],
    proposed_scope: &[String],
    accepted: bool,
    initial: &CheckRecord,
    verification: &CheckRecord,
) -> Vec<(&'static str, Value)> {
    vec![
        ("number", json!(number)),
        ("originalScope", json!(original_scope)),
        ("proposedScope", json!(proposed_scope)),
        ("accepted", json!(accepted)),
        ("initialObserved", json!(initial.observed.clone())),
        ("initialEvidence", json!(initial.evidence.clone())),
        ("verificationObserved", json!(verification.observed.clone())),
        ("verificationEvidence", json!(verification.evidence.clone())),
    ]
}
