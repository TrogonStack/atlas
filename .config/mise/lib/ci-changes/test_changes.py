import json
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

import changes

REPO = Path(__file__).resolve().parents[4]
RULES = changes.Rules.load()
CONSUMERS = {"rsworkspace": "rust", "tsworkspace": "studio"}
SOURCE_SUFFIXES = {".rs", ".mjs", ".js", ".ts", ".tsx", ".proto"}
MANIFESTS = {"Cargo.toml", "package.json", "tsconfig.json", "pnpm-workspace.yaml"}
SKIPPED_DIRS = {"node_modules", "target", "dist", "gen", "coverage"}
RELATIVE_LITERAL = re.compile(r"[\"']((?:\.\./)+[^\"'\s]*)[\"']")


def selected(files):
    return {area for area, run in changes.decide(RULES, files).areas.items() if run}


GIT_ENV = {
    **os.environ,
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
    "GIT_AUTHOR_NAME": "ci",
    "GIT_AUTHOR_EMAIL": "ci@example.invalid",
    "GIT_COMMITTER_NAME": "ci",
    "GIT_COMMITTER_EMAIL": "ci@example.invalid",
}


def git(cwd, *args):
    return subprocess.run(
        ["git", *args], cwd=cwd, env=GIT_ENV, check=True, capture_output=True, text=True
    ).stdout


class DecideTest(unittest.TestCase):
    def test_docs_only_runs_nothing(self):
        self.assertEqual(selected(["docs/README.md", "README.md"]), set())

    def test_unmatched_path_runs_everything(self):
        self.assertEqual(
            selected(["docs/x.md", "brand-new-dir/file"]), set(RULES.areas)
        )

    def test_no_files_runs_everything(self):
        self.assertEqual(selected([]), set(RULES.areas))

    def test_ci_control_files_run_everything(self):
        for path in [
            ".github/workflows/ci.yml",
            "mise.toml",
            ".config/mise/lib/ci-changes/rules.toml",
        ]:
            with self.subTest(path=path):
                self.assertEqual(selected([path]), set(RULES.areas))

    def test_non_printable_path_runs_everything(self):
        self.assertEqual(selected(["docs/a\nrust=false"]), set(RULES.areas))

    def test_proto_reaches_every_consumer(self):
        self.assertEqual(
            selected(["proto/trogonatlas/x.proto"]), {"proto", "rust", "studio"}
        )

    def test_generated_proto_crate_runs_proto_check(self):
        self.assertEqual(
            selected(["rsworkspace/crates/trogon-atlas-proto/src/gen/x.rs"]),
            {"proto", "rust"},
        )

    def test_markdown_inside_a_workspace_belongs_to_it(self):
        self.assertEqual(
            selected(["rsworkspace/crates/x/prompts/p.system.md"]), {"rust"}
        )

    def test_union_of_areas(self):
        self.assertEqual(
            selected(
                ["rsworkspace/Cargo.lock", "tsworkspace/package.json", "docs/a.md"]
            ),
            {"rust", "studio"},
        )


class RulesTest(unittest.TestCase):
    def test_area_names_cannot_inject_outputs(self):
        for areas in (["rust", "x=0\nrust=false"], ["rust", "rust"]):
            with self.subTest(areas=areas), tempfile.TemporaryDirectory() as tmp:
                file = Path(tmp, "rules.toml")
                file.write_text(f"areas = {json.dumps(areas)}\nrules = []\n")
                with self.assertRaises(ValueError):
                    changes.Rules.load(file)

    def test_every_rule_path_exists(self):
        for rule in RULES.rules:
            for path in rule.paths:
                with self.subTest(path=path):
                    self.assertTrue((REPO / path).exists(), f"{path} no longer exists")

    def test_workspace_reads_outside_itself_are_covered(self):
        """A workspace reading a file elsewhere must run when that file changes."""
        for workspace, area in CONSUMERS.items():
            for source in self._sources(REPO / workspace):
                for literal in RELATIVE_LITERAL.findall(
                    source.read_text(errors="ignore")
                ):
                    for target in self._resolve(source, literal):
                        rel = target.relative_to(REPO).as_posix() + (
                            "/" if target.is_dir() else ""
                        )
                        if rel.startswith(f"{workspace}/"):
                            continue
                        rule = RULES.match(rel)
                        with self.subTest(
                            source=str(source.relative_to(REPO)), target=rel
                        ):
                            self.assertTrue(
                                rule is None or rule.all or area in rule.areas,
                                f"{rel} changes would skip {area}",
                            )

    @staticmethod
    def _sources(root):
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = [d for d in dirnames if d not in SKIPPED_DIRS]
            for name in filenames:
                if Path(name).suffix in SOURCE_SUFFIXES or name in MANIFESTS:
                    yield Path(dirpath, name)

    @staticmethod
    def _resolve(source, literal):
        bases = [source.parent]
        bases += [
            p
            for p in source.parents
            if (p / "Cargo.toml").exists() or (p / "package.json").exists()
        ][:1]
        for base in bases:
            target = Path(os.path.normpath(base / literal))
            if target.exists() and target.is_relative_to(REPO):
                yield target


class GithubDecisionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        self.cwd = os.getcwd()
        self.addCleanup(os.chdir, self.cwd)

    def event(self, labels=()):
        path = self.dir / "event.json"
        path.write_text(
            json.dumps({"pull_request": {"labels": [{"name": n} for n in labels]}})
        )
        return str(path)

    def merge_repo(self, changed):
        repo = self.dir / "repo"
        repo.mkdir()
        git(repo, "init", "-q", "-b", "main")
        (repo / "base").write_text("base")
        git(repo, "add", ".")
        git(repo, "commit", "-q", "-m", "base")
        git(repo, "switch", "-q", "-c", "pr")
        for path in changed:
            (repo / path).parent.mkdir(parents=True, exist_ok=True)
            (repo / path).write_text("x")
        git(repo, "add", ".")
        git(repo, "commit", "-q", "-m", "pr")
        git(repo, "switch", "-q", "main")
        (repo / "main-only").write_text("main moved on")
        git(repo, "add", ".")
        git(repo, "commit", "-q", "-m", "main")
        git(repo, "merge", "-q", "--no-ff", "-m", "merge", "pr")
        os.chdir(repo)

    def decision(self, **env):
        return changes.github_decision(RULES, env)

    def test_push_runs_everything(self):
        self.assertTrue(all(self.decision(GITHUB_EVENT_NAME="push").areas.values()))

    def test_full_run_label_runs_everything(self):
        decision = self.decision(
            GITHUB_EVENT_NAME="pull_request",
            GITHUB_EVENT_PATH=self.event([changes.FULL_RUN_LABEL]),
        )
        self.assertTrue(all(decision.areas.values()))

    def test_pull_request_diffs_against_the_merge_base_parent(self):
        self.merge_repo(["docs/a.md"])
        decision = self.decision(
            GITHUB_EVENT_NAME="pull_request", GITHUB_EVENT_PATH=self.event()
        )
        self.assertFalse(any(decision.areas.values()), decision.reason)

    def test_pull_request_selects_affected_areas(self):
        self.merge_repo(["tsworkspace/package.json"])
        decision = self.decision(
            GITHUB_EVENT_NAME="pull_request", GITHUB_EVENT_PATH=self.event()
        )
        self.assertEqual({a for a, run in decision.areas.items() if run}, {"studio"})

    def test_non_merge_head_raises(self):
        repo = self.dir / "flat"
        repo.mkdir()
        git(repo, "init", "-q")
        git(repo, "commit", "-q", "--allow-empty", "-m", "x")
        os.chdir(repo)
        with self.assertRaises(RuntimeError):
            self.decision(
                GITHUB_EVENT_NAME="pull_request", GITHUB_EVENT_PATH=self.event()
            )

    def test_any_failure_writes_a_full_run(self):
        output = self.dir / "output"
        os.chdir(self.dir)
        code = changes.run_github(
            {
                "GITHUB_EVENT_NAME": "pull_request",
                "GITHUB_EVENT_PATH": str(self.dir / "missing.json"),
                "GITHUB_OUTPUT": str(output),
            }
        )
        self.assertEqual(code, 0)
        self.assertEqual(
            output.read_text(), "".join(f"{a}=true\n" for a in RULES.areas)
        )


if __name__ == "__main__":
    unittest.main()
