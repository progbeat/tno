use crate::*;

pub(crate) fn report_passed_count(report: &CheckRunReport) -> usize {
    report
        .records
        .iter()
        .filter(|record| record.passed())
        .count()
        .saturating_sub(report.skipped)
}

pub(crate) fn report_failed_count(report: &CheckRunReport) -> usize {
    report
        .records
        .iter()
        .filter(|record| !record.passed() && !record_requires_human_review(record))
        .count()
}

pub(crate) fn report_error_count(report: &CheckRunReport) -> usize {
    report
        .records
        .iter()
        .filter(|record| record_requires_human_review(record))
        .count()
}

impl CheckRecord {
    pub(crate) fn passed(&self) -> bool {
        self.result == CheckResult::Pass
    }
}
