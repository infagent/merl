from pathlib import Path

import yaml

ROOT = Path(__file__).parents[1]


def test_unit_job_installs_git_required_by_project_detection() -> None:
    configuration = yaml.safe_load((ROOT / ".gitlab-ci.yml").read_text())
    setup = configuration["unit-tests"]["before_script"]
    assert any("apt-get install" in command and "git" in command for command in setup)
