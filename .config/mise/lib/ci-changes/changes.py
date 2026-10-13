#!/usr/bin/env python3
"""Decide which CI areas a pull request affects, failing towards running everything.

`changes.py github` reads the GitHub Actions environment and writes one
`<area>=true|false` output per area. `changes.py diff BASE [HEAD]` previews the
decision for a local range.
"""

import json
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

RULES_FILE = Path(__file__).with_name("rules.toml")
FULL_RUN_LABEL = "ci:full"
FILTERED_EVENTS = ("pull_request",)
AREA_NAME = re.compile(r"[a-z][a-z0-9_]*")


@dataclass(frozen=True)
class Rule:
    reason: str
    paths: tuple[str, ...]
    areas: frozenset[str]
    all: bool

    def matches(self, path: str) -> bool:
        return any(
            path.startswith(p) if p.endswith("/") else path == p for p in self.paths
        )


@dataclass(frozen=True)
class Rules:
    areas: tuple[str, ...]
    rules: tuple[Rule, ...]

    @classmethod
    def load(cls, file: Path = RULES_FILE) -> "Rules":
        data = tomllib.loads(file.read_text())
        areas = tuple(data["areas"])
        invalid = [a for a in areas if not AREA_NAME.fullmatch(a)]
        if invalid or len(set(areas)) != len(areas):
            raise ValueError(f"areas must be unique identifiers: {areas}")
        rules = []
        for raw in data["rules"]:
            rule = Rule(
                reason=raw["reason"],
                paths=tuple(raw["paths"]),
                areas=frozenset(raw.get("areas", [])),
                all=raw.get("all", False),
            )
            unknown = rule.areas - set(areas)
            if unknown:
                raise ValueError(
                    f"rule {rule.reason!r} names unknown areas {sorted(unknown)}"
                )
            rules.append(rule)
        return cls(areas=areas, rules=tuple(rules))

    def match(self, path: str) -> Rule | None:
        return next((rule for rule in self.rules if rule.matches(path)), None)


@dataclass(frozen=True)
class Decision:
    areas: dict[str, bool]
    reason: str
    files: tuple[tuple[str, str], ...] = ()

    @classmethod
    def everything(cls, rules: Rules, reason: str, files=()) -> "Decision":
        return cls({area: True for area in rules.areas}, reason, tuple(files))

    def outputs(self) -> str:
        return "".join(
            f"{area}={'true' if run else 'false'}\n" for area, run in self.areas.items()
        )

    def summary(self) -> str:
        lines = ["## Affected CI areas", "", f"Decision: {self.reason}", ""]
        lines += [
            f"- `{area}`: {'run' if run else 'skip'}"
            for area, run in self.areas.items()
        ]
        if self.files:
            lines += ["", "| Path | Rule |", "|---|---|"]
            lines += [f"| `{path}` | {why} |" for path, why in self.files]
        return "\n".join(lines) + "\n"


def decide(rules: Rules, files: list[str]) -> Decision:
    if not files:
        return Decision.everything(rules, "no changed files detected")
    selected: set[str] = set()
    classified = []
    for path in files:
        if not path.isprintable():
            return Decision.everything(rules, f"{path!r} has non-printable characters")
        rule = rules.match(path)
        if rule is None:
            classified.append((path, "no rule matches"))
            return Decision.everything(rules, f"{path} matches no rule", classified)
        classified.append((path, rule.reason))
        if rule.all:
            return Decision.everything(rules, f"{path} {rule.reason}", classified)
        selected |= rule.areas
    areas = {area: area in selected for area in rules.areas}
    return Decision(areas, "filtered by changed paths", tuple(classified))


def changed_files(base: str, head: str) -> list[str]:
    out = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", "-z", base, head],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return [path for path in out.split("\0") if path]


def merge_parents() -> tuple[str, str]:
    out = subprocess.run(
        ["git", "rev-list", "--parents", "-n", "1", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.split()
    if len(out) != 3:
        raise RuntimeError(f"HEAD is not a two-parent merge commit: {out}")
    return out[1], out[0]


def github_decision(rules: Rules, env: dict[str, str]) -> Decision:
    event = env.get("GITHUB_EVENT_NAME", "")
    if event not in FILTERED_EVENTS:
        return Decision.everything(rules, f"{event or 'unknown'} event runs everything")
    payload = json.loads(Path(env["GITHUB_EVENT_PATH"]).read_text())
    labels = {label["name"] for label in payload["pull_request"].get("labels", [])}
    if FULL_RUN_LABEL in labels:
        return Decision.everything(rules, f"{FULL_RUN_LABEL} label is set")
    base, head = merge_parents()
    return decide(rules, changed_files(base, head))


def run_github(env: dict[str, str]) -> int:
    rules = Rules.load()
    try:
        decision = github_decision(rules, env)
    except Exception as err:  # noqa: BLE001 - any failure must fall back to a full run
        decision = Decision.everything(
            rules, f"classifier failed, running everything: {err}"
        )
    print(decision.summary())
    with open(env["GITHUB_OUTPUT"], "a") as out:
        out.write(decision.outputs())
    if env.get("GITHUB_STEP_SUMMARY"):
        with open(env["GITHUB_STEP_SUMMARY"], "a") as summary:
            summary.write(decision.summary())
    return 0


def run_diff(base: str, head: str) -> int:
    merge_base = subprocess.run(
        ["git", "merge-base", base, head], check=True, capture_output=True, text=True
    ).stdout.strip()
    print(decide(Rules.load(), changed_files(merge_base, head)).summary())
    return 0


def main(argv: list[str]) -> int:
    match argv:
        case ["github"]:
            return run_github(dict(os.environ))
        case ["diff", base]:
            return run_diff(base, "HEAD")
        case ["diff", base, head]:
            return run_diff(base, head)
    print("usage: changes.py github | changes.py diff BASE [HEAD]", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
