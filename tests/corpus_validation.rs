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
