use crate::check_types::{CheckRecord, CheckResult, CheckRunReport};

pub(crate) fn report_output_skipped_count(report: &CheckRunReport) -> usize {
    debug_assert!(report.records.len() <= report.selected + report.skipped);
    debug_assert!(report.silent <= report.skipped);
    // The check-output contract reports final non-selected expectations. That
    // includes CLI-selector exclusions plus expectations deselected later by
    // cooldown or silent exact-cache passes.
    report.skipped
}

impl CheckRecord {
    pub(crate) fn passed(&self) -> bool {
        self.result == CheckResult::Pass
    }
}
