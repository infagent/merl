#!/usr/bin/env python3
"""Keep root acceptance scenarios readable without policing helper or unit tests."""

from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
TEST = re.compile(r"(?m)^#\[test\]\s*\nfn\s+(\w+)\(\)\s*\{")
STEP = re.compile(r"(?:\.|::)(given|when|then)_[a-z0-9_]+\s*\(")
PLUMBING = re.compile(r"\b(?:assert(?:_eq|_ne)?!|Store::|Command::|serde_json::|unwrap\(|expect\()")


def check_source(path: str, source: str) -> list[str]:
    errors = []
    matches = list(TEST.finditer(source))
    test_attributes = re.findall(r"(?m)^#\[(?:[A-Za-z0-9_]+::)?test(?:\([^]]*\))?\]", source)
    if not matches:
        errors.append(f"{path}: root test file has no acceptance scenarios")
    if len(test_attributes) != len(matches):
        errors.append(f"{path}: unsupported or unrecognized test declaration")
    for match in matches:
        ending = re.search(r"(?m)^}\s*$", source[match.end() :])
        if ending is None:
            errors.append(f"{path}:{match.group(1)}: cannot find test body end")
            continue
        body = source[match.end() : match.end() + ending.start()]
        steps = [step.group(1) for step in STEP.finditer(body)]
        if not all(part in steps for part in ("given", "when", "then")):
            errors.append(f"{path}:{match.group(1)}: needs Given, When, and Then steps")
        elif not (steps.index("given") < steps.index("when") < steps.index("then")):
            errors.append(f"{path}:{match.group(1)}: first steps must read Given → When → Then")
        if PLUMBING.search(body):
            errors.append(f"{path}:{match.group(1)}: move assertions and mechanics to tests/helpers/")
    return errors


def main() -> int:
    paths = sorted((ROOT / "tests").glob("*.rs"))
    errors = [
        error
        for path in paths
        for error in check_source(str(path), path.read_text(encoding="utf-8"))
    ]
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Checked acceptance scenarios in {len(paths)} test files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
