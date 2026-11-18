//! `walk_scenarios` over manifest fixtures, offline.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use trogon_atlas_client::walk::{self, Model, Severity, WalkReport};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/walk")
        .join(name)
}

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(path)
}

fn model() -> Model {
    Model::load_paths(&[fixture("model.yaml")]).unwrap()
}

fn walk_text(cases: &str) -> WalkReport {
    let cases = walk::parse_cases(cases, "inline").unwrap();
    walk::walk(&model(), &cases)
}

/// `(case name, step, code)` of every error and warning, in report order.
fn codes(report: &WalkReport) -> Vec<(String, Option<usize>, &'static str)> {
    report
        .findings
        .iter()
        .filter(|f| f.severity != Severity::Info)
        .map(|f| {
            let name = f.case.rsplit('/').next().unwrap_or_default().to_string();
            (name, f.step, f.code)
        })
        .collect()
}

fn case(
    name: &str,
    step: Option<usize>,
    code: &'static str,
) -> (String, Option<usize>, &'static str) {
    (name.to_string(), step, code)
}

const HEADER: &str = "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Specification\nmetadata:\n  namespace: walk-reviews\n  name: inline\nspec:\n";

fn inline(spec: &str) -> String {
    format!("{HEADER}{spec}")
}

#[test]
fn fixture_cases_report_each_gap_where_it_happens() {
    let cases = walk::read_case_paths(&[fixture("cases.yaml")]).unwrap();
    let report = walk::walk(&model(), &cases);
    assert_eq!(
        codes(&report),
        vec![
            // approve-review needs `score`; review-queue never receives it.
            case(
                "approve-review-42",
                Some(2),
                "WALK_COMMAND_FIELD_UNSUPPLIED"
            ),
            case(
                "approve-review-42",
                Some(2),
                "WALK_PROJECTION_FIELD_MISSING"
            ),
            case("approve-review-42", None, "WALK_READ_MODEL_MISMATCH"),
            // Rejecting an approved review is accepted by the model.
            // The walk follows the model, so the accepted step 2 already
            // lands after the terminal event.
            case("reject-after-approve", Some(2), "WALK_REJECT_UNMODELED"),
            case("reject-after-approve", Some(2), "WALK_SCENARIO_MISSING"),
            case(
                "reject-after-approve",
                Some(2),
                "WALK_ACCEPTED_AFTER_TERMINAL"
            ),
            case("reject-after-approve", Some(3), "WALK_SCENARIO_MISSING"),
            case(
                "reject-after-approve",
                Some(3),
                "WALK_ACCEPTED_AFTER_TERMINAL"
            ),
        ],
        "{report:#?}"
    );
    let passed: Vec<(&str, bool)> = report
        .cases
        .iter()
        .map(|c| (c.case.as_str(), c.passed))
        .collect();
    assert_eq!(
        passed,
        vec![
            ("walk-reviews/approve-review-42", false),
            ("walk-reviews/reject-review-7", true),
            ("walk-reviews/reject-after-approve", false),
        ]
    );
    assert!(report.has_errors());
    // Every storyboard and scenario was exercised by some case.
    assert_eq!(report.coverage.storyboards_walked, 2);
    assert_eq!(report.coverage.storyboards_total, 2);
    assert_eq!(
        report.coverage.scenarios_walked,
        report.coverage.scenarios_total
    );
}

#[test]
fn findings_use_the_validation_issue_shape() {
    let cases = walk::read_case_paths(&[fixture("cases.yaml")]).unwrap();
    let report = walk::walk(&model(), &cases);
    let json = serde_json::to_value(&report).unwrap();
    let first = &json["findings"][0];
    assert_eq!(first["severity"], "SEVERITY_ERROR");
    assert_eq!(first["code"], "WALK_COMMAND_FIELD_UNSUPPLIED");
    assert_eq!(first["subject"]["kind"], "ENTITY_KIND_COMMAND");
    assert_eq!(first["subject"]["id"]["slug"], "approve-review");
    // uint64 renders as a decimal string, as in proto JSON.
    assert_eq!(first["subject"]["id"]["version"], "1");
    assert_eq!(first["subjectField"], "schema.fields[score]");
    assert_eq!(first["ruleCategory"], "walk");
    assert_eq!(first["ruleDefaultSeverity"], "SEVERITY_ERROR");
    assert_eq!(first["case"], "walk-reviews/approve-review-42");
    assert_eq!(first["step"], 2);
}

#[test]
fn supplying_the_gap_in_the_answer_clears_the_command_finding() {
    let report = walk_text(&inline(
        "  steps:\n  - command: submit-review\n    answer: {review_id: r-1, rating: 4}\n  - command: approve-review\n    answer: {score: 4}\n",
    ));
    let codes = codes(&report);
    assert!(
        !codes.iter().any(|c| c.2 == "WALK_COMMAND_FIELD_UNSUPPLIED"),
        "{codes:?}"
    );
    // The answer fed the command, so the approved event carries the score
    // into the projection.
    assert!(
        !codes.iter().any(|c| c.2 == "WALK_PROJECTION_FIELD_MISSING"),
        "{codes:?}"
    );
}

#[test]
fn a_step_legal_alone_is_illegal_after_earlier_steps() {
    let report = walk_text(&inline(
        "  steps:\n  - command: submit-review\n    answer: {review_id: r-1, rating: 4}\n  - command: submit-review\n    answer: {review_id: r-1, rating: 5}\n",
    ));
    assert_eq!(
        codes(&report),
        vec![
            case("inline", Some(2), "WALK_SCENARIO_MISSING"),
            case("inline", Some(2), "WALK_LIFECYCLE_VIOLATION"),
        ]
    );
}

#[test]
fn a_stream_opened_mid_lifecycle_is_flagged() {
    let report = walk_text(&inline(
        "  steps:\n  - command: approve-review\n    answer: {review_id: r-1, score: 3}\n",
    ));
    assert_eq!(
        codes(&report),
        vec![
            case("inline", Some(1), "WALK_SCENARIO_MISSING"),
            case("inline", Some(1), "WALK_LIFECYCLE_START"),
        ]
    );
}

#[test]
fn the_models_own_reject_scenario_refuses_a_repeat() {
    let report = walk_text(&inline(
        "  given:\n    events:\n    - event: review.submitted\n      payload: {review_id: r-1, rating: 4}\n  steps:\n  - command: approve-review\n    answer: {review_id: r-1, score: 4}\n    expect: {emit: [review.approved]}\n  - command: approve-review\n    answer: {review_id: r-1, score: 4}\n    expect: {reject: ALREADY_DECIDED}\n  - command: approve-review\n    answer: {review_id: r-1, score: 4}\n    expect: {emit: [review.approved]}\n",
    ));
    assert_eq!(
        codes(&report),
        vec![case("inline", Some(3), "WALK_UNEXPECTED_REJECT")]
    );
}

#[test]
fn observed_rows_that_disagree_are_flagged() {
    let report = walk_text(&inline(
        "  given:\n    events:\n    - event: review.submitted\n      payload: {review_id: r-1, rating: 4}\n    readModels:\n    - readModel: review-queue\n      rows:\n      - {review_id: r-1, status: submitted, score: 2}\n      - {review_id: r-2, status: submitted, score: 5}\n  steps:\n  - command: approve-review\n    via: auto-approve\n    expect: {emit: [review.approved]}\n  - command: approve-review\n    via: auto-approve\n    answer: {review_id: r-2, score: 3}\n",
    ));
    let found = codes(&report);
    assert!(
        found.contains(&case("inline", Some(1), "WALK_COMMAND_FIELD_AMBIGUOUS")),
        "{found:?}"
    );
    assert!(
        found.contains(&case("inline", Some(2), "WALK_COMMAND_FIELD_CONFLICT")),
        "{found:?}"
    );
}

#[test]
fn unknown_refs_and_malformed_steps_are_findings_not_crashes() {
    let report = walk_text(&inline(
        "  steps:\n  - command: no-such-command\n  - command: submit-review\n    answer: {review_id: r-1, rating: 1}\n    expect: {emit: [review.submitted], reject: X}\n  then:\n    readModels:\n    - readModel: no-such-read-model\n",
    ));
    assert_eq!(
        codes(&report),
        vec![
            case("inline", Some(1), "WALK_REF_UNRESOLVED"),
            case("inline", Some(2), "WALK_CASE_INVALID"),
            case("inline", None, "WALK_REF_UNRESOLVED"),
        ]
    );
}

#[test]
fn coverage_reports_what_no_case_walked() {
    let report = walk_text(&inline(
        "  steps:\n  - command: submit-review\n    answer: {review_id: r-1, rating: 4}\n",
    ));
    let unwalked: Vec<&str> = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Info)
        .map(|f| f.code)
        .collect();
    assert!(
        unwalked.contains(&"WALK_STORYBOARD_UNWALKED"),
        "{unwalked:?}"
    );
    assert!(unwalked.contains(&"WALK_SCENARIO_UNWALKED"), "{unwalked:?}");
    assert_eq!(report.coverage.storyboards_walked, 1);
    assert!(!report.has_errors());
}

#[test]
fn case_documents_must_be_specifications() {
    let err = walk::parse_cases(
        "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Command\nmetadata: {namespace: a, name: b}\nspec: {}\n",
        "inline",
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("Specification"), "{err:#}");
    let err = walk::parse_cases(&inline("  stepz: []\n"), "inline").unwrap_err();
    assert!(format!("{err:#}").contains("stepz"), "{err:#}");
}

#[test]
fn every_rule_is_documented_in_the_reference() {
    let reference = std::fs::read_to_string(repo("docs/reference/walk-scenarios.md")).unwrap();
    for rule in walk::RULES {
        assert!(
            reference.contains(&format!("`{}`", rule.code)),
            "docs/reference/walk-scenarios.md does not document {}",
            rule.code
        );
    }
}

/// The shipped example walks end to end with no error or warning findings.
#[test]
fn acme_reviews_example_walks() {
    let model = Model::load_paths(&[
        repo("examples/manifests/acme-reviews"),
        repo("examples/manifests/acme-fulfillment"),
    ])
    .unwrap();
    let cases = walk::read_case_paths(&[repo("examples/specifications")]).unwrap();
    let report = walk::walk(&model, &cases);
    let found: Vec<(&'static str, String)> = report
        .findings
        .iter()
        .filter(|f| f.severity != Severity::Info)
        .map(|f| (f.code, f.subject_field.clone()))
        .collect();
    assert!(found.is_empty(), "{report:#?}");
    // The shipped case actually ran: one case, its steps walked, the
    // storyboards and scenarios it covers reached.
    assert_eq!(report.cases.len(), 1, "{report:#?}");
    assert_eq!(report.cases[0].case, "acme-reviews/order-42-review");
    assert!(report.cases[0].steps > 0 && report.cases[0].passed);
    assert!(report.coverage.storyboards_walked > 0);
    assert!(report.coverage.scenarios_walked > 0);
}

#[test]
fn slice_scenarios_only_count_the_projected_rows_own_events() {
    // r-1 is submitted and a different review, r-2, is approved: no single
    // review went submitted -> approved, so the `approved` scenario of
    // review-queue-decided is not walked.
    let report = walk_text(&inline(
        "  steps:\n  - command: submit-review\n    answer: {review_id: r-1, rating: 4}\n  - command: approve-review\n    answer: {review_id: r-2, score: 3}\n",
    ));
    let unwalked: Vec<&str> = report
        .findings
        .iter()
        .filter(|f| f.code == "WALK_SCENARIO_UNWALKED")
        .map(|f| f.message.as_str())
        .collect();
    assert!(
        unwalked
            .iter()
            .any(|m| m.contains("\"approved\"") && m.contains("review-queue-decided")),
        "{unwalked:?}"
    );
}

#[test]
fn malformed_refs_are_refused_at_parse_time() {
    for bad in [
        "approve-review@2x",
        "approve-review@0",
        "/approve-review",
        "ns/",
    ] {
        let err = walk::parse_cases(
            &inline(&format!("  steps:\n  - command: {bad:?}\n")),
            "inline",
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("steps[0].command"),
            "{bad}: {err:#}"
        );
    }
    assert!(walk::parse_cases(
        &inline("  steps:\n  - command: walk-reviews/approve-review@1\n"),
        "inline"
    )
    .is_ok());
}

#[test]
fn via_must_issue_the_command() {
    let report = walk_text(&inline(
        "  steps:\n  - command: submit-review\n    via: auto-approve\n    answer: {review_id: r-1, rating: 4}\n",
    ));
    let found = codes(&report);
    assert!(
        found.contains(&case("inline", Some(1), "WALK_VIA_NOT_ISSUER")),
        "{found:?}"
    );
}

#[test]
fn a_null_answer_supplies_nothing() {
    let report = walk_text(&inline(
        "  steps:\n  - command: submit-review\n    answer: {review_id: r-1, rating: null}\n",
    ));
    let unsupplied: Vec<&str> = report
        .findings
        .iter()
        .filter(|f| f.code == "WALK_COMMAND_FIELD_UNSUPPLIED")
        .map(|f| f.subject_field.as_str())
        .collect();
    assert_eq!(unsupplied, vec!["schema.fields[rating]"]);
}

#[test]
fn an_expected_emit_that_contradicts_the_model_is_an_error() {
    let report = walk_text(&inline(
        "  given:\n    events:\n    - event: review.submitted\n      payload: {review_id: r-1, rating: 4}\n  steps:\n  - command: approve-review\n    answer: {review_id: r-1, score: 4}\n    expect: {emit: [review.rejected]}\n",
    ));
    let found = codes(&report);
    assert!(
        found.contains(&case("inline", Some(1), "WALK_EMIT_MISMATCH")),
        "{found:?}"
    );
    // The walk follows the model: the review is approved, not rejected.
    assert!(
        !found.contains(&case("inline", Some(1), "WALK_LIFECYCLE_VIOLATION")),
        "{found:?}"
    );
}

#[test]
fn a_keyed_answer_ignores_rows_without_that_key() {
    // The second row has no review_id; it must not supply r-9's score.
    let report = walk_text(&inline(
        "  given:\n    events:\n    - event: review.submitted\n      payload: {review_id: r-9, rating: 4}\n    readModels:\n    - readModel: review-queue\n      rows:\n      - {score: 99}\n  steps:\n  - command: approve-review\n    via: auto-approve\n    answer: {review_id: r-9}\n",
    ));
    let unsupplied: Vec<&str> = report
        .findings
        .iter()
        .filter(|f| f.code == "WALK_COMMAND_FIELD_UNSUPPLIED")
        .map(|f| f.subject_field.as_str())
        .collect();
    assert_eq!(unsupplied, vec!["schema.fields[score]"], "{report:#?}");
}
