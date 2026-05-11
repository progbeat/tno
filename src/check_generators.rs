use crate::*;

pub(crate) enum RawExpectationKind {
    Explicit,
    Generator,
    Invalid(&'static str),
}

pub(crate) fn raw_expectation_kind(item: &RawExpectationItem) -> RawExpectationKind {
    match (&item.q, &item.q_template, &item.path) {
        (Some(_), None, None) => RawExpectationKind::Explicit,
        (None, Some(_), Some(_)) => RawExpectationKind::Generator,
        (Some(_), Some(_), _) => {
            RawExpectationKind::Invalid("must not contain both q and q_template")
        }
        (Some(_), None, Some(_)) => {
            RawExpectationKind::Invalid("must not contain path on an explicit expectation")
        }
        (None, Some(_), None) => RawExpectationKind::Invalid("generator must contain path"),
        (None, None, Some(_)) => RawExpectationKind::Invalid("generator must contain q_template"),
        (None, None, None) => RawExpectationKind::Invalid("must contain q or q_template"),
    }
}
