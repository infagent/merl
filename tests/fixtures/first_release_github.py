#!/usr/bin/python3
"""Serve a controlled complete GraphQL snapshot through the gh process boundary."""

import pathlib
import sys

assert sys.argv[1:3] == ["api", "graphql"], sys.argv
for argument in ("--paginate", "--slurp", "owner=example", "name=project", "number=17"):
    assert argument in sys.argv, sys.argv
print(pathlib.Path(__file__).with_name("pages.json").read_text())
