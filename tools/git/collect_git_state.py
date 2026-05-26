#!/usr/bin/env python3
"""
collect_git_state.py

Collect Git state from nested repositories inside a solution/workspace folder.

Default behavior:
- Scans direct child folders of --root
- Detects Git repositories by .git directory/file
- Does not call network by default
- Writes a stable Markdown file: GIT_STATE.md
- Masks credentials in remote URLs
- Includes branch, upstream, ahead/behind, dirty status, remotes, local branches,
  remote branches, latest commits, and tags pointing at HEAD.

Usage:
    python collect_git_state.py
    python collect_git_state.py --root .
    python collect_git_state.py --output GIT_STATE.md
    python collect_git_state.py --include-root
    python collect_git_state.py --fetch
    python collect_git_state.py --json --output git_state.json

Recommended:
    python collect_git_state.py --root . --include-root --fetch
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from dataclasses import dataclass, asdict
from datetime import datetime
from pathlib import Path
from typing import Any

DEFAULT_EXCLUDE_DIRS = {
    ".git",
    ".idea",
    ".vscode",
    ".venv",
    "venv",
    "env",
    "__pycache__",
    "node_modules",
    "dist",
    "build",
    ".gradle",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
}


@dataclass
class GitCommandResult:
    ok: bool
    stdout: str
    stderr: str
    returncode: int


@dataclass
class RepoState:
    name: str
    path: str
    is_git_repo: bool
    current_branch: str | None
    head_short: str | None
    head_subject: str | None
    head_author: str | None
    head_date: str | None
    upstream: str | None
    ahead: int | None
    behind: int | None
    is_dirty: bool
    staged_count: int
    unstaged_count: int
    untracked_count: int
    status_summary: list[str]
    remotes: dict[str, str]
    local_branches: list[str]
    remote_branches: list[str]
    tags_at_head: list[str]
    recent_commits: list[str]
    warnings: list[str]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Collect Git state from nested repositories"
    )
    parser.add_argument("--root", default=".", help="Solution/workspace root directory")
    parser.add_argument("--output", default="GIT_STATE.md", help="Output file path")
    parser.add_argument(
        "--include-root",
        action="store_true",
        help="Also include the root repo itself if it is a Git repo",
    )
    parser.add_argument(
        "--fetch",
        action="store_true",
        help="Run git fetch --all --prune in every repository before collecting state",
    )
    parser.add_argument(
        "--json", action="store_true", help="Write JSON instead of Markdown"
    )
    parser.add_argument(
        "--max-commits", type=int, default=5, help="Recent commits per repo"
    )
    parser.add_argument(
        "--max-status-lines",
        type=int,
        default=40,
        help="Max changed file lines per repo",
    )
    parser.add_argument(
        "--recursive",
        action="store_true",
        help="Scan repositories recursively, not only direct child folders",
    )
    return parser.parse_args()


def run_git(repo: Path, args: list[str], timeout: int = 20) -> GitCommandResult:
    try:
        completed = subprocess.run(
            ["git", *args],
            cwd=repo,
            text=True,
            encoding="utf-8",
            errors="replace",
            capture_output=True,
            timeout=timeout,
        )
        return GitCommandResult(
            ok=completed.returncode == 0,
            stdout=completed.stdout.strip(),
            stderr=completed.stderr.strip(),
            returncode=completed.returncode,
        )
    except FileNotFoundError:
        return GitCommandResult(False, "", "git executable not found", 127)
    except subprocess.TimeoutExpired:
        return GitCommandResult(
            False, "", f"git command timed out: {' '.join(args)}", 124
        )


def has_own_git_marker(path: Path) -> bool:
    """
    True only when this directory is a Git repository root:
    - normal repo: .git directory
    - worktree/submodule: .git file
    """
    return (path / ".git").exists()


def is_git_worktree(path: Path) -> bool:
    """
    True when path is inside any Git worktree.
    Use this only for --include-root validation, not for child repo discovery.
    """
    result = run_git(path, ["rev-parse", "--is-inside-work-tree"])
    return result.ok and result.stdout.strip() == "true"


def sanitize_remote_url(url: str) -> str:
    # Hide credentials in https://user:token@host/repo.git
    url = re.sub(r"(https?://)([^/@:]+):([^/@]+)@", r"\1***:***@", url)

    # Hide token-only URLs like https://ghp_xxx@github.com/user/repo.git
    url = re.sub(r"(https?://)(gh[pousr]_[^/@]+)@", r"\1***@", url)

    # Hide common query token patterns, because humans keep inventing ways to leak secrets.
    url = re.sub(
        r"([?&](?:token|access_token|auth|key)=)[^&]+",
        r"\1***",
        url,
        flags=re.IGNORECASE,
    )

    return url


def parse_ahead_behind(
    status_branch_line: str,
) -> tuple[str | None, str | None, int | None, int | None]:
    """
    Parses:
      ## main...origin/main [ahead 2, behind 1]
      ## dev
      ## HEAD (no branch)
    """
    line = status_branch_line.removeprefix("## ").strip()

    branch = None
    upstream = None
    ahead = 0
    behind = 0

    if line.startswith("HEAD"):
        branch = "DETACHED"
    elif "..." in line:
        left, right = line.split("...", 1)
        branch = left.strip()
        upstream_part = right.strip()

        if " [" in upstream_part:
            upstream, flags = upstream_part.split(" [", 1)
            flags = flags.rstrip("]")
            ahead_match = re.search(r"ahead (\d+)", flags)
            behind_match = re.search(r"behind (\d+)", flags)
            if ahead_match:
                ahead = int(ahead_match.group(1))
            if behind_match:
                behind = int(behind_match.group(1))
        else:
            upstream = upstream_part
    else:
        branch = line.strip()

    return branch or None, upstream or None, ahead, behind


def classify_status_lines(lines: list[str]) -> tuple[int, int, int]:
    staged = 0
    unstaged = 0
    untracked = 0

    for line in lines:
        if not line:
            continue

        if line.startswith("??"):
            untracked += 1
            continue

        # porcelain v1: XY path
        x = line[0] if len(line) >= 1 else " "
        y = line[1] if len(line) >= 2 else " "

        if x != " ":
            staged += 1
        if y != " ":
            unstaged += 1

    return staged, unstaged, untracked


def collect_remotes(repo: Path) -> dict[str, str]:
    result = run_git(repo, ["remote", "-v"])
    remotes: dict[str, str] = {}

    if not result.ok or not result.stdout:
        return remotes

    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 2:
            name = parts[0]
            url = sanitize_remote_url(parts[1])
            remotes[name] = url

    return remotes


def collect_branches(repo: Path, remote: bool = False) -> list[str]:
    args = ["branch", "-r"] if remote else ["branch", "--format", "%(refname:short)"]
    result = run_git(repo, args)

    if not result.ok or not result.stdout:
        return []

    branches: list[str] = []
    for raw in result.stdout.splitlines():
        name = raw.strip().lstrip("*").strip()
        if not name or "->" in name:
            continue
        branches.append(name)

    return sorted(set(branches))


def collect_recent_commits(repo: Path, max_commits: int) -> list[str]:
    fmt = "%h | %ci | %an | %s"
    result = run_git(repo, ["log", f"-{max_commits}", f"--format={fmt}"])

    if not result.ok or not result.stdout:
        return []

    return result.stdout.splitlines()


def collect_tags_at_head(repo: Path) -> list[str]:
    result = run_git(repo, ["tag", "--points-at", "HEAD"])

    if not result.ok or not result.stdout:
        return []

    return sorted(line.strip() for line in result.stdout.splitlines() if line.strip())


def collect_head(repo: Path) -> tuple[str | None, str | None, str | None, str | None]:
    fmt = "%h%x1f%s%x1f%an%x1f%ci"
    result = run_git(repo, ["log", "-1", f"--format={fmt}"])

    if not result.ok or not result.stdout:
        return None, None, None, None

    parts = result.stdout.split("\x1f")
    if len(parts) != 4:
        return None, None, None, None

    return parts[0], parts[1], parts[2], parts[3]


def collect_repo_state(
    repo: Path, root: Path, fetch: bool, max_commits: int, max_status_lines: int
) -> RepoState:
    warnings: list[str] = []

    if fetch:
        fetch_result = run_git(repo, ["fetch", "--all", "--prune"], timeout=60)
        if not fetch_result.ok:
            warnings.append(
                f"fetch failed: {fetch_result.stderr or fetch_result.stdout}"
            )

    status_result = run_git(repo, ["status", "--short", "--branch"])
    status_lines = status_result.stdout.splitlines() if status_result.stdout else []

    current_branch = None
    upstream = None
    ahead = None
    behind = None

    if status_lines and status_lines[0].startswith("## "):
        current_branch, upstream, ahead, behind = parse_ahead_behind(status_lines[0])
        file_status_lines = status_lines[1:]
    else:
        file_status_lines = status_lines

    staged_count, unstaged_count, untracked_count = classify_status_lines(
        file_status_lines
    )
    is_dirty = bool(file_status_lines)

    if len(file_status_lines) > max_status_lines:
        shown_status_lines = file_status_lines[:max_status_lines]
        shown_status_lines.append(
            f"... {len(file_status_lines) - max_status_lines} more"
        )
    else:
        shown_status_lines = file_status_lines

    head_short, head_subject, head_author, head_date = collect_head(repo)

    return RepoState(
        name=repo.name,
        path=repo.relative_to(root).as_posix() if repo != root else ".",
        is_git_repo=True,
        current_branch=current_branch,
        head_short=head_short,
        head_subject=head_subject,
        head_author=head_author,
        head_date=head_date,
        upstream=upstream,
        ahead=ahead,
        behind=behind,
        is_dirty=is_dirty,
        staged_count=staged_count,
        unstaged_count=unstaged_count,
        untracked_count=untracked_count,
        status_summary=shown_status_lines,
        remotes=collect_remotes(repo),
        local_branches=collect_branches(repo, remote=False),
        remote_branches=collect_branches(repo, remote=True),
        tags_at_head=collect_tags_at_head(repo),
        recent_commits=collect_recent_commits(repo, max_commits=max_commits),
        warnings=warnings,
    )


def find_repos(root: Path, include_root: bool, recursive: bool) -> list[Path]:
    repos: list[Path] = []

    if include_root and is_git_worktree(root):
        repos.append(root)

    if recursive:
        for path in sorted(root.rglob("*")):
            if not path.is_dir():
                continue

            rel_parts = path.relative_to(root).parts
            if any(part in DEFAULT_EXCLUDE_DIRS for part in rel_parts):
                continue

            if has_own_git_marker(path):
                repos.append(path)
    else:
        for path in sorted(root.iterdir(), key=lambda p: p.name.lower()):
            if not path.is_dir():
                continue

            if path.name in DEFAULT_EXCLUDE_DIRS:
                continue

            if has_own_git_marker(path):
                repos.append(path)

    unique: list[Path] = []
    seen: set[Path] = set()

    for repo in repos:
        resolved = repo.resolve()
        if resolved in seen:
            continue
        seen.add(resolved)
        unique.append(repo)

    return unique


def render_markdown(root: Path, states: list[RepoState], fetch: bool) -> str:
    now = datetime.now().isoformat(timespec="minutes")

    lines: list[str] = []
    lines.append("# Git State")
    lines.append("")
    lines.append(f"Generated at: `{now}`")
    lines.append(f"Root: `{root.resolve()}`")
    lines.append(f"Fetch before scan: `{'yes' if fetch else 'no'}`")
    lines.append("")
    lines.append("## Summary")
    lines.append("")
    lines.append(
        "| Repo | Branch | Upstream | Ahead | Behind | Dirty | HEAD | Last commit |"
    )
    lines.append("|---|---|---|---:|---:|---|---|---|")

    for state in states:
        dirty = "YES" if state.is_dirty else "no"
        branch = state.current_branch or "-"
        upstream = state.upstream or "-"
        ahead = state.ahead if state.ahead is not None else "-"
        behind = state.behind if state.behind is not None else "-"
        head = state.head_short or "-"
        subject = (state.head_subject or "-").replace("|", "\\|")
        lines.append(
            f"| `{state.name}` | `{branch}` | `{upstream}` | {ahead} | {behind} | {dirty} | `{head}` | {subject} |"
        )

    lines.append("")
    lines.append("## Details")
    lines.append("")

    for state in states:
        lines.append(f"### {state.name}")
        lines.append("")
        lines.append(f"- Path: `{state.path}`")
        lines.append(f"- Current branch: `{state.current_branch or '-'}`")
        lines.append(f"- Upstream: `{state.upstream or '-'}`")
        lines.append(
            f"- Ahead/behind: `{state.ahead if state.ahead is not None else '-'} / {state.behind if state.behind is not None else '-'}`"
        )
        lines.append(f"- Dirty: `{'yes' if state.is_dirty else 'no'}`")
        lines.append(
            f"- Staged / unstaged / untracked: `{state.staged_count} / {state.unstaged_count} / {state.untracked_count}`"
        )
        lines.append(f"- HEAD: `{state.head_short or '-'}`")
        lines.append(f"- HEAD subject: {state.head_subject or '-'}")
        lines.append(
            f"- HEAD author/date: `{state.head_author or '-'} / {state.head_date or '-'}`"
        )

        if state.tags_at_head:
            lines.append(f"- Tags at HEAD: `{', '.join(state.tags_at_head)}`")
        else:
            lines.append("- Tags at HEAD: `-`")

        lines.append("")
        lines.append("#### Remotes")
        lines.append("")
        if state.remotes:
            for name, url in state.remotes.items():
                lines.append(f"- `{name}`: `{url}`")
        else:
            lines.append("- `(none)`")

        lines.append("")
        lines.append("#### Local branches")
        lines.append("")
        if state.local_branches:
            for branch in state.local_branches:
                marker = " ← current" if branch == state.current_branch else ""
                lines.append(f"- `{branch}`{marker}")
        else:
            lines.append("- `(none)`")

        lines.append("")
        lines.append("#### Remote branches")
        lines.append("")
        if state.remote_branches:
            for branch in state.remote_branches:
                lines.append(f"- `{branch}`")
        else:
            lines.append("- `(none)`")

        lines.append("")
        lines.append("#### Working tree status")
        lines.append("")
        if state.status_summary:
            lines.append("```text")
            lines.extend(state.status_summary)
            lines.append("```")
        else:
            lines.append("```text")
            lines.append("clean")
            lines.append("```")

        lines.append("")
        lines.append("#### Recent commits")
        lines.append("")
        if state.recent_commits:
            lines.append("```text")
            lines.extend(state.recent_commits)
            lines.append("```")
        else:
            lines.append("```text")
            lines.append("(no commits)")
            lines.append("```")

        if state.warnings:
            lines.append("")
            lines.append("#### Warnings")
            lines.append("")
            for warning in state.warnings:
                lines.append(f"- {warning}")

        lines.append("")

    lines.append("## Notes for agents")
    lines.append("")
    lines.append("- `Dirty = YES` means the repository has uncommitted changes.")
    lines.append("- `Ahead > 0` means local branch has commits not pushed to upstream.")
    lines.append("- `Behind > 0` means local branch is missing commits from upstream.")
    lines.append("- This file is generated. Do not edit it manually.")
    lines.append("")

    return "\n".join(lines)


def main() -> int:
    args = parse_args()
    root = Path(args.root).resolve()

    if not root.exists() or not root.is_dir():
        raise SystemExit(f"Root directory not found: {root}")

    repos = find_repos(
        root=root, include_root=args.include_root, recursive=args.recursive
    )

    states = [
        collect_repo_state(
            repo=repo,
            root=root,
            fetch=args.fetch,
            max_commits=args.max_commits,
            max_status_lines=args.max_status_lines,
        )
        for repo in repos
    ]

    output = Path(args.output)
    if not output.is_absolute():
        output = root / output

    if args.json:
        payload: dict[str, Any] = {
            "generated_at": datetime.now().isoformat(timespec="minutes"),
            "root": str(root),
            "fetch": bool(args.fetch),
            "repositories": [asdict(state) for state in states],
        }
        output.write_text(
            json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
        )
    else:
        output.write_text(
            render_markdown(root=root, states=states, fetch=args.fetch),
            encoding="utf-8",
        )

    print(f"git state written to: {output}")
    print(f"repositories found: {len(states)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
