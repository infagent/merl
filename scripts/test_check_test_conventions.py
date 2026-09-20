"""Regression cases for the root acceptance-scenario CI check."""

import unittest

from check_test_conventions import check_source


class TestConventions(unittest.TestCase):
    def test_acceptance_scenario_boundaries(self):
        cases = [
            (
                "valid scenario",
                "#[test]\nfn example() {\n    Story::given_a_project().when_changed().then_visible();\n}\n",
                None,
            ),
            (
                "missing phase",
                "#[test]\nfn example() {\n    Story::given_a_project().then_visible();\n}\n",
                "needs Given, When, and Then steps",
            ),
            (
                "wrong order",
                "#[test]\nfn example() {\n    Story::when_changed().given_a_project().then_visible();\n}\n",
                "first steps must read Given",
            ),
            (
                "direct assertion",
                "#[test]\nfn example() {\n    Story::given_a_project().when_changed().then_visible();\n    assert!(true);\n}\n",
                "move assertions and mechanics",
            ),
            (
                "root mechanics",
                "#[test]\nfn example() {\n    Story::given_a_project().when_changed().then_visible();\n    Store::open();\n}\n",
                "move assertions and mechanics",
            ),
            (
                "unsupported declaration",
                "#[tokio::test]\nasync fn example() {\n    Story::given_a_project().when_changed().then_visible();\n}\n",
                "unsupported or unrecognized test declaration",
            ),
        ]
        for name, source, expected in cases:
            with self.subTest(name=name):
                errors = check_source("example.rs", source)
                if expected is None:
                    self.assertEqual(errors, [])
                else:
                    self.assertTrue(any(expected in error for error in errors), errors)


if __name__ == "__main__":
    unittest.main()
