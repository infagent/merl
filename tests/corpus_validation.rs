#[path = "helpers/corpus_fixture.rs"]
mod corpus_fixture;

use corpus_fixture::CorpusFixture;

#[test]
fn gold_state_cannot_use_an_observation_from_its_future() {
    CorpusFixture::from_json(include_str!("../corpus/development/DEV-C1.json"))
        .given_valid_fixture()
        .at_cutoff(3)
        .expects_active("gain-fixed-20db")
        .at_cutoff(4)
        .expects_superseded("gain-fixed-20db")
        .expects_partial_support("gain-fixed-20db")
        .expects_disputed_by("gain-fixed-20db", 4)
        .expects_active("gain-sweep-10-20-30db")
        .expects_supersedes("gain-sweep-10-20-30db", "gain-fixed-20db", 4)
        .at_cutoff(3)
        .add_support("gain-fixed-20db", 4)
        .then_rejected_as_future_information();
}

#[test]
fn fixture_rejects_temporal_and_identity_ambiguity() {
    CorpusFixture::from_json(include_str!("../corpus/development/DEV-C1.json"))
        .given_valid_fixture()
        .rejects_observation_before_creation()
        .rejects_observation_after_capture()
        .rejects_duplicate_source_version()
        .rejects_duplicate_gold_cutoff()
        .rejects_duplicate_gold_object();
}

#[test]
fn one_note_can_report_a_fact_propose_a_claim_and_request_work() {
    CorpusFixture::from_json(include_str!("../corpus/development/DEV-C2.json"))
        .given_valid_fixture()
        .at_cutoff(1)
        .expects_active("double-read-hypothesis")
        .at_cutoff(2)
        .expects_active("checksum-still-fails")
        .expects_active("length-read-once")
        .expects_candidate("weaken-double-read")
        .expects_candidate("repair-checksum-parser")
        .expects_candidate("pr-229-merge")
        .expects_disputed_by("double-read-hypothesis", 2)
        .at_cutoff(3)
        .expects_open("repair-checksum-parser")
        .expects_candidate("weaken-double-read")
        .expects_waits_for("repair-checksum-parser", "pr-229-merge", 2);
}

#[test]
fn an_edit_can_change_support_without_immediately_replacing_a_decision() {
    CorpusFixture::from_json(include_str!("../corpus/development/DEV-C3.json"))
        .given_valid_fixture()
        .at_cutoff(1)
        .expects_active("csv-export")
        .at_cutoff(2)
        .expects_active("csv-export")
        .expects_revalidation_pending("csv-export")
        .expects_evidence_changed_by("csv-export", 2)
        .expects_source_supersession(2, 1)
        .at_cutoff(3)
        .expects_active("csv-export")
        .expects_current_support("csv-export")
        .at_cutoff(4)
        .expects_superseded("csv-export")
        .expects_partial_support("csv-export")
        .expects_active("parquet-export")
        .expects_supersedes("parquet-export", "csv-export", 4)
        .expects_source_supersession(4, 2);
}
