#!/usr/bin/env python3
"""
build_repo_map.py

Generate an AI-friendly repository snapshot from the project root.

Usage:
    python build_repo_map.py
    python build_repo_map.py --root .
    python build_repo_map.py --root SyncServer
    python build_repo_map.py --root ../Warehouse_web
    python build_repo_map.py --output repo_map.md
    python build_repo_map.py --format json
    python build_repo_map.py --output repo_map.json --format json
    python build_repo_map.py --include-hidden
    python build_repo_map.py --include-dotenv

What it does:
- Builds a filtered directory tree
- Detects likely entry points
- Extracts HTTP routes from FastAPI and Django-style route files
- Lists models, repos, services, schemas
- Detects basic route -> service/repo/model dependency hints
- Reads env vars from .env.example and optionally .env
- Produces Markdown report by default, JSON with --format json
- If --output is not passed, names the file automatically:
  repo_map_<repository_name>_<ddmmyyhhmm>.md

Safe defaults:
- Hides noisy folders like .git, .idea, .venv, node_modules, __pycache__
- Does not dump source code bodies into the report
- Does not include .env by default unless you pass --include-dotenv
"""

from __future__ import annotations

import argparse
import ast
import json
import os
import re
from collections import defaultdict
from datetime import datetime
from pathlib import Path
from typing import Iterable, Any

DEFAULT_EXCLUDE_DIRS = {
    ".git",
    ".idea",
    ".venv",
    "venv",
    "env",
    "__pycache__",
    "node_modules",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".coverage",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".cache",
    ".DS_Store",
    "bin",
    "obj",
}

DEFAULT_EXCLUDE_FILES = {
    ".env",
    "poetry.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "repo_map.txt",
    "repo_map.md",
    "repo_map.json",
}

READ_FIRST_CANDIDATES = [
    "README.md",
    "INDEX.md",
    "AI_CONTEXT.md",
    "AI_ENTRY_POINTS.md",
    "ARCHITECTURE.md",
    "SYSTEM_MAP.md",
    "DOMAIN_MODEL.md",
    "API_CONTRACT.md",
    "USE_CASES.md",
    "MEMORY.md",
    "main.py",
    "manage.py",
    "docker-compose.yml",
    "compose.yml",
    "Dockerfile",
]

ROUTE_DECORATORS = {"get", "post", "put", "patch", "delete", "options", "head"}
SENSITIVE_NAME_HINTS = {
    "auth": "Authentication / authorization logic",
    "permission": "Permissions / access control",
    "access": "Permissions / access control",
    "role": "Roles / access rules",
    "security": "Security-sensitive logic",
    "secret": "Secrets / credentials handling",
    "token": "Token handling",
    "password": "Password handling",
    "migration": "Database migration / schema evolution",
    "alembic": "Database migration / schema evolution",
    "transaction": "Transaction management",
    "uow": "Transaction management / unit of work",
    "delete": "Deletion logic",
    "remove": "Deletion logic",
    "sync": "Sync / replication / eventual consistency",
    "billing": "Financially sensitive logic",
    "payment": "Financially sensitive logic",
}
ENTRYPOINT_NAMES = {
    "main.py",
    "manage.py",
    "app.py",
    "server.py",
    "wsgi.py",
    "asgi.py",
    "Program.cs",
    "index.js",
    "cli.py",
}
INFRA_FILENAMES = {
    "Dockerfile",
    "docker-compose.yml",
    "compose.yml",
    "nginx.conf",
    "requirements.txt",
    "pyproject.toml",
    ".github/workflows",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build AI-friendly repo map")
    parser.add_argument(
        "--root",
        default=".",
        help="Project root directory. Can be absolute or relative to the current working directory.",
    )
    parser.add_argument(
        "--output",
        default=None,
        help="Output file path. If omitted, generated automatically inside --root.",
    )
    parser.add_argument(
        "--format",
        choices=["md", "txt", "json"],
        default="md",
        help="Output format: md (default), txt, or json",
    )
    parser.add_argument(
        "--include-hidden",
        action="store_true",
        help="Include hidden files/directories except explicitly excluded ones",
    )
    parser.add_argument(
        "--include-dotenv",
        action="store_true",
        help="Also parse .env variables (off by default for safety)",
    )
    parser.add_argument(
        "--max-tree-entries",
        type=int,
        default=3000,
        help="Safety cap for number of tree lines",
    )
    parser.add_argument(
        "--max-section-items",
        type=int,
        default=200,
        help="Safety cap for number of items per section",
    )
    return parser.parse_args()


def is_hidden(path: Path) -> bool:
    return any(part.startswith(".") for part in path.parts if part not in {".", ".."})


def should_skip(path: Path, root: Path, include_hidden: bool) -> bool:
    rel = path.relative_to(root)
    parts = set(rel.parts)

    if any(part in DEFAULT_EXCLUDE_DIRS for part in parts):
        return True

    if path.name in DEFAULT_EXCLUDE_FILES:
        return True

    if re.fullmatch(r"repo_map_.+\\.(md|txt|json)", path.name):
        return True

    if not include_hidden and is_hidden(rel):
        return True

    return False


def iter_paths(root: Path, include_hidden: bool) -> Iterable[Path]:
    for dirpath, dirnames, filenames in os.walk(root):
        current = Path(dirpath)

        filtered_dirs = []
        for d in dirnames:
            candidate = current / d
            if not should_skip(candidate, root, include_hidden):
                filtered_dirs.append(d)
        dirnames[:] = sorted(filtered_dirs)

        for filename in sorted(filenames):
            candidate = current / filename
            if should_skip(candidate, root, include_hidden):
                continue
            yield candidate


def build_tree(root: Path, include_hidden: bool, max_entries: int) -> list[str]:
    lines: list[str] = []

    def walk(directory: Path, prefix: str = "") -> None:
        nonlocal lines
        if len(lines) >= max_entries:
            return

        try:
            children = []
            for child in sorted(
                directory.iterdir(), key=lambda p: (not p.is_dir(), p.name.lower())
            ):
                if should_skip(child, root, include_hidden):
                    continue
                children.append(child)
        except PermissionError:
            return

        for idx, child in enumerate(children):
            if len(lines) >= max_entries:
                return
            connector = "└── " if idx == len(children) - 1 else "├── "
            rel = child.relative_to(root)
            suffix = "/" if child.is_dir() else ""
            lines.append(f"{prefix}{connector}{rel.as_posix()}{suffix}")
            if child.is_dir():
                extension = "    " if idx == len(children) - 1 else "│   "
                walk(child, prefix + extension)

    walk(root)
    return lines


def read_text_safe(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        try:
            return path.read_text(encoding="utf-8-sig")
        except Exception:
            return ""
    except Exception:
        return ""


def try_parse_ast(path: Path) -> ast.AST | None:
    text = read_text_safe(path)
    if not text.strip():
        return None
    try:
        return ast.parse(text)
    except SyntaxError:
        return None


def detect_project_type(root: Path) -> list[str]:
    hints: list[str] = []

    if (root / "requirements.txt").exists() or (root / "pyproject.toml").exists():
        hints.append("Python")
    if (root / "main.py").exists():
        hints.append("App entry via main.py")
    if (root / "manage.py").exists():
        hints.append("Django")
    if any(root.rglob("fastapi*.py")) or any(root.rglob("routes*.py")):
        hints.append("FastAPI-style routes (possible)")
    if (
        (root / "docker-compose.yml").exists()
        or (root / "compose.yml").exists()
        or (root / "Dockerfile").exists()
    ):
        hints.append("Docker")
    if (root / "tests").exists():
        hints.append("Tests")
    if any(root.rglob("alembic.ini")) or (root / "migrations").exists():
        hints.append("DB migrations")
    if any(root.rglob("*.csproj")):
        hints.append(".NET")
    if any(root.rglob("package.json")):
        hints.append("Node.js / frontend tooling")

    return hints


def detect_architecture_signals(root: Path) -> list[dict[str, str]]:
    signals: list[dict[str, str]] = []

    def add(name: str, confidence: str, evidence: str) -> None:
        signals.append({"signal": name, "confidence": confidence, "evidence": evidence})

    if (root / "app" / "services").exists() or any(root.rglob("services")):
        add(
            "Service layer likely present",
            "Confirmed by structure",
            "Found services directory",
        )
    if (
        (root / "app" / "repos").exists()
        or any(root.rglob("repos"))
        or any(root.rglob("repositories"))
    ):
        add(
            "Repository/data access layer likely present",
            "Confirmed by structure",
            "Found repos/repositories directory",
        )
    if (root / "app" / "models").exists() or any(root.rglob("models")):
        add(
            "Model layer likely present",
            "Confirmed by structure",
            "Found models directory",
        )
    if any(root.rglob("uow.py")) or any(root.rglob("*unit_of_work*.py")):
        add(
            "Unit of work pattern likely present",
            "Confirmed by filename",
            "Found uow/unit_of_work file",
        )
    if (root / "manage.py").exists():
        add("Django application detected", "Confirmed by code", "manage.py exists")
    if any(root.rglob("APIRouter")) or any(root.rglob("fastapi")):
        add(
            "FastAPI-style routing likely present",
            "Inferred from repository structure",
            "FastAPI/router-related files found",
        )
    if (
        (root / "Dockerfile").exists()
        or (root / "docker-compose.yml").exists()
        or (root / "compose.yml").exists()
    ):
        add(
            "Containerized development/deployment likely present",
            "Confirmed by files",
            "Docker-related files found",
        )
    if any(root.rglob(".github/workflows")):
        add(
            "CI workflow likely present",
            "Confirmed by structure",
            ".github/workflows found",
        )
    if any(root.rglob("alembic.ini")) or (root / "migrations").exists():
        add(
            "Schema migration mechanism likely present",
            "Confirmed by files",
            "Migration files found",
        )

    return signals


def detect_entry_points(root: Path) -> list[str]:
    entries: list[str] = []

    for path in iter_paths(root, include_hidden=False):
        rel = path.relative_to(root).as_posix()
        if path.name in ENTRYPOINT_NAMES:
            entries.append(rel)
        elif rel.startswith("scripts/") and path.suffix in {".py", ".ps1", ".sh"}:
            entries.append(rel)
        elif rel.startswith("tools/") and path.suffix in {".py", ".ps1", ".sh"}:
            entries.append(rel)

    seen = set()
    unique = []
    for item in entries:
        if item not in seen:
            seen.add(item)
            unique.append(item)
    return unique


def summarize_path(rel_path: str) -> str:
    p = rel_path.replace("\\", "/")
    name = Path(p).name

    if "/api/" in p and name.startswith("routes"):
        return "HTTP routes / API handlers"
    if "/services/" in p:
        return "Business logic / orchestration"
    if "/repos/" in p or "/repositories/" in p:
        return "Data access / repository layer"
    if "/models/" in p:
        return "ORM or domain model"
    if "/schemas/" in p:
        return "Pydantic / DTO schemas"
    if "/core/" in p:
        return "Core config / infrastructure"
    if p.startswith("tests/"):
        return "Tests"
    if p.startswith("docs/adr/"):
        return "Architecture decision record"
    if p.startswith("docs/"):
        return "Project documentation"
    if p.startswith("db/"):
        return "Database schema / bootstrap"
    if p.endswith(".sql"):
        return "SQL schema or migration"
    if name == "README.md":
        return "Project overview"
    if name.startswith("AI_") or name in {
        "ARCHITECTURE.md",
        "SYSTEM_MAP.md",
        "DOMAIN_MODEL.md",
        "API_CONTRACT.md",
        "USE_CASES.md",
        "INDEX.md",
        "MEMORY.md",
        "PROJECT_BRAIN.md",
    }:
        return "AI/project guidance document"
    if name in {"docker-compose.yml", "compose.yml", "Dockerfile"}:
        return "Container / deployment config"
    return "Project file"


def collect_module_map(root: Path) -> dict[str, list[str]]:
    groups: dict[str, list[str]] = defaultdict(list)

    for path in iter_paths(root, include_hidden=False):
        rel = path.relative_to(root).as_posix()
        if rel.startswith("app/api/") or "/api/" in rel:
            groups["API"].append(rel)
        elif rel.startswith("app/services/") or "/services/" in rel:
            groups["Services"].append(rel)
        elif (
            rel.startswith("app/repos/")
            or rel.startswith("app/repositories/")
            or "/repos/" in rel
            or "/repositories/" in rel
        ):
            groups["Repositories"].append(rel)
        elif rel.startswith("app/models/") or "/models/" in rel:
            groups["Models"].append(rel)
        elif rel.startswith("app/schemas/") or "/schemas/" in rel:
            groups["Schemas"].append(rel)
        elif rel.startswith("app/core/") or "/core/" in rel:
            groups["Core"].append(rel)
        elif rel.startswith("db/") or "/migrations/" in rel or "alembic" in rel:
            groups["Database"].append(rel)
        elif rel.startswith("docs/"):
            groups["Docs"].append(rel)
        elif rel.startswith("tests/") or "/tests/" in rel:
            groups["Tests"].append(rel)
        elif any(
            seg in rel
            for seg in [
                ".github/workflows",
                "Dockerfile",
                "docker-compose.yml",
                "compose.yml",
                "nginx",
            ]
        ):
            groups["Infra"].append(rel)
        elif rel.endswith(".py"):
            groups["Python files"].append(rel)

    return dict(groups)


def parse_fastapi_routes(file_path: Path, root: Path) -> list[dict[str, Any]]:
    text = read_text_safe(file_path)
    if not text.strip():
        return []

    try:
        tree = ast.parse(text)
    except SyntaxError:
        return []

    router_prefixes: dict[str, str] = {}
    include_router_refs: list[dict[str, str]] = []

    class Visitor(ast.NodeVisitor):
        def visit_Assign(self, node: ast.Assign) -> None:
            try:
                if isinstance(node.value, ast.Call):
                    func = node.value.func
                    if isinstance(func, ast.Name) and func.id == "APIRouter":
                        prefix = ""
                        for kw in node.value.keywords:
                            if kw.arg == "prefix" and isinstance(
                                kw.value, ast.Constant
                            ):
                                prefix = str(kw.value.value)
                        for target in node.targets:
                            if isinstance(target, ast.Name):
                                router_prefixes[target.id] = prefix
            except Exception:
                pass
            self.generic_visit(node)

        def visit_Expr(self, node: ast.Expr) -> None:
            try:
                value = node.value
                if isinstance(value, ast.Call) and isinstance(
                    value.func, ast.Attribute
                ):
                    if value.func.attr == "include_router" and isinstance(
                        value.func.value, ast.Name
                    ):
                        target_router = value.func.value.id
                        included = ""
                        if value.args and isinstance(value.args[0], ast.Name):
                            included = value.args[0].id
                        include_router_refs.append(
                            {"target": target_router, "included": included}
                        )
            except Exception:
                pass
            self.generic_visit(node)

    Visitor().visit(tree)

    routes: list[dict[str, Any]] = []

    for node in tree.body:
        if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue

        for decorator in node.decorator_list:
            if not isinstance(decorator, ast.Call):
                continue

            func = decorator.func
            router_name = None
            method_name = None

            if isinstance(func, ast.Attribute) and isinstance(func.value, ast.Name):
                router_name = func.value.id
                method_name = func.attr.lower()

            if router_name is None or method_name not in ROUTE_DECORATORS:
                continue

            path_value = ""
            if decorator.args and isinstance(decorator.args[0], ast.Constant):
                path_value = str(decorator.args[0].value)

            full_path = f"{router_prefixes.get(router_name, '')}{path_value}" or "/"
            routes.append(
                {
                    "framework": "FastAPI",
                    "method": method_name.upper(),
                    "path": full_path,
                    "handler": node.name,
                    "file": file_path.relative_to(root).as_posix(),
                    "router": router_name,
                }
            )

    return routes


def parse_django_routes(file_path: Path, root: Path) -> list[dict[str, Any]]:
    text = read_text_safe(file_path)
    if not text.strip():
        return []

    try:
        tree = ast.parse(text)
    except SyntaxError:
        return []

    routes: list[dict[str, Any]] = []

    def extract_constant_str(node: ast.AST) -> str:
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            return node.value
        return "<dynamic>"

    def extract_target(node: ast.AST) -> str:
        if isinstance(node, ast.Name):
            return node.id
        if isinstance(node, ast.Attribute):
            if isinstance(node.value, ast.Name):
                return f"{node.value.id}.{node.attr}"
            return node.attr
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            if node.func.attr == "as_view":
                base = node.func.value
                if isinstance(base, ast.Name):
                    return f"{base.id}.as_view"
                if isinstance(base, ast.Attribute) and isinstance(base.value, ast.Name):
                    return f"{base.value.id}.{base.attr}.as_view"
        return "<dynamic>"

    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        func = node.func
        func_name = None
        if isinstance(func, ast.Name):
            func_name = func.id
        elif isinstance(func, ast.Attribute):
            func_name = func.attr

        if func_name not in {"path", "re_path"}:
            continue

        route_path = extract_constant_str(node.args[0]) if node.args else "<dynamic>"
        target = extract_target(node.args[1]) if len(node.args) > 1 else "<unknown>"
        routes.append(
            {
                "framework": "Django",
                "method": "N/A",
                "path": route_path,
                "handler": target,
                "file": file_path.relative_to(root).as_posix(),
                "router": "urlpatterns",
            }
        )

    return routes


def collect_api_map(root: Path) -> list[dict[str, Any]]:
    routes: list[dict[str, Any]] = []
    for path in root.rglob("*.py"):
        if should_skip(path, root, include_hidden=False):
            continue
        rel = path.relative_to(root).as_posix()
        name = path.name.lower()
        text = read_text_safe(path)
        if "/api/" in rel or name.startswith("routes") or name == "urls.py":
            if "APIRouter" in text or "FastAPI" in text:
                routes.extend(parse_fastapi_routes(path, root))
            if "urlpatterns" in text or "django.urls" in text or name == "urls.py":
                routes.extend(parse_django_routes(path, root))

    routes.sort(key=lambda x: (x["path"], x["method"], x["handler"], x["file"]))
    return routes


def collect_import_hints(root: Path, max_items: int) -> list[dict[str, str]]:
    hints: list[dict[str, str]] = []

    interesting = {
        "api": "API",
        "services": "Service",
        "repos": "Repository",
        "repositories": "Repository",
        "models": "Model",
        "schemas": "Schema",
        "core": "Core",
    }

    def classify_rel(rel: str) -> str | None:
        parts = rel.replace("\\", "/").split("/")
        for part in parts:
            if part in interesting:
                return interesting[part]
        return None

    for path in root.rglob("*.py"):
        if len(hints) >= max_items:
            break
        if should_skip(path, root, include_hidden=False):
            continue

        rel = path.relative_to(root).as_posix()
        src_type = classify_rel(rel)
        if src_type is None:
            continue

        tree = try_parse_ast(path)
        if tree is None:
            continue

        for node in ast.walk(tree):
            if isinstance(node, ast.ImportFrom):
                module = node.module or ""
                if any(token in module.split(".") for token in interesting):
                    target_kind = None
                    for token, label in interesting.items():
                        if token in module.split("."):
                            target_kind = label
                            break
                    hints.append(
                        {
                            "from_file": rel,
                            "from_kind": src_type,
                            "to_module": module,
                            "to_kind": target_kind or "Unknown",
                            "relation": f"{src_type} imports {target_kind or 'Unknown'}",
                        }
                    )
                    if len(hints) >= max_items:
                        break

    return hints


def collect_dependency_hints(root: Path, max_items: int) -> list[dict[str, str]]:
    hints: list[dict[str, str]] = []
    files = [
        p for p in root.rglob("*.py") if not should_skip(p, root, include_hidden=False)
    ]

    service_names = {
        p.stem for p in files if "/services/" in p.relative_to(root).as_posix()
    }
    repo_names = {
        p.stem
        for p in files
        if "/repos/" in p.relative_to(root).as_posix()
        or "/repositories/" in p.relative_to(root).as_posix()
    }
    model_names = {
        p.stem for p in files if "/models/" in p.relative_to(root).as_posix()
    }

    for path in files:
        if len(hints) >= max_items:
            break
        rel = path.relative_to(root).as_posix()
        text = read_text_safe(path)
        if "/api/" in rel or path.name.startswith("routes"):
            for service in sorted(service_names):
                if service and re.search(rf"\b{re.escape(service)}\b", text):
                    hints.append(
                        {
                            "kind": "route_to_service",
                            "from": rel,
                            "to": service,
                            "hint": f"{rel} references service {service}",
                        }
                    )
                    if len(hints) >= max_items:
                        break
        if "/services/" in rel:
            for repo in sorted(repo_names):
                if repo and re.search(rf"\b{re.escape(repo)}\b", text):
                    hints.append(
                        {
                            "kind": "service_to_repo",
                            "from": rel,
                            "to": repo,
                            "hint": f"{rel} references repo {repo}",
                        }
                    )
                    if len(hints) >= max_items:
                        break
            for model in sorted(model_names):
                if model and re.search(rf"\b{re.escape(model)}\b", text):
                    hints.append(
                        {
                            "kind": "service_to_model",
                            "from": rel,
                            "to": model,
                            "hint": f"{rel} references model {model}",
                        }
                    )
                    if len(hints) >= max_items:
                        break
        if "/repos/" in rel or "/repositories/" in rel:
            for model in sorted(model_names):
                if model and re.search(rf"\b{re.escape(model)}\b", text):
                    hints.append(
                        {
                            "kind": "repo_to_model",
                            "from": rel,
                            "to": model,
                            "hint": f"{rel} references model {model}",
                        }
                    )
                    if len(hints) >= max_items:
                        break

    return hints


def parse_env_file(path: Path) -> list[tuple[str, str]]:
    if not path.exists():
        return []

    rows: list[tuple[str, str]] = []
    for raw_line in read_text_safe(path).splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()

        if not re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
            continue

        preview = "<set>" if value else "<empty>"
        rows.append((key, preview))

    return rows


def extract_env_refs_from_python(path: Path) -> list[str]:
    tree = try_parse_ast(path)
    if tree is None:
        return []

    found: set[str] = set()

    for node in ast.walk(tree):
        # os.getenv("NAME")
        if isinstance(node, ast.Call):
            if isinstance(node.func, ast.Attribute):
                if (
                    node.func.attr == "getenv"
                    and node.args
                    and isinstance(node.args[0], ast.Constant)
                    and isinstance(node.args[0].value, str)
                ):
                    key = node.args[0].value
                    if re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
                        found.add(key)
            # Field(..., env="NAME")
            for kw in node.keywords:
                if (
                    kw.arg == "env"
                    and isinstance(kw.value, ast.Constant)
                    and isinstance(kw.value.value, str)
                ):
                    key = kw.value.value
                    if re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
                        found.add(key)

        # os.environ["NAME"]
        if isinstance(node, ast.Subscript):
            if isinstance(node.value, ast.Attribute) and node.value.attr == "environ":
                sl = node.slice
                if isinstance(sl, ast.Constant) and isinstance(sl.value, str):
                    key = sl.value
                    if re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
                        found.add(key)

    return sorted(found)


def collect_env_map(root: Path, include_dotenv: bool) -> list[tuple[str, str, str]]:
    result: list[tuple[str, str, str]] = []

    for candidate in [root / ".env.example", root / ".env"]:
        if candidate.name == ".env" and not include_dotenv:
            continue
        for key, preview in parse_env_file(candidate):
            result.append((key, candidate.name, preview))

    for path in root.rglob("*.py"):
        if should_skip(path, root, include_hidden=False):
            continue
        rel = path.relative_to(root).as_posix()
        if any(token in rel for token in ["/core/", "config", "settings", ".env"]):
            for key in extract_env_refs_from_python(path):
                result.append((key, rel, "referenced"))

    dedup: dict[str, tuple[str, str, str]] = {}
    for key, source, preview in result:
        dedup[f"{key}:{source}"] = (key, source, preview)

    return sorted(dedup.values(), key=lambda x: (x[0], x[1]))


def collect_domain_entities(root: Path) -> list[str]:
    entities: list[str] = []
    for path in sorted(root.rglob("*.py")):
        if should_skip(path, root, include_hidden=False):
            continue
        rel = path.relative_to(root).as_posix()
        if "/models/" not in rel:
            continue
        if path.name.startswith("_") or path.name == "__init__.py":
            continue
        entities.append(path.stem)
    return entities


def collect_relationship_hints(root: Path, max_items: int) -> list[dict[str, str]]:
    hints: list[dict[str, str]] = []

    for path in sorted(root.rglob("*.py")):
        if len(hints) >= max_items:
            break
        if should_skip(path, root, include_hidden=False):
            continue
        rel = path.relative_to(root).as_posix()
        if "/models/" not in rel:
            continue
        text = read_text_safe(path)
        for pattern, label in [
            (r"ForeignKey\s*\(", "ForeignKey detected"),
            (r"relationship\s*\(", "ORM relationship detected"),
            (r"ManyToManyField\s*\(", "ManyToMany relation detected"),
            (r"OneToOneField\s*\(", "OneToOne relation detected"),
            (r"on_delete\s*=", "Deletion behavior configured"),
        ]:
            if re.search(pattern, text):
                hints.append({"file": rel, "hint": label})
                if len(hints) >= max_items:
                    break

    return hints


def collect_read_first(root: Path) -> list[str]:
    result = []
    for name in READ_FIRST_CANDIDATES:
        path = root / name
        if path.exists():
            result.append(name)

    dynamic_candidates = [
        "app/api/routes_admin.py",
        "app/api/routes_sync.py",
        "app/services/access_service.py",
        "app/services/uow.py",
        "app/models/user.py",
        "app/schemas/admin.py",
        "app/core/config.py",
        "app/core/settings.py",
        "app/api/urls.py",
    ]
    for fallback in dynamic_candidates:
        path = root / fallback
        if path.exists():
            result.append(fallback)

    for entry in detect_entry_points(root)[:20]:
        result.append(entry)

    seen = set()
    unique = []
    for item in result:
        if item not in seen:
            seen.add(item)
            unique.append(item)
    return unique


def collect_infra_map(root: Path) -> list[dict[str, str]]:
    infra: list[dict[str, str]] = []
    for path in iter_paths(root, include_hidden=True):
        rel = path.relative_to(root).as_posix()
        if path.is_dir():
            continue
        if any(
            token in rel
            for token in [
                "Dockerfile",
                "docker-compose.yml",
                "compose.yml",
                ".github/workflows",
                "nginx",
                "terraform",
                "helm",
                "k8s",
                "kubernetes",
            ]
        ):
            infra.append({"file": rel, "summary": summarize_path(rel)})
    return infra


def collect_sensitive_areas(root: Path, max_items: int) -> list[dict[str, str]]:
    items: list[dict[str, str]] = []
    for path in iter_paths(root, include_hidden=False):
        if len(items) >= max_items:
            break
        rel = path.relative_to(root).as_posix()
        lower = rel.lower()
        for token, label in SENSITIVE_NAME_HINTS.items():
            if token in lower:
                items.append({"file": rel, "reason": label})
                break
    return items


def collect_unknowns(
    root: Path,
    api_map: list[dict[str, Any]],
    entry_points: list[str],
    infra_map: list[dict[str, str]],
) -> list[str]:
    unknowns: list[str] = []
    if not entry_points:
        unknowns.append("Application startup entry point not clearly detected.")
    if not api_map:
        unknowns.append(
            "No HTTP route map detected, or framework route extraction is incomplete."
        )
    if not infra_map:
        unknowns.append("Deployment/infrastructure files not clearly detected.")
    if not (root / "README.md").exists():
        unknowns.append("README.md not found.")
    if not any(root.rglob("tests")):
        unknowns.append("Tests directory not clearly detected.")
    if (
        not any(root.rglob("*.sql"))
        and not any(root.rglob("alembic.ini"))
        and not (root / "migrations").exists()
    ):
        unknowns.append("Migration strategy not clearly detected.")
    return unknowns


def guess_flows(root: Path, dependency_hints: list[dict[str, str]]) -> list[str]:
    guesses: list[str] = []

    route_to_service = [h for h in dependency_hints if h["kind"] == "route_to_service"]
    service_to_repo = [h for h in dependency_hints if h["kind"] == "service_to_repo"]
    repo_to_model = [h for h in dependency_hints if h["kind"] == "repo_to_model"]

    if route_to_service:
        guesses.append("Route flow likely present: API handler -> service layer")
    if service_to_repo:
        guesses.append("Data flow likely present: service layer -> repository layer")
    if repo_to_model:
        guesses.append(
            "Persistence flow likely present: repository layer -> model/entity layer"
        )
    if (root / "app" / "services" / "uow.py").exists():
        guesses.append(
            "Transaction flow likely present: service/route -> UnitOfWork -> repositories -> commit/rollback"
        )
    if (root / "manage.py").exists():
        guesses.append(
            "Django flow likely present: URL config -> view -> service/model/ORM"
        )
    if any(p for p in root.rglob("*.py") if "APIRouter" in read_text_safe(p)):
        guesses.append(
            "FastAPI flow likely present: APIRouter -> endpoint handler -> service -> repository/database"
        )

    if not guesses:
        guesses.append("No reliable flow chain inferred automatically.")

    return guesses


def build_repo_snapshot(
    root: Path,
    include_hidden: bool,
    include_dotenv: bool,
    max_tree_entries: int,
    max_section_items: int,
) -> dict[str, Any]:
    project_name = root.resolve().name
    project_type = detect_project_type(root)
    architecture_signals = detect_architecture_signals(root)
    tree_lines = build_tree(
        root, include_hidden=include_hidden, max_entries=max_tree_entries
    )
    entry_points = detect_entry_points(root)
    module_map = collect_module_map(root)
    api_map = collect_api_map(root)
    env_map = collect_env_map(root, include_dotenv=include_dotenv)
    entities = collect_domain_entities(root)
    relationship_hints = collect_relationship_hints(root, max_section_items)
    read_first = collect_read_first(root)
    infra_map = collect_infra_map(root)
    import_hints = collect_import_hints(root, max_section_items)
    dependency_hints = collect_dependency_hints(root, max_section_items)
    sensitive_areas = collect_sensitive_areas(root, max_section_items)
    flow_guesses = guess_flows(root, dependency_hints)
    unknowns = collect_unknowns(root, api_map, entry_points, infra_map)

    return {
        "project_meta": {
            "name": project_name,
            "root": str(root.resolve()),
            "detected_stack_hints": project_type,
            "generated_by": "build_repo_map.py",
            "generated_at": datetime.now().isoformat(timespec="minutes"),
            "notes": "Auto-generated snapshot for AI/context sharing.",
        },
        "architecture_signals": architecture_signals,
        "directory_tree": tree_lines,
        "entry_points": entry_points,
        "critical_files_to_read_first": read_first,
        "module_map": module_map,
        "domain_model": {
            "entities": entities,
            "relationship_hints": relationship_hints,
        },
        "api_map": api_map,
        "env_config_map": [
            {"key": key, "source": source, "value": preview}
            for key, source, preview in env_map
        ],
        "infra_map": infra_map,
        "import_hints": import_hints,
        "dependency_hints": dependency_hints,
        "important_flows_auto_guessed": flow_guesses,
        "sensitive_areas": sensitive_areas,
        "unknowns_and_limitations": unknowns
        + [
            "Auto-generator does not infer business rules from full code semantics.",
            "Manual review is still required for auth, security, and destructive operations.",
            "Route extraction is partial for complex dynamic router setups.",
        ],
    }


def format_section(title: str) -> str:
    line = "=" * 72
    return f"{line}\n{title}\n{line}"


def sanitize_filename_part(value: str) -> str:
    """Return a filesystem-friendly filename fragment."""
    cleaned = re.sub(r"[^\w.-]+", "_", value.strip(), flags=re.UNICODE)
    cleaned = cleaned.strip("._-")
    return cleaned or "repository"


def make_default_output_path(root: Path, output_format: str) -> Path:
    """Build repo_map_<repo>_<ddmmyyhhmm>.<ext> inside the selected root."""
    repo_name = sanitize_filename_part(root.resolve().name)
    timestamp = datetime.now().strftime("%d%m%y%H%M")
    ext = "json" if output_format == "json" else output_format
    return root / f"repo_map_{repo_name}_{timestamp}.{ext}"


def render_txt(snapshot: dict[str, Any], max_section_items: int) -> str:
    lines: list[str] = []

    meta = snapshot["project_meta"]
    lines.append(format_section("PROJECT META"))
    lines.append(f"Name: {meta['name']}")
    lines.append(f"Root: {meta['root']}")
    lines.append(
        f"Detected stack/hints: {', '.join(meta['detected_stack_hints']) or 'Unknown'}"
    )
    lines.append(f"Generated by: {meta['generated_by']}")
    if meta.get("generated_at"):
        lines.append(f"Generated at: {meta['generated_at']}")
    lines.append(f"Notes: {meta['notes']}")
    lines.append("")

    lines.append(format_section("ARCHITECTURE SIGNALS"))
    if snapshot["architecture_signals"]:
        for item in snapshot["architecture_signals"][:max_section_items]:
            lines.append(
                f"- {item['signal']} :: {item['confidence']} :: evidence={item['evidence']}"
            )
    else:
        lines.append("(none detected)")
    lines.append("")

    lines.append(format_section("DIRECTORY TREE"))
    if snapshot["directory_tree"]:
        lines.extend(snapshot["directory_tree"][: max_section_items * 10])
    else:
        lines.append("(no visible files found)")
    lines.append("")

    lines.append(format_section("ENTRY POINTS"))
    if snapshot["entry_points"]:
        for item in snapshot["entry_points"][:max_section_items]:
            lines.append(f"- {item}")
    else:
        lines.append("(none detected)")
    lines.append("")

    lines.append(format_section("CRITICAL FILES TO READ FIRST"))
    if snapshot["critical_files_to_read_first"]:
        for idx, item in enumerate(
            snapshot["critical_files_to_read_first"][:max_section_items], start=1
        ):
            lines.append(f"{idx}. {item}")
    else:
        lines.append("(none detected)")
    lines.append("")

    lines.append(format_section("MODULE MAP"))
    module_map = snapshot["module_map"]
    if module_map:
        for group_name, files in module_map.items():
            lines.append(f"[{group_name}]")
            for rel in files[:max_section_items]:
                lines.append(f"- {rel} :: {summarize_path(rel)}")
            if len(files) > max_section_items:
                lines.append(f"- ... {len(files) - max_section_items} more")
            lines.append("")
    else:
        lines.append("(no modules detected)")
        lines.append("")

    lines.append(format_section("DOMAIN MODEL"))
    entities = snapshot["domain_model"]["entities"]
    rel_hints = snapshot["domain_model"]["relationship_hints"]
    if entities:
        lines.append("Entities:")
        for entity in entities[:max_section_items]:
            lines.append(f"- {entity}")
    else:
        lines.append("(no model entities detected)")
    lines.append("")
    if rel_hints:
        lines.append("Relationship hints:")
        for item in rel_hints[:max_section_items]:
            lines.append(f"- {item['file']} :: {item['hint']}")
    else:
        lines.append("(no relationship hints detected)")
    lines.append("")

    lines.append(format_section("API MAP"))
    if snapshot["api_map"]:
        for route in snapshot["api_map"][:max_section_items]:
            lines.append(
                f"- [{route['framework']}] {route['method']:6s} {route['path']} :: {route['handler']} [{route['file']}]"
            )
    else:
        lines.append("(no routes detected)")
    lines.append("")

    lines.append(format_section("IMPORT HINTS"))
    if snapshot["import_hints"]:
        for item in snapshot["import_hints"][:max_section_items]:
            lines.append(
                f"- {item['from_file']} :: {item['relation']} :: {item['to_module']}"
            )
    else:
        lines.append("(no import hints detected)")
    lines.append("")

    lines.append(format_section("DEPENDENCY HINTS"))
    if snapshot["dependency_hints"]:
        for item in snapshot["dependency_hints"][:max_section_items]:
            lines.append(f"- {item['hint']}")
    else:
        lines.append("(no dependency hints detected)")
    lines.append("")

    lines.append(format_section("ENV / CONFIG MAP"))
    if snapshot["env_config_map"]:
        for item in snapshot["env_config_map"][:max_section_items]:
            lines.append(
                f"- {item['key']} :: source={item['source']} :: value={item['value']}"
            )
    else:
        lines.append("(no env variables detected from .env.example/config)")
    lines.append("")

    lines.append(format_section("INFRA MAP"))
    if snapshot["infra_map"]:
        for item in snapshot["infra_map"][:max_section_items]:
            lines.append(f"- {item['file']} :: {item['summary']}")
    else:
        lines.append("(no infra files detected)")
    lines.append("")

    lines.append(format_section("IMPORTANT FLOWS (AUTO-GUESSED)"))
    for item in snapshot["important_flows_auto_guessed"][:max_section_items]:
        lines.append(f"- {item}")
    lines.append("")

    lines.append(format_section("SENSITIVE AREAS"))
    if snapshot["sensitive_areas"]:
        for item in snapshot["sensitive_areas"][:max_section_items]:
            lines.append(f"- {item['file']} :: {item['reason']}")
    else:
        lines.append("(none auto-detected)")
    lines.append("")

    lines.append(format_section("UNKNOWNS / LIMITATIONS"))
    for item in snapshot["unknowns_and_limitations"][:max_section_items]:
        lines.append(f"- {item}")
    lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    args = parse_args()
    root = Path(args.root).resolve()

    if not root.exists() or not root.is_dir():
        raise SystemExit(f"Root directory not found: {root}")

    snapshot = build_repo_snapshot(
        root=root,
        include_hidden=args.include_hidden,
        include_dotenv=args.include_dotenv,
        max_tree_entries=args.max_tree_entries,
        max_section_items=args.max_section_items,
    )

    if args.output:
        output_path = Path(args.output)
        if not output_path.is_absolute():
            output_path = root / output_path
    else:
        output_path = make_default_output_path(root, args.format)

    if args.format == "json":
        output_path.write_text(
            json.dumps(snapshot, ensure_ascii=False, indent=2), encoding="utf-8"
        )
    else:
        output_path.write_text(
            render_txt(snapshot, max_section_items=args.max_section_items),
            encoding="utf-8",
        )

    print(f"repo map written to: {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
