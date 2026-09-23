//! Gold-blind packets for independent corpus annotation.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::fixture::{Fixture, validate};

/// Builds a review packet that retains the source history but removes Merl's answers.
///
/// The packet carries the exact fixture digest so a later submission cannot be
/// attributed to a changed history. The blank submission belongs to the reviewer;
/// it does not imply agreement with the labels removed here.
///
/// # Errors
/// Rejects malformed or invalid fixtures and JSON shapes without a gold-state field.
pub fn independent_review_packet(fixture_bytes: &[u8]) -> Result<Value, String> {
    let fixture: Fixture = serde_json::from_slice(fixture_bytes)
        .map_err(|error| format!("could not parse fixture: {error}"))?;
    validate(&fixture).map_err(|error| format!("invalid fixture: {error}"))?;
    let mut input: Value = serde_json::from_slice(fixture_bytes)
        .map_err(|error| format!("could not parse fixture JSON: {error}"))?;
    let object = input
        .as_object_mut()
        .ok_or_else(|| "fixture must be a JSON object".to_owned())?;
    if object.remove("gold_states").is_none() {
        return Err("fixture has no gold states to blind".to_owned());
    }
    let digest = Sha256::digest(fixture_bytes);
    let digest = digest
        .iter()
        .fold(String::from("sha256:"), |mut text, byte| {
            use std::fmt::Write as _;
            write!(&mut text, "{byte:02x}").expect("writing a digest is infallible");
            text
        });
    Ok(json!({
        "schema": "merl.corpus-independent-review/v1",
        "fixture_id": fixture.id,
        "fixture_sha256": digest,
        "input": input,
        "submission": {
            "reviewer": "",
            "gold_states": [],
            "notes": []
        }
    }))
}
