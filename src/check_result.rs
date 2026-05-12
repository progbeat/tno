use crate::*;

pub(crate) fn report_passed_count(report: &CheckRunReport) -> usize {
    report
        .records
        .iter()
        .filter(|record| record.passed())
        .count()
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

pub(crate) fn report_output_skipped_count(report: &CheckRunReport) -> usize {
    debug_assert!(report.records.len() <= report.selected + report.skipped);
    debug_assert!(report.silent <= report.skipped);
    report.skipped
}

impl CheckRecord {
    pub(crate) fn passed(&self) -> bool {
        self.result == CheckResult::Pass
    }
}
