#!/usr/bin/python3
"""Interpret only the workflow's fixed phrases at the real process boundary.

Source IDs and represented values come from Merl's input, so assertions cannot
silently cite fixture-invented provenance. Unknown prose fails the scenario.
"""

import json
import pathlib
import sys

request = json.load(sys.stdin)
with pathlib.Path(__file__).with_name("compiler-calls.jsonl").open("a") as calls:
    calls.write(json.dumps(request) + "\n")
context = request["context"]
source = next(s for s in context["sources"] if s["id"] == context["trigger"])
body = source["body"]


def assertion(subject, predicate, phrase, value="none"):
    """Use byte offsets, as required by the compiler response protocol."""
    start = body.encode().index(phrase.encode())
    return {
        "source": source["id"],
        "span_start": start,
        "span_end": start + len(phrase.encode()),
        "subject": subject,
        "predicate": predicate,
        "value": value,
        "act": "request",
        "epistemic_basis": "reported",
        "polarity": "positive",
        "confidence_millis": 900,
        "attributed_to": None,
    }


if body == "Choose a gain policy.":
    assertions = []
elif body == "Keep gain fixed.":
    assertions = [assertion("D1", "decision", body)]
elif body in ("Use double precision.", "Use double precision. Confirmed by measurements."):
    assertions = [assertion("D2", "decision", "Use double precision.")]
elif body == "Retain calibration logs. Measurements explain drift.":
    origin = source["semantic_origin"]
    assert origin["command"] == "retain-logs"
    assertions = [
        assertion("D3", "decision", "Retain calibration logs.", origin["value"]),
        assertion("F1", "finding", "Measurements explain drift."),
    ]
else:
    raise AssertionError(f"Unexpected compiler source: {body!r}")

print(json.dumps({"schema": "merl.compiler-response/v1", "assertions": assertions}))
