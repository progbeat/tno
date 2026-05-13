use crate::*;

pub(crate) enum RawExpectationKind {
    Explicit,
    Generator,
    Include,
    Invalid(&'static str),
}

pub(crate) fn raw_expectation_kind(item: &RawExpectationItem) -> RawExpectationKind {
    match (
        &item.q,
        &item.q_template,
        &item.path,
        &item.include,
        &item.a,
    ) {
        (Some(_), None, None, None, Some(_)) => RawExpectationKind::Explicit,
        (None, Some(_), Some(_), None, Some(_)) => RawExpectationKind::Generator,
        (None, None, None, Some(_), None) => RawExpectationKind::Include,
        (_, _, _, Some(_), Some(_)) => {
            RawExpectationKind::Invalid("include item must not contain a")
        }
        (Some(_), _, _, Some(_), _) => {
            RawExpectationKind::Invalid("include item must not contain q")
        }
        (_, Some(_), _, Some(_), _) => {
            RawExpectationKind::Invalid("include item must not contain q_template")
        }
        (_, _, Some(_), Some(_), _) => {
            RawExpectationKind::Invalid("include item must not contain path")
        }
        (Some(_), Some(_), _, _, _) => {
            RawExpectationKind::Invalid("must not contain both q and q_template")
        }
        (Some(_), None, Some(_), _, _) => {
            RawExpectationKind::Invalid("must not contain path on an explicit expectation")
        }
        (Some(_), None, None, None, None) => RawExpectationKind::Invalid("must contain a"),
        (None, Some(_), None, _, _) => RawExpectationKind::Invalid("generator must contain path"),
        (None, Some(_), Some(_), None, None) => RawExpectationKind::Invalid("must contain a"),
        (None, None, Some(_), _, _) => {
            RawExpectationKind::Invalid("generator must contain q_template")
        }
        (None, None, None, None, Some(_)) => {
            RawExpectationKind::Invalid("must contain q or q_template")
        }
        (None, None, None, None, None) => {
            RawExpectationKind::Invalid("must contain q, q_template, or include")
        }
    }
}
