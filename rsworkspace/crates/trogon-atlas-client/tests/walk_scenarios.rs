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

fn walk_on(model: &Model, cases: &str) -> WalkReport {
    let cases = walk::parse_cases(cases, "inline").unwrap();
    walk::walk(model, &cases)
}

fn walk_text(cases: &str) -> WalkReport {
    walk_on(&model(), cases)
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
        "  given:\n    events:\n    - event: review.submitted\n      payload: {review_id: r-1, rating: 4}\n  steps:\n  - command: approve-review\n    answer: {review_id: r-1, score: 4}\n    expect: {emit: [review.rejected]}\n  then:\n    readModels:\n    - readModel: review-queue\n      rows:\n      - {review_id: r-1, score: 4}\n",
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
    // Only review.approved carries the score into the row.
    assert!(
        !found.contains(&case("inline", None, "WALK_READ_MODEL_MISMATCH")),
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

/// A ref to a walk-reviews entity at version 1, in manifest flow style.
fn r(slug: &str) -> String {
    format!("{{id: {{namespace: walk-reviews, slug: {slug}, version: '1'}}}}")
}

fn doc(kind: &str, name: &str, spec: &str) -> String {
    format!(
        "---\napiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: {kind}\nmetadata: {{name: {name}, namespace: walk-reviews}}\nspec:\n  title: {name}\n{spec}"
    )
}

fn schema(fields: &[&str]) -> String {
    let fields: Vec<String> = fields
        .iter()
        .map(|f| format!("{{name: {f}, type: {{string: {{}}}}}}"))
        .collect();
    format!(
        "  schema: {{'@type': type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema, fields: [{}]}}\n",
        fields.join(", ")
    )
}

/// An Event or Command with string fields, on `swimlane` when given.
fn entity(kind: &str, name: &str, swimlane: Option<&str>, fields: &[&str]) -> String {
    let lane = swimlane
        .map(|s| format!("  swimlane: {}\n", r(s)))
        .unwrap_or_default();
    doc(kind, name, &format!("{}{lane}", schema(fields)))
}

/// A command slice; each scenario is `(id, given events, emitted events)`,
/// and one that emits nothing rejects with `REFUSED`.
fn command_slice(
    name: &str,
    cmd: &str,
    emits: &[&str],
    scenarios: &[(&str, &[&str], &[&str])],
) -> String {
    let events = |list: &[&str]| -> String {
        let refs: Vec<String> = list
            .iter()
            .map(|e| format!("{{event: {}}}", r(e)))
            .collect();
        format!("[{}]", refs.join(", "))
    };
    let scenarios: Vec<String> = scenarios
        .iter()
        .map(|(id, given, emit)| {
            let outcome = if emit.is_empty() {
                "reject: {reasonCode: REFUSED}".to_string()
            } else {
                format!("emit: {{events: {}}}", events(emit))
            };
            format!(
                "  - {{id: {id}, title: {id}, when: {{command: {}}}, given: {}, {outcome}}}",
                r(cmd),
                events(given),
            )
        })
        .collect();
    doc(
        "CommandSlice",
        name,
        &format!(
            "  command: {{command: {}}}\n  emittedEvents: {}\n  scenarios:\n{}\n",
            r(cmd),
            events(emits),
            scenarios.join("\n")
        ),
    )
}

/// The fixture model plus `extra` manifest documents.
fn walk_with(extra: &[String], spec: &str) -> WalkReport {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extra.yaml");
    std::fs::write(&path, extra.concat()).unwrap();
    let model = Model::load_paths(&[fixture("model.yaml"), path]).unwrap();
    walk_on(&model, &inline(spec))
}

fn unwalked(report: &WalkReport) -> Vec<&str> {
    report
        .findings
        .iter()
        .filter(|f| f.code == "WALK_SCENARIO_UNWALKED")
        .map(|f| f.message.as_str())
        .collect()
}

#[test]
fn an_event_on_the_commands_lane_lands_on_the_commands_stream() {
    // reply.posted lacks thread_id, so only the command can address the
    // thread; the second reply must see the first.
    let report = walk_with(
        &[
            doc("Swimlane", "thread", "  streamId: thread-{thread_id}\n"),
            entity("Command", "post-reply", Some("thread"), &["thread_id", "body"]),
            entity("Event", "reply.posted", Some("thread"), &["body"]),
            command_slice(
                "post-reply-slice",
                "post-reply",
                &["reply.posted"],
                &[
                    ("first", &[], &["reply.posted"]),
                    ("followup", &["reply.posted"], &["reply.posted"]),
                ],
            ),
        ],
        "  steps:\n  - command: post-reply\n    answer: {thread_id: t-1, body: a}\n  - command: post-reply\n    answer: {thread_id: t-1, body: b}\n",
    );
    assert_eq!(codes(&report), vec![], "{report:#?}");
    assert!(
        !unwalked(&report).iter().any(|m| m.contains("\"followup\"")),
        "{report:#?}"
    );
}

#[test]
fn an_event_on_another_lane_reads_its_own_streams_history() {
    // currency is only on the ledger stream, never on the payment stream
    // the command addresses.
    let report = walk_with(
        &[
            doc("Swimlane", "payment", "  streamId: payment-{payment_id}\n"),
            doc("Swimlane", "ledger", "  streamId: ledger-{account_id}\n"),
            entity("Command", "charge", Some("payment"), &["payment_id", "account_id"]),
            entity("Event", "ledger.opened", Some("ledger"), &["account_id", "currency"]),
            entity("Event", "ledger.charged", Some("ledger"), &["account_id", "currency"]),
            command_slice(
                "charge-slice",
                "charge",
                &["ledger.charged"],
                &[("charge", &[], &["ledger.charged"])],
            ),
        ],
        "  given:\n    events:\n    - event: ledger.opened\n      payload: {account_id: a-1, currency: USD}\n  steps:\n  - command: charge\n    answer: {payment_id: p-1, account_id: a-1}\n",
    );
    assert_eq!(codes(&report), vec![], "{report:#?}");
}

#[test]
fn lane_less_commands_do_not_share_a_stream() {
    let report = walk_with(
        &[
            entity("Command", "ping", None, &["id"]),
            entity("Event", "pinged", None, &["id"]),
            entity("Command", "pong", None, &["id"]),
            entity("Event", "ponged", None, &["id"]),
            entity("Event", "pong.failed", None, &["id"]),
            command_slice("ping-slice", "ping", &["pinged"], &[("ping", &[], &["pinged"])]),
            command_slice(
                "pong-slice",
                "pong",
                &["ponged", "pong.failed"],
                &[("pong", &[], &["ponged"])],
            ),
        ],
        "  steps:\n  - command: ping\n    answer: {id: x}\n  - command: pong\n    answer: {id: y}\n",
    );
    assert_eq!(codes(&report), vec![], "{report:#?}");
}

#[test]
fn an_empty_given_needs_an_empty_stream_not_just_no_lifecycle_event() {
    // review.noted is outside the review lifecycle, but it is on the stream.
    let report = walk_with(
        &[
            entity("Command", "note-review", Some("review"), &["review_id"]),
            entity("Event", "review.noted", Some("review"), &["review_id"]),
            command_slice(
                "note-review-slice",
                "note-review",
                &["review.noted"],
                &[("note", &[], &["review.noted"])],
            ),
        ],
        "  steps:\n  - command: note-review\n    answer: {review_id: r-1}\n  - command: submit-review\n    answer: {review_id: r-1, rating: 4}\n",
    );
    assert_eq!(
        codes(&report),
        vec![case("inline", Some(2), "WALK_SCENARIO_MISSING")],
        "{report:#?}"
    );
}

#[test]
fn a_historical_null_does_not_source_an_event_field() {
    // The latest comment cleared detail; the older value must not revive.
    let report = walk_with(
        &[
            entity("Event", "review.commented", Some("review"), &["review_id", "detail"]),
            entity("Event", "review.flagged", Some("review"), &["review_id", "detail"]),
            entity("Command", "flag-review", Some("review"), &["review_id"]),
            command_slice(
                "flag-review-slice",
                "flag-review",
                &["review.flagged"],
                &[("flag", &["review.commented"], &["review.flagged"])],
            ),
        ],
        "  given:\n    events:\n    - event: review.commented\n      payload: {review_id: r-1, detail: old}\n    - event: review.commented\n      payload: {review_id: r-1, detail: null}\n  steps:\n  - command: flag-review\n    answer: {review_id: r-1}\n",
    );
    assert_eq!(
        codes(&report),
        vec![case("inline", Some(1), "WALK_EVENT_FIELD_UNSOURCED")],
        "{report:#?}"
    );
}

#[test]
fn an_opaque_event_schema_is_reported() {
    let report = walk_with(
        &[
            entity("Command", "archive", None, &["review_id"]),
            doc(
                "Event",
                "archived",
                "  schema: {'@type': type.googleapis.com/google.protobuf.Struct, value: {}}\n",
            ),
            command_slice(
                "archive-slice",
                "archive",
                &["archived"],
                &[("archive", &[], &["archived"])],
            ),
        ],
        "  steps:\n  - command: archive\n    answer: {review_id: r-1}\n",
    );
    let opaque: Vec<&str> = report
        .findings
        .iter()
        .filter(|f| f.code == "WALK_SCHEMA_OPAQUE")
        .map(|f| f.message.as_str())
        .collect();
    assert!(opaque.iter().any(|m| m.contains("archived")), "{report:#?}");
}

#[test]
fn an_unpinned_external_event_completes_a_pinned_slice_scenario() {
    let upstream = "{id: {namespace: upstream, slug: order.placed, version: '2'}}";
    let report = walk_with(
        &[
            doc(
                "ReadModel",
                "order-board",
                &format!(
                    "  key: [order_id]\n{}  sourceEvents: [{upstream}]\n",
                    schema(&["order_id"])
                ),
            ),
            doc(
                "ReadModelSlice",
                "order-board-placed",
                &format!(
                    "  projectionRole: PROJECTION_ROLE_UPSERT\n  readModel: {{readModel: {rm}}}\n  sourceEvents: [{{event: {upstream}}}]\n  scenarios:\n  - {{id: placed, title: placed, when: [{{event: {upstream}}}], then: {{readModel: {rm}, payload: {{'@type': type.googleapis.com/google.protobuf.Struct, value: {{order_id: o-1}}}}}}}}\n",
                    rm = r("order-board")
                ),
            ),
        ],
        "  given:\n    events:\n    - event: upstream/order.placed\n      payload: {order_id: o-1}\n",
    );
    assert!(
        !unwalked(&report)
            .iter()
            .any(|m| m.contains("order-board-placed")),
        "{report:#?}"
    );
}

#[test]
fn an_unexpected_reject_still_counts_as_walked() {
    let report = walk_text(&inline(
        "  given:\n    events:\n    - event: review.submitted\n      payload: {review_id: r-1, rating: 4}\n    - event: review.approved\n      payload: {review_id: r-1, score: 4}\n  steps:\n  - command: approve-review\n    answer: {review_id: r-1, score: 4}\n    expect: {emit: [review.approved]}\n",
    ));
    assert_eq!(
        codes(&report),
        vec![case("inline", Some(1), "WALK_UNEXPECTED_REJECT")]
    );
    assert!(
        !unwalked(&report)
            .iter()
            .any(|m| m.contains("already-approved")),
        "{report:#?}"
    );
}

#[test]
fn an_expected_event_the_slice_cannot_emit_is_not_walked() {
    // approve-review-slice never emits review.rejected, so no row appears.
    let report = walk_text(&inline(
        "  steps:\n  - command: approve-review\n    answer: {review_id: r-1, score: 1}\n    expect: {emit: [review.rejected]}\n  then:\n    readModels:\n    - readModel: review-queue\n      rows: []\n",
    ));
    assert_eq!(
        codes(&report),
        vec![case("inline", Some(1), "WALK_EVENT_NOT_EMITTED_BY_SLICE")],
        "{report:#?}"
    );
}

fn thread_model(scenarios: &[(&str, &[&str], &[&str])]) -> Vec<String> {
    vec![
        doc("Swimlane", "thread", "  streamId: thread-{thread_id}\n"),
        entity(
            "Command",
            "post-reply",
            Some("thread"),
            &["thread_id", "body"],
        ),
        entity("Event", "reply.posted", Some("thread"), &["body"]),
        command_slice(
            "post-reply-slice",
            "post-reply",
            &["reply.posted"],
            scenarios,
        ),
    ]
}

#[test]
fn numeric_stream_ids_ignore_float_form() {
    let report = walk_with(
        &thread_model(&[
            ("first", &[], &["reply.posted"]),
            ("followup", &["reply.posted"], &["reply.posted"]),
        ]),
        "  steps:\n  - command: post-reply\n    answer: {thread_id: 4, body: a}\n  - command: post-reply\n    answer: {thread_id: 4.0, body: b}\n",
    );
    assert!(
        !unwalked(&report).iter().any(|m| m.contains("\"followup\"")),
        "{report:#?}"
    );
}

#[test]
fn another_instance_of_the_same_lane_is_not_related() {
    // Both threads hold a reply with body "a"; t-2 has no reply yet.
    let report = walk_with(
        &thread_model(&[
            ("first", &[], &["reply.posted"]),
            ("duplicate", &["reply.posted"], &[]),
        ]),
        "  steps:\n  - command: post-reply\n    answer: {thread_id: t-1, body: a}\n  - command: post-reply\n    answer: {thread_id: t-2, body: a}\n    expect: {emit: [reply.posted]}\n",
    );
    assert_eq!(codes(&report), vec![], "{report:#?}");
}

#[test]
fn seed_rows_without_a_key_do_not_overwrite_each_other() {
    let report = walk_text(&inline(
        "  given:\n    readModels:\n    - readModel: review-queue\n      rows:\n      - {score: 1}\n    - readModel: review-queue\n      rows:\n      - {score: 2}\n  then:\n    readModels:\n    - readModel: review-queue\n      rows:\n      - {score: 1}\n      - {score: 2}\n",
    ));
    assert_eq!(codes(&report), vec![], "{report:#?}");
}

#[test]
fn a_destination_stream_key_can_come_from_the_answer() {
    // account_id is ledger state, not a charge field.
    let report = walk_with(
        &[
            doc("Swimlane", "payment", "  streamId: payment-{payment_id}\n"),
            doc(
                "Swimlane",
                "ledger",
                "  streamId: ledger-{account_id}\n  state: [{name: account_id, type: {string: {}}}]\n",
            ),
            entity("Command", "charge", Some("payment"), &["payment_id"]),
            entity("Event", "ledger.opened", Some("ledger"), &["account_id", "currency"]),
            entity("Event", "ledger.charged", Some("ledger"), &["account_id", "currency"]),
            command_slice(
                "charge-slice",
                "charge",
                &["ledger.charged"],
                &[("charge", &[], &["ledger.charged"])],
            ),
        ],
        "  given:\n    events:\n    - event: ledger.opened\n      payload: {account_id: a-1, currency: USD}\n  steps:\n  - command: charge\n    answer: {payment_id: p-1, account_id: a-1}\n",
    );
    assert_eq!(codes(&report), vec![], "{report:#?}");
}

#[test]
fn a_command_accepted_on_a_terminal_stream_is_flagged_wherever_it_emits() {
    let report = walk_with(
        &[
            doc("Swimlane", "audit", ""),
            entity("Command", "audit-review", Some("review"), &["review_id"]),
            entity("Event", "audit.logged", Some("audit"), &["review_id"]),
            command_slice(
                "audit-review-slice",
                "audit-review",
                &["audit.logged"],
                &[("audit", &["review.approved"], &["audit.logged"])],
            ),
        ],
        "  given:\n    events:\n    - event: review.submitted\n      payload: {review_id: r-1, rating: 4}\n    - event: review.approved\n      payload: {review_id: r-1, score: 4}\n  steps:\n  - command: audit-review\n    answer: {review_id: r-1}\n",
    );
    assert_eq!(
        codes(&report),
        vec![case("inline", Some(1), "WALK_ACCEPTED_AFTER_TERMINAL")],
        "{report:#?}"
    );
}

#[test]
fn a_scenario_must_reach_the_current_lifecycle_occurrence() {
    // The ticket was reopened: [opened, closed] describes an earlier cycle.
    let ticket = r("ticket.opened");
    let closed = r("ticket.closed");
    let report = walk_with(
        &[
            doc(
                "Swimlane",
                "ticket",
                &format!(
                    "  streamId: ticket-{{ticket_id}}\n  transitions:\n  - {{after: {ticket}, next: [{{event: {closed}}}]}}\n  - {{after: {closed}, next: [{{event: {ticket}}}]}}\n"
                ),
            ),
            entity("Event", "ticket.opened", Some("ticket"), &["ticket_id"]),
            entity("Event", "ticket.closed", Some("ticket"), &["ticket_id"]),
            entity("Command", "close-ticket", Some("ticket"), &["ticket_id"]),
            command_slice(
                "close-ticket-slice",
                "close-ticket",
                &["ticket.closed"],
                &[
                    ("close", &["ticket.opened"], &["ticket.closed"]),
                    ("already-closed", &["ticket.opened", "ticket.closed"], &[]),
                ],
            ),
        ],
        "  given:\n    events:\n    - event: ticket.opened\n      payload: {ticket_id: t-1}\n    - event: ticket.closed\n      payload: {ticket_id: t-1}\n    - event: ticket.opened\n      payload: {ticket_id: t-1}\n  steps:\n  - command: close-ticket\n    answer: {ticket_id: t-1}\n    expect: {emit: [ticket.closed]}\n",
    );
    assert_eq!(codes(&report), vec![], "{report:#?}");
}

#[test]
fn an_answer_no_eligible_row_holds_is_a_conflict() {
    let report = walk_text(&inline(
        "  given:\n    events:\n    - event: review.submitted\n      payload: {review_id: r-1, rating: 4}\n    readModels:\n    - readModel: review-queue\n      rows:\n      - {review_id: r-1, status: submitted, score: 2}\n      - {review_id: r-2, status: submitted, score: 5}\n  steps:\n  - command: approve-review\n    via: auto-approve\n    answer: {score: 3}\n",
    ));
    let conflicts: Vec<&str> = report
        .findings
        .iter()
        .filter(|f| f.code == "WALK_COMMAND_FIELD_CONFLICT")
        .map(|f| f.subject_field.as_str())
        .collect();
    assert_eq!(conflicts, vec!["schema.fields[score]"], "{report:#?}");
}
