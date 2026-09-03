//! JUnit XML reports: what the runner wrote, read for one scenario.
//!
//! The report is the one runner artifact telos parses. Its stdout is not
//! reproducible across machines; a JUnit file is a stable, structured
//! artifact nearly every runner can emit, and it is the only reading under
//! which a green verdict means "the scenario's test executed and passed"
//! rather than "the process exited 0" (`docs/contracts.md`, `test`).

use crate::ids::{RepoPath, ScenarioId};
use crate::witness::{names_scenario, scenario_pattern};

/// One parsed JUnit report: every `testcase` it holds, wherever nested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    cases: Vec<TestCase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestCase {
    name: String,
    status: CaseStatus,
}

/// A `testcase`'s outcome, read from its child elements: `failure` or
/// `error` is failed (the test ran and raised), `skipped` is skipped,
/// nothing is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseStatus {
    Passed,
    Failed,
    Skipped,
}

/// What a report says about one scenario, over the testcases named after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportVerdict {
    /// At least one passed, none failed, none skipped.
    Passed { passed: u32 },
    /// At least one failed.
    Failed { passed: u32, failed: u32 },
    /// Nothing proves the scenario ran.
    NotExecuted(NotExecuted),
}

/// Why a run proved nothing about a scenario. Each reason renders to one
/// frozen sentence ([`NotExecuted::message`]) that every surface reuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotExecuted {
    /// No file at the configured report path after the run.
    ReportMissing,
    /// The file exists but is not readable JUnit XML; carries the parser's
    /// own message.
    ReportInvalid(String),
    /// No `testcase` is named after the scenario.
    NoTestcase,
    /// Testcases named after the scenario exist, none failed, and this
    /// many were skipped.
    Skipped(u32),
}

impl Report {
    /// Parses JUnit XML. Every `testcase` element anywhere in the document
    /// counts, so a `testsuites` root and a bare `testsuite` root read the
    /// same. The error is `roxmltree`'s message, kept for the
    /// `ReportInvalid` wording.
    pub fn parse(xml: &str) -> Result<Report, String> {
        let document = roxmltree::Document::parse(xml).map_err(|error| error.to_string())?;
        let cases = document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "testcase")
            .map(|node| TestCase {
                name: node.attribute("name").unwrap_or_default().to_string(),
                status: case_status(node),
            })
            .collect();
        Ok(Report { cases })
    }

    /// The verdict for `scenario`, in the frozen order: any failure is
    /// `Failed`; otherwise any skip is `NotExecuted(Skipped)` -- a skipped
    /// twin next to a passed test is exactly the shape a zero-test green
    /// hides in; otherwise any pass is `Passed`; otherwise `NoTestcase`.
    pub fn verdict(&self, scenario: ScenarioId) -> ReportVerdict {
        let (mut passed, mut failed, mut skipped) = (0u32, 0u32, 0u32);
        for case in self
            .cases
            .iter()
            .filter(|case| names_scenario(&case.name, scenario))
        {
            match case.status {
                CaseStatus::Passed => passed += 1,
                CaseStatus::Failed => failed += 1,
                CaseStatus::Skipped => skipped += 1,
            }
        }
        if failed > 0 {
            ReportVerdict::Failed { passed, failed }
        } else if skipped > 0 {
            ReportVerdict::NotExecuted(NotExecuted::Skipped(skipped))
        } else if passed > 0 {
            ReportVerdict::Passed { passed }
        } else {
            ReportVerdict::NotExecuted(NotExecuted::NoTestcase)
        }
    }
}

fn case_status(node: roxmltree::Node) -> CaseStatus {
    let mut status = CaseStatus::Passed;
    for child in node.children().filter(|child| child.is_element()) {
        match child.tag_name().name() {
            "failure" | "error" => return CaseStatus::Failed,
            "skipped" => status = CaseStatus::Skipped,
            _ => {}
        }
    }
    status
}

impl NotExecuted {
    /// The frozen sentence for this reason, naming the report path and the
    /// scenario's `scn_NNNN` pattern (`docs/contracts.md`).
    pub fn message(&self, report: &RepoPath, scenario: ScenarioId) -> String {
        let pattern = scenario_pattern(scenario);
        match self {
            NotExecuted::ReportMissing => {
                format!("the runner did not write the report at `{report}`")
            }
            NotExecuted::ReportInvalid(error) => {
                format!("the report at `{report}` is not valid JUnit XML: {error}")
            }
            NotExecuted::NoTestcase => {
                format!("the report at `{report}` contains no testcase named after `{pattern}`")
            }
            NotExecuted::Skipped(count) => format!(
                "{count} testcase(s) named after `{pattern}` were skipped in the report at `{report}`"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nextest() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="3" failures="1" errors="0">
  <testsuite name="billing::tests" tests="3" failures="1" errors="0" skipped="0">
    <testcase name="scn_0091_issued_invoice_is_open" classname="billing::tests" time="0.001"/>
    <testcase name="scn_0107_full_payment_settles_the_invoice" classname="billing::tests" time="0.002">
      <failure message="assertion failed"><![CDATA[left: "open" right: "settled"]]></failure>
    </testcase>
    <testcase name="unrelated_helper_test" classname="billing::tests" time="0.000"/>
  </testsuite>
</testsuites>
"#
    }

    fn pytest() -> &'static str {
        r#"<?xml version="1.0" encoding="utf-8"?><testsuites><testsuite name="pytest" errors="1" failures="0" skipped="1" tests="3" time="0.03"><testcase classname="tests.test_billing" name="scn_0108_cancel_open_invoice" time="0.001"><skipped type="pytest.skip" message="not yet"/></testcase><testcase classname="tests.test_billing" name="scn_0109_refund" time="0.001"><error message="fixture error">boom</error></testcase><testcase classname="tests.test_billing" name="scn_0110_close" time="0.001"/></testsuite></testsuites>"#
    }

    fn jest_junit() -> &'static str {
        r#"<testsuite name="billing" tests="2" failures="0" errors="0" skipped="0">
  <testcase classname="billing cancel" name="scn_0108_cancel_open_invoice closes it" time="0.01"/>
  <testcase classname="billing cancel" name="scn_0108_cancel_open_invoice keeps the balance" time="0.01"/>
</testsuite>"#
    }

    #[test]
    fn a_passed_testcase_named_after_the_scenario_is_passed() {
        let report = Report::parse(nextest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(91)),
            ReportVerdict::Passed { passed: 1 }
        );
    }

    #[test]
    fn a_failure_child_is_failed() {
        let report = Report::parse(nextest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(107)),
            ReportVerdict::Failed {
                passed: 0,
                failed: 1
            }
        );
    }

    #[test]
    fn an_error_child_is_failed_too() {
        let report = Report::parse(pytest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(109)),
            ReportVerdict::Failed {
                passed: 0,
                failed: 1
            }
        );
    }

    #[test]
    fn a_skipped_testcase_is_not_executed() {
        let report = Report::parse(pytest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(108)),
            ReportVerdict::NotExecuted(NotExecuted::Skipped(1))
        );
    }

    #[test]
    fn a_pass_next_to_a_skip_is_still_not_executed() {
        let xml = r#"<testsuite><testcase name="scn_0108_a"/><testcase name="scn_0108_b"><skipped/></testcase></testsuite>"#;
        assert_eq!(
            Report::parse(xml).unwrap().verdict(ScenarioId(108)),
            ReportVerdict::NotExecuted(NotExecuted::Skipped(1))
        );
    }

    #[test]
    fn a_failure_outranks_a_skip() {
        let xml = r#"<testsuite><testcase name="scn_0108_a"><failure/></testcase><testcase name="scn_0108_b"><skipped/></testcase><testcase name="scn_0108_c"/></testsuite>"#;
        assert_eq!(
            Report::parse(xml).unwrap().verdict(ScenarioId(108)),
            ReportVerdict::Failed {
                passed: 1,
                failed: 1
            }
        );
    }

    #[test]
    fn no_matching_testcase_is_not_executed() {
        let report = Report::parse(nextest()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(108)),
            ReportVerdict::NotExecuted(NotExecuted::NoTestcase)
        );
    }

    #[test]
    fn a_testsuite_root_counts_every_matching_case() {
        let report = Report::parse(jest_junit()).unwrap();
        assert_eq!(
            report.verdict(ScenarioId(108)),
            ReportVerdict::Passed { passed: 2 }
        );
    }

    #[test]
    fn matching_respects_the_identifier_boundary() {
        let xml = r#"<testsuite><testcase name="descn_0108x"/><testcase name="xscn_0108"/><testcase name="test::scn_0108_y"/></testsuite>"#;
        assert_eq!(
            Report::parse(xml).unwrap().verdict(ScenarioId(108)),
            ReportVerdict::Passed { passed: 1 }
        );
    }

    #[test]
    fn malformed_xml_is_an_error_carrying_the_parser_message() {
        let error = Report::parse("<testsuites><testcase name=\"scn_0001\"").unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn every_reason_has_its_frozen_wording() {
        let report = RepoPath::new("target/telos-report.xml");
        let scenario = ScenarioId(108);
        assert_eq!(
            NotExecuted::ReportMissing.message(&report, scenario),
            "the runner did not write the report at `target/telos-report.xml`"
        );
        assert_eq!(
            NotExecuted::ReportInvalid("unexpected end of stream".to_string())
                .message(&report, scenario),
            "the report at `target/telos-report.xml` is not valid JUnit XML: unexpected end of stream"
        );
        assert_eq!(
            NotExecuted::NoTestcase.message(&report, scenario),
            "the report at `target/telos-report.xml` contains no testcase named after `scn_0108`"
        );
        assert_eq!(
            NotExecuted::Skipped(2).message(&report, scenario),
            "2 testcase(s) named after `scn_0108` were skipped in the report at `target/telos-report.xml`"
        );
    }
}
