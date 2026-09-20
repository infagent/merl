#[path = "helpers/corpus_fixture.rs"]
mod corpus_fixture;

use corpus_fixture::CorpusFixture;

#[test]
fn gold_state_cannot_use_an_observation_from_its_future() {
    CorpusFixture::from_json(include_str!("../corpus/development/DEV-C1.json"))
        .given_valid_fixture()
        .when_viewed_at_cutoff(3)
        .then_is_active("gain-fixed-20db")
        .when_viewed_at_cutoff(4)
        .then_is_superseded("gain-fixed-20db")
        .then_has_partial_support("gain-fixed-20db")
        .then_is_disputed_by("gain-fixed-20db", 4)
        .then_is_active("gain-sweep-10-20-30db")
        .then_supersedes("gain-sweep-10-20-30db", "gain-fixed-20db", 4)
        .when_viewed_at_cutoff(3)
        .when_future_support_is_added("gain-fixed-20db", 4)
        .then_rejected_as_future_information();
}

#[test]
fn fixture_rejects_temporal_and_identity_ambiguity() {
    CorpusFixture::from_json(include_str!("../corpus/development/DEV-C1.json"))
        .given_valid_fixture()
        .when_invalid_variants_are_validated()
        .then_rejects_an_observation_before_creation()
        .then_rejects_an_observation_after_capture()
        .then_rejects_a_duplicate_source_version()
        .then_rejects_a_duplicate_gold_cutoff()
        .then_rejects_a_duplicate_gold_object();
}

#[test]
fn one_note_can_report_a_fact_propose_a_claim_and_request_work() {
    CorpusFixture::from_json(include_str!("../corpus/development/DEV-C2.json"))
        .given_valid_fixture()
        .when_viewed_at_cutoff(1)
        .then_is_active("double-read-hypothesis")
        .when_viewed_at_cutoff(2)
        .then_is_active("checksum-still-fails")
        .then_is_active("length-read-once")
        .then_is_candidate("weaken-double-read")
        .then_is_active("repair-checksum-parser")
        .then_task_awaits_acceptance("repair-checksum-parser")
        .then_task_starts_after_pr_merge("repair-checksum-parser", "github:example/parser/pull/229")
        .then_is_disputed_by("double-read-hypothesis", 2)
        .when_viewed_at_cutoff(3)
        .then_task_is_accepted_but_deferred_until_pr_merge(
            "repair-checksum-parser",
            "github:example/parser/pull/229",
        )
        .then_is_candidate("weaken-double-read")
        .when_deferred_task_lacks_reason_or_condition("repair-checksum-parser")
        .then_rejects_deferred_task_without_reason_or_condition("repair-checksum-parser");
}

#[test]
fn an_edit_can_change_support_without_immediately_replacing_a_decision() {
    CorpusFixture::from_json(include_str!("../corpus/development/DEV-C3.json"))
        .given_valid_fixture()
        .when_viewed_at_cutoff(1)
        .then_is_active("csv-export")
        .when_viewed_at_cutoff(2)
        .then_is_active("csv-export")
        .then_revalidation_is_pending("csv-export")
        .then_evidence_changed_by("csv-export", 2)
        .then_source_supersedes(2, 1)
        .when_viewed_at_cutoff(3)
        .then_is_active("csv-export")
        .then_has_current_support("csv-export")
        .when_viewed_at_cutoff(4)
        .then_is_superseded("csv-export")
        .then_has_partial_support("csv-export")
        .then_is_active("parquet-export")
        .then_supersedes("parquet-export", "csv-export", 4)
        .then_source_supersedes(4, 2);
}
