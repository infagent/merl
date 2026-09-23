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
    let digest = sha256(fixture_bytes);
    let input_bytes = serde_json::to_vec(&input)
        .map_err(|error| format!("could not encode blinded input: {error}"))?;
    let input_digest = sha256(&input_bytes);
    Ok(json!({
        "schema": "merl.corpus-independent-review/v1",
        "fixture_id": fixture.id,
        "fixture_sha256": digest,
        "input_sha256": input_digest,
        "input": input,
        "submission": {
            "reviewer": "",
            "gold_states": [],
            "notes": []
        }
    }))
}

/// Verifies that a returned review packet contains the generated blinded input unchanged.
///
/// Reviewer answers and notes may change. The fixture identity, fixture digest, blinded input,
/// and blinded-input digest may not.
///
/// # Errors
/// Rejects an invalid fixture, malformed packet, or any change to the generated review input.
pub fn verify_independent_review(fixture_bytes: &[u8], packet: &Value) -> Result<(), String> {
    let expected = independent_review_packet(fixture_bytes)?;
    for field in [
        "schema",
        "fixture_id",
        "fixture_sha256",
        "input_sha256",
        "input",
    ] {
        if packet.get(field) != expected.get(field) {
            return Err(format!("review packet changed protected field {field}"));
        }
    }
    if packet
        .get("submission")
        .and_then(Value::as_object)
        .is_none()
    {
        return Err("review packet has no submission object".to_owned());
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::from("sha256:"), |mut text, byte| {
            use std::fmt::Write as _;
            write!(&mut text, "{byte:02x}").expect("writing a digest is infallible");
            text
        })
}
