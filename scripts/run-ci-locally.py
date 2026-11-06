#!/usr/bin/env python3
"""Run a GitHub Actions workflow on this machine, job by job, in a container shaped like the hosted runner.

The workflow file is the program: every `run:` step executes verbatim, every `${{ }}` expression is evaluated with
GitHub's semantics, and every `uses:` action is replaced by the local equivalent in ACTIONS below. Anything the
runner does not understand (an unmapped action, an unknown runner label, a key or expression function it has no
meaning for) is refused by `plan`, never skipped, so the local run cannot silently drift from what CI runs.

    run-ci-locally.py plan  [--workflow W] [--event E]... [--input K=V]...   # parse, evaluate, give each leg's verdict
    run-ci-locally.py run   [--workflow W] [--tree DIR | --source REPO --ref REV] [--job J]... [--matrix k=v]...
                            [--event E] [--input K=V]... [--from-run ID] [--profile LABEL=CPUSxGB]...
                            [--timeout-scale X] [--keep]
    run-ci-locally.py exec  --run ID --job KEY -- CMD...             # a command in a finished job's container state
    run-ci-locally.py list                                           # the runs under the state directory
    run-ci-locally.py eval  [--workspace DIR] EXPR                   # one expression, hashFiles() over DIR

Each job runs in its own container from its label's image, scripts/test/ci-runner/<image>.Dockerfile (ubuntu:24.04
or ubuntu:22.04 plus the hosted image's packages), as uid 1001 `runner` with passwordless sudo, at the hosted paths
(/home/runner/work/<repo>/<repo>, RUNNER_TEMP=/home/runner/work/_temp). Rust is the hosted image's: today's stable
channel, `--profile minimal` plus rustfmt and clippy, in a RUSTUP_HOME copied fresh for every job, so a target one
job adds is not seen by the next. The size of the GitHub-hosted labels is applied: its cores pinned
(`--cpuset-cpus`, so nproc reads 4) and its memory a hard limit; a larger runner's size is the org's to define and is
taken from `--profile`, else the job runs unconstrained and the summary says its timing is not representative. On a
machine slower or busier than the runner, `--timeout-scale` stretches every timeout and the summary says so.

What this machine cannot be is named, never pretended: a leg on a macOS, Windows or arm64 label, a leg that needs a
job none of whose legs run here, and a leg that downloads an artifact only such a leg makes are each reported as
"not run locally" with the reason, and a run that left one out says so in its result instead of GREEN. No job that
holds a `write` permission runs, and no credential reaches a container: the local `github.token` is empty and nothing
from the host's git, ssh or gh configuration is mounted. A workflow_dispatch takes the inputs the workflow declares
(`--input`), so a release workflow runs as the dispatch that releases a tag.

Jobs run one at a time (one suite on the machine at a time). A job that passes gives its workspace back (its logs and
artifacts stay); a failing job's is kept for `exec`. State lives in $CI_LOCAL_STATE (default target/ci-local
in this repository; a full run of test.yml writes a few hundred gigabytes there, so point it at a roomy disk): the
toolchain store, a shared cargo registry, and each run's workspaces and artifacts. Logs go to logs/ci-local/<run>/ in
this repository, one file per step, with summary.txt beside them.
"""

import argparse
import datetime
import glob
import hashlib
import itertools
import json
import math
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import time
import urllib.request
import uuid

try:
    import yaml
except ImportError:  # python3-yaml ships with cloud-init on the hosted image and on this box
    sys.exit("run-ci-locally: needs PyYAML (python3-yaml)")

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
IMAGES_DIR = os.path.join(REPO_ROOT, "scripts", "test", "ci-runner")
STATE = os.path.abspath(os.environ.get("CI_LOCAL_STATE", os.path.join(REPO_ROOT, "target", "ci-local")))
LOG_ROOT = os.path.join(REPO_ROOT, "logs", "ci-local")

HOME = "/home/runner"
WORK = "/home/runner/work"
TEMP = f"{WORK}/_temp"
BASE_PATH = f"{HOME}/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# The GitHub-hosted runners this machine can be: each label's image (scripts/test/ci-runner/<image>.Dockerfile) and
# size. A public repository's standard runner has 4 vCPUs and 16 GB; a larger runner is named and sized by the
# organisation that owns it, so None means its size is not recorded here and must come from --profile.
RUNNERS = {
    "ubuntu-latest": ("ubuntu-24.04", (4, 16)),
    "ubuntu-24.04": ("ubuntu-24.04", (4, 16)),
    "ubuntu-22.04": ("ubuntu-22.04", (4, 16)),
    "ubuntu-latest-m": ("ubuntu-24.04", None),
}
IMAGE_OS = {"ubuntu-24.04": "ubuntu24", "ubuntu-22.04": "ubuntu22"}

# Hosted labels this machine cannot be: another operating system or another architecture. A leg on one is named in
# the plan and the run's result and never run, and nothing it would have made is pretended.
UNHOSTED = {
    **{label: ("a macOS host", "macOS", "ARM64") for label in ("macos-latest", "macos-14", "macos-15")},
    "macos-13": ("a macOS host", "macOS", "X64"),
    **{label: ("a Windows host", "Windows", "X64") for label in ("windows-latest", "windows-2022", "windows-2025")},
    **{label: ("an arm64 host", "Linux", "ARM64") for label in ("ubuntu-22.04-arm", "ubuntu-24.04-arm")},
}

WORKFLOW_KEYS = {"name", "on", "env", "jobs", "permissions", "concurrency", "defaults", "run-name"}
JOB_KEYS = {"name", "runs-on", "needs", "if", "timeout-minutes", "strategy", "steps", "env", "defaults", "outputs",
            "permissions", "continue-on-error"}
STEP_KEYS = {"name", "id", "uses", "with", "run", "shell", "env", "if", "working-directory", "continue-on-error",
             "timeout-minutes"}
STRATEGY_KEYS = {"matrix", "fail-fast", "max-parallel"}
EVENTS = {"push", "pull_request", "schedule", "workflow_dispatch"}
CONTEXTS = {"github", "env", "matrix", "strategy", "runner", "needs", "steps", "job", "inputs"}
FUNCTIONS = {"success", "failure", "always", "cancelled", "contains", "startswith", "endswith", "format", "join",
             "tojson", "fromjson", "hashfiles"}
STATUS_FUNCTIONS = {"success", "failure", "always", "cancelled"}


class Refusal(Exception):
    """Something in the workflow this runner has no faithful meaning for."""


# ───────────────────────────── expressions ─────────────────────────────

TOKEN = re.compile(r"""
    (?P<ws>\s+)
  | (?P<num>-?(?:0x[0-9a-fA-F]+|\d+(?:\.\d+)?(?:[eE][-+]?\d+)?))
  | (?P<str>'(?:[^']|'')*')
  | (?P<op>==|!=|<=|>=|&&|\|\||[<>!().,\[\]*])
  | (?P<ident>[A-Za-z_][A-Za-z0-9_-]*)
""", re.X)


def tokenize(src):
    out, i = [], 0
    while i < len(src):
        m = TOKEN.match(src, i)
        if not m:
            raise Refusal(f"expression `{src}`: cannot read `{src[i:]}`")
        i = m.end()
        kind = m.lastgroup
        if kind == "ws":
            continue
        text = m.group(kind)
        if kind == "num":
            out.append(("lit", int(text, 16) if text.lower().startswith("0x") else float(text)))
        elif kind == "str":
            out.append(("lit", text[1:-1].replace("''", "'")))
        elif kind == "ident" and text in ("true", "false", "null"):
            out.append(("lit", {"true": True, "false": False, "null": None}[text]))
        else:
            out.append((kind, text))
    return out


class Parser:
    """GitHub's expression grammar: || below && below equality below comparison below ! below access and calls."""

    def __init__(self, src):
        self.src, self.toks, self.i = src, tokenize(src), 0

    def peek(self, *ops):
        return self.i < len(self.toks) and self.toks[self.i][0] == "op" and self.toks[self.i][1] in ops

    def take(self, op):
        if not self.peek(op):
            raise Refusal(f"expression `{self.src}`: expected `{op}`")
        self.i += 1

    def parse(self):
        node = self.disjunction()
        if self.i != len(self.toks):
            raise Refusal(f"expression `{self.src}`: unexpected `{self.toks[self.i][1]}`")
        return node

    def binary(self, ops, below):
        node = below()
        while self.peek(*ops):
            op = self.toks[self.i][1]
            self.i += 1
            node = ("bin", op, node, below())
        return node

    def disjunction(self):
        return self.binary(("||",), self.conjunction)

    def conjunction(self):
        return self.binary(("&&",), self.equality)

    def equality(self):
        return self.binary(("==", "!="), self.comparison)

    def comparison(self):
        return self.binary(("<", ">", "<=", ">="), self.unary)

    def unary(self):
        if self.peek("!"):
            self.i += 1
            return ("not", self.unary())
        return self.postfix()

    def postfix(self):
        node = self.primary()
        while True:
            if self.peek("."):
                self.i += 1
                if self.i >= len(self.toks) or self.toks[self.i][0] != "ident":
                    raise Refusal(f"expression `{self.src}`: expected a property name after `.`")
                node = ("get", node, ("lit", self.toks[self.i][1]))
                self.i += 1
            elif self.peek("["):
                self.i += 1
                if self.peek("*"):
                    raise Refusal(f"expression `{self.src}`: object filters (`[*]`) are not supported")
                key = self.disjunction()
                self.take("]")
                node = ("get", node, key)
            else:
                return node

    def primary(self):
        if self.i >= len(self.toks):
            raise Refusal(f"expression `{self.src}`: ends early")
        kind, val = self.toks[self.i]
        self.i += 1
        if kind == "lit":
            return ("lit", val)
        if kind == "op" and val == "(":
            node = self.disjunction()
            self.take(")")
            return node
        if kind == "ident":
            if self.peek("("):
                self.i += 1
                args = []
                if not self.peek(")"):
                    args.append(self.disjunction())
                    while self.peek(","):
                        self.i += 1
                        args.append(self.disjunction())
                self.take(")")
                if val.lower() not in FUNCTIONS:
                    raise Refusal(f"expression `{self.src}`: function `{val}` is not supported locally")
                return ("call", val.lower(), args)
            if val not in CONTEXTS:
                raise Refusal(f"expression `{self.src}`: context `{val}` is not available locally")
            return ("ctx", val)
        raise Refusal(f"expression `{self.src}`: unexpected `{val}`")


def calls_in(node):
    if node[0] == "call":
        yield node[1]
        for a in node[2]:
            yield from calls_in(a)
    elif node[0] == "bin":
        yield from calls_in(node[2])
        yield from calls_in(node[3])
    elif node[0] in ("not",):
        yield from calls_in(node[1])
    elif node[0] == "get":
        yield from calls_in(node[1])
        yield from calls_in(node[2])


def to_number(v):
    if v is None:
        return 0.0
    if isinstance(v, bool):
        return 1.0 if v else 0.0
    if isinstance(v, (int, float)):
        return float(v)
    if isinstance(v, str):
        s = v.strip()
        if s == "":
            return 0.0
        try:
            return float(int(s, 16)) if s.lower().startswith("0x") else float(s)
        except ValueError:
            return math.nan
    return math.nan


def truthy(v):
    if v is None or v is False:
        return False
    if isinstance(v, (int, float)) and not isinstance(v, bool):
        return v != 0 and not math.isnan(v)
    if isinstance(v, str):
        return v != ""
    return True


def loose_equal(a, b):
    if type(a) is type(b) or (isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool)
                              and not isinstance(b, bool)):
        if isinstance(a, str):
            return a.casefold() == b.casefold()
        if isinstance(a, (dict, list)):
            return a is b
        return a == b
    if isinstance(a, (dict, list)) or isinstance(b, (dict, list)):
        return False
    x, y = to_number(a), to_number(b)
    return not (math.isnan(x) or math.isnan(y)) and x == y


def stringify(v):
    if v is None:
        return ""
    if v is True:
        return "true"
    if v is False:
        return "false"
    if isinstance(v, float):
        return str(int(v)) if v.is_integer() else repr(v)
    if isinstance(v, (dict, list)):
        return json.dumps(v, indent=2)
    return str(v)


def evaluate(node, ctx):
    kind = node[0]
    if kind == "lit":
        return node[1]
    if kind == "ctx":
        return ctx.get(node[1], {})
    if kind == "get":
        base, key = evaluate(node[1], ctx), evaluate(node[2], ctx)
        if isinstance(base, dict):
            if isinstance(key, str):
                for k, v in base.items():
                    if isinstance(k, str) and k.casefold() == key.casefold():
                        return v
            return None
        if isinstance(base, list):
            n = to_number(key)
            return base[int(n)] if not math.isnan(n) and 0 <= int(n) < len(base) else None
        return None
    if kind == "not":
        return not truthy(evaluate(node[1], ctx))
    if kind == "bin":
        op = node[1]
        if op == "&&":
            left = evaluate(node[2], ctx)
            return evaluate(node[3], ctx) if truthy(left) else left
        if op == "||":
            left = evaluate(node[2], ctx)
            return left if truthy(left) else evaluate(node[3], ctx)
        a, b = evaluate(node[2], ctx), evaluate(node[3], ctx)
        if op == "==":
            return loose_equal(a, b)
        if op == "!=":
            return not loose_equal(a, b)
        if isinstance(a, str) and isinstance(b, str):
            a, b = a.casefold(), b.casefold()
        else:
            a, b = to_number(a), to_number(b)
        return {"<": a < b, ">": a > b, "<=": a <= b, ">=": a >= b}[op]
    if kind == "call":
        name, args = node[1], [evaluate(a, ctx) for a in node[2]]
        status = ctx.get("job", {}).get("status", "success")
        if name == "success":
            return status == "success"
        if name == "failure":
            return status == "failure"
        if name == "always":
            return True
        if name == "cancelled":
            return status == "cancelled"
        if name == "contains":
            hay, needle = args
            if isinstance(hay, list):
                return any(loose_equal(x, needle) for x in hay)
            return stringify(needle).casefold() in stringify(hay).casefold()
        if name == "startswith":
            return stringify(args[0]).casefold().startswith(stringify(args[1]).casefold())
        if name == "endswith":
            return stringify(args[0]).casefold().endswith(stringify(args[1]).casefold())
        if name == "format":
            out = stringify(args[0])
            for i, a in enumerate(args[1:]):
                out = out.replace("{" + str(i) + "}", stringify(a))
            return out.replace("{{", "{").replace("}}", "}")
        if name == "join":
            sep = stringify(args[1]) if len(args) > 1 else ","
            return sep.join(stringify(x) for x in args[0]) if isinstance(args[0], list) else stringify(args[0])
        if name == "tojson":
            return json.dumps(args[0], indent=2)
        if name == "fromjson":
            return json.loads(stringify(args[0]))
        if name == "hashfiles":
            return hash_files(ctx.get("__workspace__"), [stringify(a) for a in args])
    raise Refusal(f"cannot evaluate {node!r}")


def glob_regex(pattern):
    """A workspace-relative glob as a regex over `/`-separated paths: `**` any number of directories, `*` and `?`
    within one path segment."""
    out, i = "", 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out, i = out + "(?:.*/)?", i + 3
        elif pattern.startswith("**", i):
            out, i = out + ".*", i + 2
        elif pattern[i] == "*":
            out, i = out + "[^/]*", i + 1
        elif pattern[i] == "?":
            out, i = out + "[^/]", i + 1
        else:
            out, i = out + re.escape(pattern[i]), i + 1
    return re.compile(out + r"\Z")


def hash_files(workspace, patterns):
    """`hashFiles(patterns)`: the files under the workspace the patterns match (a `!` pattern removes), each file's
    SHA-256 in path order fed to one SHA-256, as hex; the empty string when nothing matches. Before a run there is no
    workspace and the value is the empty string."""
    if not workspace:
        return ""
    keep = [glob_regex(p.strip().removeprefix("./")) for p in patterns if p.strip() and not p.strip().startswith("!")]
    drop = [glob_regex(p.strip()[1:].removeprefix("./")) for p in patterns if p.strip().startswith("!")]
    matched = []
    for root, dirs, files in os.walk(workspace):
        dirs.sort()
        for f in files:
            rel = os.path.relpath(os.path.join(root, f), workspace).replace(os.sep, "/")
            if any(k.match(rel) for k in keep) and not any(d.match(rel) for d in drop):
                matched.append(rel)
    if not matched:
        return ""
    total = hashlib.sha256()
    for rel in sorted(matched):
        with open(os.path.join(workspace, rel), "rb") as f:
            total.update(hashlib.sha256(f.read()).digest())
    return total.hexdigest()


TEMPLATE = re.compile(r"\$\{\{(.*?)\}\}", re.S)


def expressions_in(text):
    return [m.group(1).strip() for m in TEMPLATE.finditer(text)] if isinstance(text, str) else []


def sole_expression(text):
    """The expression `text` is exactly, `${{ e }}` and nothing beside it; None for any other string."""
    m = TEMPLATE.match(text)
    return m.group(1).strip() if m and m.end() == len(text) else None


def interpolate(text, ctx):
    """`text` with each `${{ }}` replaced by its value; a string that is one expression keeps the value's type."""
    if not isinstance(text, str):
        return text
    sole = sole_expression(text)
    if sole is not None:
        return evaluate(Parser(sole).parse(), ctx)
    return TEMPLATE.sub(lambda m: stringify(evaluate(Parser(m.group(1).strip()).parse(), ctx)), text)


def condition(text):
    """An `if:` value as an expression tree, with GitHub's implicit success() when no status function appears."""
    if isinstance(text, bool):
        return ("lit", text)
    src = str(text).strip()
    sole = sole_expression(src)
    if sole is not None:
        src = sole
    tree = Parser(src).parse()
    if not STATUS_FUNCTIONS.intersection(calls_in(tree)):
        tree = ("bin", "&&", ("call", "success", []), tree)
    return tree


# ───────────────────────────── the workflow model ─────────────────────────────

def load_workflow(path):
    with open(path) as f:
        try:
            wf = yaml.safe_load(f)
        except yaml.YAMLError as e:
            raise Refusal(f"{path} is not valid YAML: {e}")
    if not isinstance(wf, dict):
        raise Refusal(f"{path} is not a workflow (no top-level mapping)")
    if True in wf:  # YAML 1.1 reads the key `on` as a boolean
        wf["on"] = wf.pop(True)
    return wf


def triggers(wf):
    on = wf.get("on")
    if isinstance(on, str):
        return [on]
    if isinstance(on, list):
        return list(on)
    return list((on or {}).keys())


def matrix_legs(job):
    """The matrix's combinations as GitHub expands them: the axes' product, less every combination an `exclude` entry
    matches (partially is enough); then each `include` entry extends every one of those combinations whose original
    values it does not overwrite (values an earlier entry added may be), or is a combination of its own when it
    extends none."""
    strategy = job.get("strategy") or {}
    unknown = set(strategy) - STRATEGY_KEYS
    if unknown:
        raise Refusal(f"strategy keys {sorted(unknown)} are not supported locally")
    matrix = strategy.get("matrix")
    if not matrix:
        return [{}]
    if not isinstance(matrix, dict):
        raise Refusal("a matrix given by an expression is not supported locally")
    axes = [k for k in matrix if k not in ("include", "exclude")]
    for k in axes:
        if not isinstance(matrix[k], list):
            raise Refusal(f"matrix axis `{k}` is not a list")
    entries = {}
    for k in ("include", "exclude"):
        entries[k] = matrix.get(k) or []
        if not isinstance(entries[k], list) or not all(isinstance(e, dict) for e in entries[k]):
            raise Refusal(f"matrix `{k}` is not a list of mappings")
    combos = [dict(zip(axes, c)) for c in itertools.product(*(matrix[k] for k in axes))] if axes else []
    combos = [c for c in combos
              if not any(all(k in c and loose_equal(c[k], v) for k, v in e.items()) for e in entries["exclude"])]
    extra = []
    for e in entries["include"]:
        fits = [c for c in combos if all(k not in axes or loose_equal(c[k], v) for k, v in e.items())]
        for c in fits:
            c.update(e)
        if not fits:
            extra.append(dict(e))
    return (combos + extra) or [{}]


def job_key(name, leg):
    return name if not leg else f"{name} ({', '.join(stringify(v) for v in leg.values())})"


def writes(permissions):
    if permissions in ("write-all",):
        return True
    if isinstance(permissions, dict):
        return any(v == "write" for v in permissions.values())
    return False


def action_ref(uses):
    return uses.strip()


def check_workflow(wf):
    """Refuse anything the runner would have to guess at. Returns the jobs in dependency order."""
    problems = []
    for k in wf:
        if k not in WORKFLOW_KEYS:
            problems.append(f"workflow key `{k}`")
    for ev in triggers(wf):
        if ev not in EVENTS:
            problems.append(f"trigger `{ev}`")
    jobs = wf.get("jobs") or {}

    def scan(where, value):
        if isinstance(value, dict):
            for k, v in value.items():
                scan(where, v)
        elif isinstance(value, list):
            for v in value:
                scan(where, v)
        elif isinstance(value, str):
            for e in expressions_in(value):
                try:
                    Parser(e).parse()
                except Refusal as r:
                    problems.append(f"{where}: {r}")

    scan("workflow env", wf.get("env") or {})
    for name, job in jobs.items():
        for k in job:
            if k not in JOB_KEYS:
                problems.append(f"job `{name}`: key `{k}`")
        try:
            legs = matrix_legs(job)
        except Refusal as r:
            problems.append(f"job `{name}`: {r}")
            legs = [{}]
        for need in as_list(job.get("needs")):
            if need not in jobs:
                problems.append(f"job `{name}` needs unknown job `{need}`")
        if "if" in job:
            try:
                condition(job["if"])
            except Refusal as r:
                problems.append(f"job `{name}` if: {r}")
        labels = job.get("runs-on")
        hosted = []
        for leg in legs:
            ctx = {"matrix": leg, "github": {}, "env": {}}
            try:
                label = interpolate(labels, ctx) if isinstance(labels, str) else labels
            except Refusal as r:
                problems.append(f"job `{name}` runs-on: {r}")
                continue
            for lab in as_list(label):
                if lab not in RUNNERS and lab not in UNHOSTED:
                    problems.append(f"job `{name}`: runner label `{lab}` has no local profile")
            if as_list(label) and as_list(label)[0] in RUNNERS:
                hosted.append((leg, as_list(label)[0]))
        scan(f"job `{name}`", {k: v for k, v in job.items() if k not in ("steps", "if")})
        for i, step in enumerate(job.get("steps") or []):
            where = f"job `{name}` step {i + 1} ({step.get('name') or step.get('uses') or 'run'})"
            for k in step:
                if k not in STEP_KEYS:
                    problems.append(f"{where}: key `{k}`")
            if ("uses" in step) == ("run" in step):
                problems.append(f"{where}: needs exactly one of `uses` and `run`")
            publishes = writes(job.get("permissions")) or writes(wf.get("permissions"))
            if "uses" in step and action_ref(step["uses"]) not in ACTIONS and not publishes and any(
                    might_run(step, wf, ev, leg, label) for ev in triggers(wf) for leg, label in hosted):
                problems.append(f"{where}: action `{step['uses']}` has no local equivalent")
            if "shell" in step and step["shell"] not in SHELLS and any(
                    might_run(step, wf, ev, leg, label) for ev in triggers(wf) for leg, label in hosted):
                problems.append(f"{where}: shell `{step['shell']}` is not supported locally")
            if "if" in step:
                try:
                    condition(step["if"])
                except Refusal as r:
                    problems.append(f"{where} if: {r}")
            scan(where, {k: v for k, v in step.items() if k != "if"})
        dshell = ((job.get("defaults") or {}).get("run") or {}).get("shell")
        if dshell and dshell not in SHELLS:
            problems.append(f"job `{name}`: default shell `{dshell}` is not supported locally")
    order, seen = [], set()

    def visit(n, stack):
        if n in seen:
            return
        if n in stack:
            problems.append(f"job `{n}`: dependency cycle")
            return
        for d in as_list(jobs.get(n, {}).get("needs")):
            if d in jobs:
                visit(d, stack | {n})
        seen.add(n)
        order.append(n)

    for n in jobs:
        visit(n, frozenset())
    if problems:
        raise Refusal("the workflow uses what the local runner cannot run faithfully:\n  - " + "\n  - ".join(problems))
    return order


def as_list(v):
    if v is None:
        return []
    return v if isinstance(v, list) else [v]


def might_run(step, wf, event, leg, label):
    """Whether `step` can run on a leg of `label` under `event`: its `if:` is not known false before the run."""
    if "if" not in step or needs_runtime(step["if"]):
        return True
    try:
        return truthy(evaluate(condition(step["if"]), plan_ctx(wf, event, "refs/heads/main", {}, leg, label)))
    except Refusal:
        return True


def runner_ctx(label):
    """The `runner` context of a leg on `label`: this machine's Linux x64 for a hosted label, the label's own
    operating system and architecture for one it cannot host."""
    os_name, arch = ("Linux", "X64") if label not in UNHOSTED else UNHOSTED[label][1:]
    return {"os": os_name, "arch": arch, "temp": TEMP, "tool_cache": "/opt/hostedtoolcache", "name": "ci-local",
            "environment": "github-hosted"}


def dispatch_inputs(wf, event, given):
    """The `inputs` context: for a workflow_dispatch, every input the workflow declares, its given value or its
    default, typed as declared; empty for any other event. An input the workflow does not declare is refused, and a
    required one not given."""
    on = wf.get("on") if isinstance(wf.get("on"), dict) else {}
    declared = ((on.get("workflow_dispatch") or {}).get("inputs") or {}) if "workflow_dispatch" in on else {}
    for k in given:
        if k not in declared:
            raise Refusal(f"input `{k}` is not declared by the workflow's workflow_dispatch (it declares "
                          f"{sorted(declared) or 'none'})")
    if event != "workflow_dispatch":
        return {}
    out = {}
    for k, spec in declared.items():
        spec = spec or {}
        kind = spec.get("type", "string")
        if k in given:
            v = given[k]
        elif "default" in spec:
            v = spec["default"]
        elif spec.get("required"):
            raise Refusal(f"input `{k}` is required by the workflow's workflow_dispatch")
        else:
            v = False if kind == "boolean" else ""
        if kind == "boolean":
            out[k] = v if isinstance(v, bool) else stringify(v).lower() == "true"
        elif kind == "number":
            out[k] = to_number(v)
        else:
            out[k] = stringify(v) if v is not None else ""
    return out


def github_ctx(event, ref_name, inputs):
    return {"event_name": event, "ref": ref_name, "ref_name": ref_name.split("/")[-1],
            "ref_type": "tag" if ref_name.startswith("refs/tags/") else "branch",
            "event": {"inputs": inputs} if event == "workflow_dispatch" else {}}


def plan_ctx(wf, event, ref_name, inputs, leg, label):
    """What is known of a leg before it runs: the event, the ref, the inputs, the matrix, the runner and the
    workflow's env; needs, steps and job outputs read as empty."""
    ctx = {"github": github_ctx(event, ref_name, inputs), "env": {}, "matrix": leg or {}, "needs": {}, "steps": {},
           "job": {"status": "success"}, "runner": runner_ctx(label), "strategy": {}, "inputs": inputs}
    ctx["env"] = {k: stringify(interpolate(v, ctx)) for k, v in (wf.get("env") or {}).items()}
    return ctx


def artifacts_of(job, ctx, action):
    """The artifact names a leg's `action` steps (upload or download) name, for the steps not known to be skipped."""
    names = []
    for step in job.get("steps") or []:
        if not str(step.get("uses", "")).startswith(action + "@"):
            continue
        if "if" in step and not needs_runtime(step["if"]) and not truthy(evaluate(condition(step["if"]), ctx)):
            continue
        name = interpolate((step.get("with") or {}).get("name"), ctx)
        if name:
            names.append(stringify(name))
    return names


def local_verdicts(wf, order, event, ref_name, inputs):
    """What this machine does with every leg of every job under `event`: {job: [(leg, label, verdict)]}, the verdict
    "runs", "skipped by its if:" or "not run locally: <why>" — the job publishes (a write permission), its runner is
    one this machine cannot host, it needs a job none of whose legs run here, or it downloads an artifact only a leg
    not run here makes."""
    verdicts, made_here, made_elsewhere, nowhere = {}, set(), set(), set()
    for name in order:
        job = wf["jobs"][name]
        legs = []
        for leg in matrix_legs(job):
            label = as_list(interpolate(job.get("runs-on"), plan_ctx(wf, event, ref_name, inputs, leg, None)))[0]
            ctx = plan_ctx(wf, event, ref_name, inputs, leg, label)
            needed = [d for d in as_list(job.get("needs")) if d in nowhere]
            if writes(job.get("permissions")) or writes(wf.get("permissions")):
                verdict = "not run locally: holds a write permission"
            elif label in UNHOSTED:
                verdict = f"not run locally: needs {UNHOSTED[label][0]}"
            elif needed:
                verdict = f"not run locally: needs {needed[0]}, which does not run locally"
            elif "if" in job and not needs_runtime(job["if"]) and not truthy(evaluate(condition(job["if"]), ctx)):
                verdict = "skipped by its if:"
            else:
                missing = [a for a in artifacts_of(job, ctx, "actions/download-artifact")
                           if a in made_elsewhere and a not in made_here]
                verdict = (f"not run locally: needs the artifact {missing[0]}, made on a leg this machine cannot host"
                           if missing else "runs")
            made = artifacts_of(job, ctx, "actions/upload-artifact")
            if verdict == "runs":
                made_here.update(made)
            elif verdict.startswith("not run locally"):
                made_elsewhere.update(made)
            legs.append((leg, label, verdict))
        if all(v.startswith("not run locally") for _, _, v in legs):
            nowhere.add(name)
        verdicts[name] = legs
    return verdicts


SHELLS = {
    None: ["bash", "-e", "{0}"],
    "bash": ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "{0}"],
    "sh": ["sh", "-e", "{0}"],
}


# ───────────────────────────── host side: sources, toolchains, containers ─────────────────────────────

def sh(cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)


def git(*args, cwd=None, capture=False):
    r = subprocess.run(["git", *args], cwd=cwd, check=True, text=True,
                       stdout=subprocess.PIPE if capture else None)
    return r.stdout.strip() if capture else None


def snapshot_tree(tree, dest):
    """The working tree of `tree` as one commit in a fresh repository at `dest`: what `git add -A` would commit."""
    files = subprocess.run(["git", "-C", tree, "ls-files", "-z", "-co", "--exclude-standard"], check=True,
                           stdout=subprocess.PIPE).stdout.split(b"\0")
    present = [f for f in files if f and os.path.lexists(os.path.join(tree.encode(), f))]
    os.makedirs(dest)
    pack = subprocess.Popen(["tar", "-C", tree, "--null", "-T", "-", "-cf", "-"], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE)
    unpack = subprocess.Popen(["tar", "-C", dest, "-xf", "-"], stdin=pack.stdout)
    pack.stdin.write(b"\0".join(present))
    pack.stdin.close()
    if unpack.wait() != 0 or pack.wait() != 0:
        raise SystemExit("run-ci-locally: copying the working tree failed")
    env = dict(os.environ, GIT_AUTHOR_NAME="ci-local", GIT_AUTHOR_EMAIL="ci-local@localhost",
               GIT_COMMITTER_NAME="ci-local", GIT_COMMITTER_EMAIL="ci-local@localhost")
    for args in (["init", "-q", "-b", "main"], ["add", "-A"], ["commit", "-q", "-m", f"working tree of {tree}"]):
        subprocess.run(["git", "-C", dest, *args], check=True, env=env)
    return git("-C", dest, "rev-parse", "HEAD", capture=True)


def stable_version():
    try:
        with urllib.request.urlopen("https://static.rust-lang.org/dist/channel-rust-stable.toml", timeout=30) as r:
            text = r.read().decode()
        m = re.search(r'\[pkg\.rust\]\s*\nversion = "([0-9.]+)', text)
        return m.group(1) if m else None
    except OSError:
        return None


def toolchain_store():
    """A RUSTUP_HOME holding what the hosted image ships: the stable channel, minimal profile, rustfmt and clippy."""
    root = os.path.join(STATE, "toolchain-store")
    os.makedirs(root, exist_ok=True)
    version = stable_version()
    stores = sorted(d for d in os.listdir(root) if d.startswith("rustup-") and not d.endswith(".partial"))
    if version and f"rustup-{version}" in stores:
        return os.path.join(root, f"rustup-{version}"), version
    if not version:
        if not stores:
            raise SystemExit("run-ci-locally: cannot reach static.rust-lang.org and no toolchain store exists")
        print(f"run-ci-locally: stable channel unreachable; using {stores[-1]}", file=sys.stderr)
        return os.path.join(root, stores[-1]), stores[-1][len("rustup-"):]
    partial = os.path.join(root, f"rustup-{version}.partial")
    shutil.rmtree(partial, ignore_errors=True)
    cargo_home = partial + "-cargo"
    os.makedirs(os.path.join(cargo_home, "bin"), exist_ok=True)
    env = dict(os.environ, RUSTUP_HOME=partial, CARGO_HOME=cargo_home)
    rustup = shutil.which("rustup")
    sh([rustup, "toolchain", "install", "stable", "--profile", "minimal", "-c", "rustfmt", "-c", "clippy",
        "--no-self-update"], env=env)
    sh([rustup, "default", "stable"], env=env)
    shutil.rmtree(cargo_home)
    os.rename(partial, os.path.join(root, f"rustup-{version}"))
    return os.path.join(root, f"rustup-{version}"), version


RUSTUP_PROXIES = ["cargo", "rustc", "rustdoc", "rustfmt", "cargo-fmt", "cargo-clippy", "clippy-driver", "rust-gdb",
                  "rust-gdbgui", "rust-lldb", "rust-analyzer", "cargo-miri", "rls"]


def image_tag(image):
    return f"logos-ci-runner:{image}"


def ensure_image(image):
    """The container image for `image` (scripts/test/ci-runner/<image>.Dockerfile), rebuilt when its Dockerfile
    changed since it was built."""
    tag, dockerfile_path = image_tag(image), os.path.join(IMAGES_DIR, f"{image}.Dockerfile")
    have = subprocess.run(["docker", "image", "inspect", tag], stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL).returncode == 0
    digest_file = os.path.join(STATE, f"image-{image}.dockerfile")
    with open(dockerfile_path) as f:
        dockerfile = f.read()
    built_from = open(digest_file).read() if os.path.exists(digest_file) else None
    if not have or built_from != dockerfile:
        print(f"run-ci-locally: building {tag}", flush=True)
        sh(["docker", "build", "-t", tag, "-f", dockerfile_path, IMAGES_DIR], stdout=subprocess.DEVNULL)
        with open(digest_file, "w") as f:
            f.write(dockerfile)


def node_major():
    try:
        out = subprocess.run(["node", "--version"], check=True, text=True, stdout=subprocess.PIPE).stdout
        return int(out.strip().lstrip("v").split(".")[0])
    except (OSError, subprocess.CalledProcessError, ValueError):
        return None


def slug(text):
    return re.sub(r"[^A-Za-z0-9]+", "-", text).strip("-").lower()[:60] or "step"


# ───────────────────────────── a run ─────────────────────────────

class Run:
    def __init__(self, args, wf, source, sha, run_id):
        self.args, self.wf, self.source, self.sha, self.id = args, wf, source, sha, run_id
        self.dir = os.path.join(STATE, "runs", run_id)
        self.logs = os.path.join(LOG_ROOT, run_id)
        self.artifacts = os.path.join(self.dir, "artifacts")
        self.repo = args.repository
        self.repo_name = self.repo.split("/")[-1]
        self.workspace = f"{WORK}/{self.repo_name}/{self.repo_name}"
        self.inputs = dispatch_inputs(wf, args.event, given_inputs(args))
        self.state = {"id": run_id, "source": source, "sha": sha, "event": args.event, "workflow": args.workflow,
                      "inputs": self.inputs, "jobs": {}}
        self.profiles = {label: profile for label, (_, profile) in RUNNERS.items()}
        for p in args.profile or []:
            label, _, size = p.partition("=")
            cpus, _, gb = size.lower().partition("x")
            self.profiles[label] = (int(cpus), int(gb))
        self.container = None

    def save(self):
        with open(os.path.join(self.dir, "run.json"), "w") as f:
            json.dump(self.state, f, indent=2)

    def github(self, job_name):
        return {
            **github_ctx(self.args.event, self.args.ref_name, self.inputs),
            "repository": self.repo, "repository_owner": self.repo.split("/")[0], "sha": self.sha,
            "workspace": self.workspace, "token": "", "run_id": self.id, "run_number": "1", "run_attempt": "1",
            "actor": "ci-local", "job": job_name, "server_url": "https://github.com", "api_url": "https://api.github.com",
            "workflow": self.wf.get("name", ""), "base_ref": "", "head_ref": "",
        }

    def log(self, line):
        print(line, flush=True)
        with open(os.path.join(self.logs, "summary.txt"), "a") as f:
            f.write(line + "\n")

    # The job's container: started once, every step a `docker exec` in it.
    def start_container(self, key, jobdir, profile, image):
        name = f"ci-local-{self.id}-{slug(key)}"
        for d in ("workdir", "tmp"):
            os.makedirs(os.path.join(jobdir, d), exist_ok=True)
        os.chmod(os.path.join(jobdir, "tmp"), 0o1777)
        os.makedirs(os.path.join(jobdir, "workdir", "_temp"), exist_ok=True)
        os.makedirs(self.host_path(jobdir, self.workspace), exist_ok=True)
        os.makedirs(os.path.join(STATE, "cache", "cargo-registry"), exist_ok=True)
        os.makedirs(os.path.join(STATE, "cache", "cargo-git"), exist_ok=True)
        cmd = ["docker", "run", "-d", "--name", name, "--init", "--user", "runner", "--hostname", "ci-local",
               "--shm-size", "2g", "--ulimit", "nofile=65536:65536", "--security-opt", "seccomp=unconfined",
               "--cap-add", "SYS_PTRACE",
               "-v", f"{jobdir}/workdir:{WORK}", "-v", f"{jobdir}/tmp:/tmp",
               "-v", f"{jobdir}/rustup:{HOME}/.rustup", "-v", f"{jobdir}/cargo:{HOME}/.cargo",
               "-v", f"{STATE}/cache/cargo-registry:{HOME}/.cargo/registry",
               "-v", f"{STATE}/cache/cargo-git:{HOME}/.cargo/git",
               "-v", f"{os.path.realpath(shutil.which('node'))}:/usr/local/bin/node:ro",
               "-v", "/usr/lib/node_modules:/usr/local/lib/node_modules:ro",
               "-v", "/opt/google/chrome:/opt/google/chrome:ro"]
        cmd += resource_flags(profile)
        cmd += [image_tag(image), "sleep", "infinity"]
        subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        sh(cmd, stdout=subprocess.DEVNULL)
        self.container = name
        return name

    def stop_container(self):
        if self.container and not self.args.keep:
            subprocess.run(["docker", "rm", "-f", self.container], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.container = None

    def host_path(self, jobdir, path):
        path = os.path.normpath(path)
        if path == WORK or path.startswith(WORK + "/"):
            return os.path.join(jobdir, "workdir", os.path.relpath(path, WORK))
        if path == "/tmp" or path.startswith("/tmp/"):
            return os.path.join(jobdir, "tmp", os.path.relpath(path, "/tmp"))
        raise Refusal(f"path `{path}` is outside the job's mounts")

    def exec_script(self, script, env, workdir, logfile, timeout):
        """Run `script` (argv) in the job's container; True on exit 0. A timeout removes the container."""
        cmd = ["docker", "exec", "-w", workdir]
        for k, v in env.items():
            cmd += ["-e", f"{k}={v}"]
        cmd += [self.container, *script]
        with open(logfile, "ab") as out:
            p = subprocess.Popen(cmd, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                return p.wait(timeout=timeout) == 0, False
            except subprocess.TimeoutExpired:
                subprocess.run(["docker", "rm", "-f", self.container], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL)
                os.killpg(p.pid, signal.SIGKILL)
                p.wait()
                return False, True

    def run_job(self, name, job, leg, needs_ctx):
        key = job_key(name, leg)
        jobdir = os.path.join(self.dir, "jobs", slug(key))
        logdir = os.path.join(self.logs, slug(key))
        os.makedirs(logdir, exist_ok=True)
        os.makedirs(jobdir, exist_ok=True)
        wf_env = self.wf.get("env") or {}
        base_ctx = {"github": self.github(name), "matrix": leg, "needs": needs_ctx, "env": {}, "job": {"status": "success"},
                    "runner": runner_ctx(None), "inputs": self.inputs,
                    "strategy": {"fail-fast": (job.get("strategy") or {}).get("fail-fast", True)}, "steps": {},
                    "__workspace__": self.host_path(jobdir, self.workspace)}
        env = {k: stringify(interpolate(v, base_ctx)) for k, v in wf_env.items()}
        base_ctx["env"] = dict(env)
        env.update({k: stringify(interpolate(v, base_ctx)) for k, v in (job.get("env") or {}).items()})
        labels = as_list(interpolate(job.get("runs-on"), base_ctx))
        image, _ = RUNNERS[labels[0]]
        profile = self.profiles.get(labels[0])
        timeout_min = float(interpolate(job.get("timeout-minutes", 360), base_ctx)) * self.args.timeout_scale
        record = {"key": key, "job": name, "label": labels[0], "image": image, "profile": profile,
                  "timeout_minutes": timeout_min, "steps": [], "outputs": {}, "dir": jobdir}
        self.state["jobs"][key] = record
        started = time.monotonic()
        deadline = started + timeout_min * 60
        self.log(f"── job {key}  [{labels[0]}: {'%d CPU / %d GB' % profile if profile else 'size not recorded, unconstrained'}"
                 f", timeout {timeout_min:g} min]")

        rustup_home = os.path.join(jobdir, "rustup")
        cargo_home = os.path.join(jobdir, "cargo")
        shutil.rmtree(rustup_home, ignore_errors=True)
        shutil.rmtree(cargo_home, ignore_errors=True)
        sh(["cp", "-a", self.toolchain, rustup_home])
        os.makedirs(os.path.join(cargo_home, "bin"))
        shutil.copy2(os.path.realpath(shutil.which("rustup")), os.path.join(cargo_home, "bin", "rustup"))
        for p in RUSTUP_PROXIES:
            os.symlink("rustup", os.path.join(cargo_home, "bin", p))
        self.start_container(key, jobdir, profile, image)

        runtime_env = {
            "CI": "true", "GITHUB_ACTIONS": "true", "HOME": HOME, "PATH": BASE_PATH, "LANG": "C.UTF-8",
            "GITHUB_WORKSPACE": self.workspace, "GITHUB_REPOSITORY": self.repo, "GITHUB_EVENT_NAME": self.args.event,
            "GITHUB_REF": self.args.ref_name, "GITHUB_REF_NAME": self.args.ref_name.split("/")[-1],
            "GITHUB_SHA": self.sha, "GITHUB_RUN_ID": self.id, "GITHUB_RUN_NUMBER": "1", "GITHUB_JOB": name,
            "GITHUB_ACTOR": "ci-local", "GITHUB_SERVER_URL": "https://github.com",
            "GITHUB_API_URL": "https://api.github.com", "GITHUB_WORKFLOW": self.wf.get("name", ""),
            "RUNNER_OS": "Linux", "RUNNER_ARCH": "X64", "RUNNER_TEMP": TEMP, "RUNNER_NAME": "ci-local",
            "RUNNER_TOOL_CACHE": "/opt/hostedtoolcache", "RUNNER_ENVIRONMENT": "github-hosted", "ImageOS": IMAGE_OS[image],
            "RUSTUP_HOME": f"{HOME}/.rustup", "CARGO_HOME": f"{HOME}/.cargo",
        }
        exported, paths = {}, []
        status = "success"
        steps_ctx = base_ctx["steps"]
        defaults = (job.get("defaults") or {}).get("run") or {}
        try:
            for i, step in enumerate(job.get("steps") or []):
                label = step.get("name") or step.get("uses") or step.get("run", "").splitlines()[0]
                ctx = dict(base_ctx, env={**env, **exported}, job={"status": status})
                step_env = {k: stringify(interpolate(v, ctx)) for k, v in (step.get("env") or {}).items()}
                ctx["env"] = {**env, **exported, **step_env}
                label = stringify(interpolate(label, ctx))
                logfile = os.path.join(logdir, f"{i + 1:02d}-{slug(label)}.log")
                cond = condition(step.get("if", "success()"))
                if not truthy(evaluate(cond, ctx)):
                    record["steps"].append({"name": label, "result": "skipped"})
                    self.log(f"   skip  {label}")
                    if "id" in step:
                        steps_ctx[step["id"]] = {"outputs": {}, "outcome": "skipped", "conclusion": "skipped"}
                    continue
                remaining = deadline - time.monotonic()
                if "timeout-minutes" in step:
                    remaining = min(remaining,
                                    float(interpolate(step["timeout-minutes"], ctx)) * 60 * self.args.timeout_scale)
                t0 = time.monotonic()
                files = {k: f"{TEMP}/_runner_file_commands/{k.lower()}_{uuid.uuid4().hex}" for k in
                         ("GITHUB_ENV", "GITHUB_PATH", "GITHUB_OUTPUT", "GITHUB_STEP_SUMMARY")}
                os.makedirs(self.host_path(jobdir, f"{TEMP}/_runner_file_commands"), exist_ok=True)
                for f in files.values():
                    open(self.host_path(jobdir, f), "w").close()
                full_env = {**runtime_env, **env, **exported, **step_env, **files,
                            "PATH": ":".join(reversed(paths)) + (":" if paths else "") + BASE_PATH}
                wd_rel = step.get("working-directory") or defaults.get("working-directory")
                workdir = os.path.normpath(os.path.join(self.workspace, stringify(interpolate(wd_rel, ctx)))) \
                    if wd_rel else self.workspace
                with open(logfile, "w") as f:
                    f.write(f"# {label}\n")
                timed_out = False
                if "run" in step:
                    script = interpolate(step["run"], ctx)
                    host_script = self.host_path(jobdir, f"{TEMP}/{uuid.uuid4()}.sh")
                    with open(host_script, "w") as f:
                        f.write(script)
                    shell = step.get("shell") or defaults.get("shell")
                    argv = [a.replace("{0}", self.container_path(jobdir, host_script)) for a in SHELLS[shell]]
                    with open(logfile, "a") as f:
                        f.write(f"$ {' '.join(argv[:-1])} <<'SCRIPT'\n{script}\nSCRIPT\n")
                    ok, timed_out = self.exec_script(argv, full_env, workdir, logfile, max(remaining, 1))
                else:
                    with_ = {k: interpolate(v, ctx) for k, v in (step.get("with") or {}).items()}
                    ok, timed_out = ACTIONS[action_ref(step["uses"])](self, jobdir, step["uses"], with_, full_env,
                                                                         workdir, logfile, max(remaining, 1))
                for k, v in parse_file_commands(self.host_path(jobdir, files["GITHUB_ENV"])).items():
                    exported[k] = v
                with open(self.host_path(jobdir, files["GITHUB_PATH"])) as f:
                    paths += [l.strip() for l in f if l.strip()]
                outputs = parse_file_commands(self.host_path(jobdir, files["GITHUB_OUTPUT"]))
                elapsed = time.monotonic() - t0
                outcome = "success" if ok else "failure"
                conclusion = "success" if ok or step.get("continue-on-error") else "failure"
                if "id" in step:
                    steps_ctx[step["id"]] = {"outputs": outputs, "outcome": outcome, "conclusion": conclusion}
                record["steps"].append({"name": label, "result": "timed out" if timed_out else outcome,
                                        "seconds": round(elapsed), "log": logfile})
                self.log(f"   {'TIMEOUT' if timed_out else 'ok   ' if ok else 'FAIL '} {label}  ({fmt_secs(elapsed)})"
                         + ("" if ok else f"  → {os.path.relpath(logfile, REPO_ROOT)}"))
                if timed_out:
                    status = "failure"
                    record["timed_out"] = True
                    break
                if conclusion == "failure":
                    status = "failure"
            ctx = dict(base_ctx, env={**env, **exported}, job={"status": status})
            record["outputs"] = {k: stringify(interpolate(v, ctx)) for k, v in (job.get("outputs") or {}).items()}
            record["env"] = {**runtime_env, **env, **exported,
                             "PATH": ":".join(reversed(paths)) + (":" if paths else "") + BASE_PATH}
        finally:
            self.stop_container()
        record["result"] = status
        record["seconds"] = round(time.monotonic() - started)
        record["disk_gb"] = round(du_gb(jobdir), 1)
        self.log(f"   {status.upper()}  {key} in {fmt_secs(record['seconds'])} (timeout {timeout_min:g} min), "
                 f"{record['disk_gb']} GB on disk")
        if status == "success" and not self.args.keep_workspaces:
            for d in ("workdir", "tmp", "rustup", "cargo"):
                shutil.rmtree(os.path.join(jobdir, d), ignore_errors=True)
            record["pruned"] = True
        self.save()
        return status

    def container_path(self, jobdir, host):
        rel = os.path.relpath(host, os.path.join(jobdir, "workdir"))
        return f"{WORK}/{rel}"


def resource_flags(profile):
    """A runner's size as the job sees it: its cores pinned (so nproc and available_parallelism read the runner's
    count, not this machine's), its memory a hard limit with no swap beyond it."""
    if not profile:
        return []
    cpus, gb = profile
    total = os.cpu_count()
    first = max(total - cpus, 0)
    return ["--cpuset-cpus", f"{first}-{total - 1}", "--memory", f"{gb}g", "--memory-swap", f"{gb}g"]


def parse_file_commands(path):
    out = {}
    with open(path) as f:
        lines = f.read().split("\n")
    i = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        if not line:
            continue
        if "<<" in line and ("=" not in line or line.index("<<") < line.index("=")):
            k, delim = line.split("<<", 1)
            buf = []
            while i < len(lines) and lines[i] != delim:
                buf.append(lines[i])
                i += 1
            i += 1
            out[k] = "\n".join(buf)
        else:
            k, _, v = line.partition("=")
            out[k] = v
    return out


def du_gb(path):
    r = subprocess.run(["du", "-sx", "--block-size=1M", path], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                       text=True)
    try:
        return int(r.stdout.split()[0]) / 1024
    except (IndexError, ValueError):
        return 0.0


def fmt_secs(s):
    s = int(s)
    return f"{s // 3600}h{s % 3600 // 60:02d}m" if s >= 3600 else f"{s // 60}m{s % 60:02d}s"


# ───────────────────────────── the actions, as this machine does them ─────────────────────────────

def checkout(run, jobdir, uses, with_, env, workdir, logfile, timeout):
    """The run's commit, or the one `ref` names in the source repository (a tag or a branch checks out detached, as
    actions/checkout leaves it), one commit deep."""
    unknown = set(with_) - {"fetch-depth", "ref"}
    if unknown or str(with_.get("fetch-depth", 1)) != "1":
        raise Refusal(f"{uses}: inputs {sorted(with_)} are not supported locally")
    ws = run.host_path(jobdir, run.workspace)
    shutil.rmtree(ws, ignore_errors=True)
    os.makedirs(ws)
    branch = run.args.ref_name.split("/")[-1]
    ref = stringify(with_.get("ref") or "")
    with open(logfile, "a") as log:
        sha = run.sha
        if ref:
            found = subprocess.run(["git", "-C", run.source, "rev-parse", "--verify", "-q", f"{ref}^{{commit}}"],
                                   text=True, stdout=subprocess.PIPE, stderr=log)
            if found.returncode != 0:
                log.write(f"the source repository has no ref `{ref}`\n")
                return False, False
            sha = found.stdout.strip()
        place = ["checkout", "-q", "--force", "--detach", sha] if ref else \
            ["checkout", "-q", "--force", "-B", branch, f"refs/remotes/origin/{branch}"]
        for args in (["init", "-q"], ["remote", "add", "origin", run.source],
                     ["fetch", "-q", "--no-tags", "--depth=1", "origin", f"+{sha}:refs/remotes/origin/{branch}"],
                     place):
            if subprocess.run(["git", "-C", ws, *args], stdout=log, stderr=subprocess.STDOUT).returncode != 0:
                return False, False
        log.write(f"checked out {sha}" + (f" ({ref}, detached)\n" if ref else f" as {branch}\n"))
    return True, False


def rust_toolchain(run, jobdir, uses, with_, env, workdir, logfile, timeout):
    ref = uses.split("@", 1)[1]
    toolchain = with_.get("toolchain") or (ref if ref != "master" else None)
    if not toolchain:
        raise Refusal(f"{uses}: no toolchain given")
    unknown = set(with_) - {"toolchain", "targets", "components"}
    if unknown:
        raise Refusal(f"{uses}: inputs {sorted(unknown)} are not supported locally")
    targets = " ".join(f"--target {t}" for t in re.split(r"[,\s]+", stringify(with_.get("targets", ""))) if t)
    components = " ".join(f"--component {c}" for c in re.split(r"[,\s]+", stringify(with_.get("components", ""))) if c)
    script = f"""
rustup toolchain install {shlex.quote(str(toolchain))} {targets} {components} --profile minimal --no-self-update --force-non-host
rustup default {shlex.quote(str(toolchain))} --force-non-host
echo CARGO_HOME={HOME}/.cargo >> "$GITHUB_ENV"
echo {HOME}/.cargo/bin >> "$GITHUB_PATH"
if [ -z "${{CARGO_INCREMENTAL+set}}" ]; then echo CARGO_INCREMENTAL=0 >> "$GITHUB_ENV"; fi
if [ -z "${{CARGO_TERM_COLOR+set}}" ]; then echo CARGO_TERM_COLOR=always >> "$GITHUB_ENV"; fi
rustc +{shlex.quote(str(toolchain))} --version --verbose
"""
    return run_inline(run, jobdir, script, env, workdir, logfile, timeout)


def install_nextest(run, jobdir, uses, with_, env, workdir, logfile, timeout):
    if with_:
        raise Refusal(f"{uses}: inputs {sorted(with_)} are not supported locally")
    host = shutil.which("cargo-nextest")
    if not host:
        raise SystemExit("run-ci-locally: install-action@nextest needs cargo-nextest on this machine")
    shutil.copy2(host, os.path.join(jobdir, "cargo", "bin", "cargo-nextest"))
    version = subprocess.run([host, "nextest", "--version"], text=True, stdout=subprocess.PIPE).stdout.splitlines()[0]
    with open(logfile, "a") as f:
        f.write(f"installed this machine's {version} (the hosted job installs the newest release)\n")
    return True, False


def rust_cache(run, jobdir, uses, with_, env, workdir, logfile, timeout):
    with open(logfile, "a") as f:
        f.write("no cache restored: target/ starts cold, the cargo registry is shared across local jobs\n")
    with open(run.host_path(jobdir, env["GITHUB_ENV"]), "a") as f:
        f.write("CARGO_INCREMENTAL=0\nCACHE_ON_FAILURE=false\n")
    return True, False


def upload_artifact(run, jobdir, uses, with_, env, workdir, logfile, timeout):
    """Each path (a file, a directory's contents, or a glob's matches) into the run's artifact store under `name`;
    nothing found fails only when `if-no-files-found: error` (v4's default is `warn`)."""
    unknown = set(with_) - {"name", "path", "retention-days", "if-no-files-found"}
    if unknown:
        raise Refusal(f"{uses}: inputs {sorted(unknown)} are not supported locally")
    name = stringify(with_.get("name", "artifact"))
    dest = os.path.join(run.artifacts, name)
    shutil.rmtree(dest, ignore_errors=True)
    os.makedirs(dest)
    found = False
    for p in stringify(with_["path"]).split("\n"):
        p = p.strip()
        if not p:
            continue
        pattern = run.host_path(jobdir, p if os.path.isabs(p) else os.path.join(workdir, p))
        for src in sorted(glob.glob(pattern)) if glob.has_magic(pattern) else [pattern]:
            if os.path.isdir(src):
                shutil.copytree(src, dest, dirs_exist_ok=True)
                found = True
            elif os.path.exists(src):
                os.link(src, os.path.join(dest, os.path.basename(src))) if same_fs(src, dest) else \
                    shutil.copy2(src, dest)
                found = True
    with open(logfile, "a") as f:
        f.write(f"artifact {name}: {sorted(os.listdir(dest))}\n")
    return found or stringify(with_.get("if-no-files-found", "warn")) in ("ignore", "warn"), False


def download_artifact(run, jobdir, uses, with_, env, workdir, logfile, timeout):
    unknown = set(with_) - {"name", "path"}
    if unknown:
        raise Refusal(f"{uses}: inputs {sorted(unknown)} are not supported locally")
    name = stringify(with_["name"])
    src = os.path.join(run.artifacts, name)
    if not os.path.isdir(src):
        with open(logfile, "a") as f:
            f.write(f"no artifact named {name} in this run\n")
        return False, False
    dest = run.host_path(jobdir, stringify(with_.get("path", workdir)))
    os.makedirs(dest, exist_ok=True)
    for entry in os.listdir(src):
        s, d = os.path.join(src, entry), os.path.join(dest, entry)
        if os.path.isdir(s):
            shutil.copytree(s, d, dirs_exist_ok=True)
        elif same_fs(s, dest):
            if os.path.exists(d):
                os.remove(d)
            os.link(s, d)
        else:
            shutil.copy2(s, d)
    with open(logfile, "a") as f:
        f.write(f"artifact {name} → {stringify(with_.get('path', workdir))}\n")
    return True, False


def cache(run, jobdir, uses, with_, env, workdir, logfile, timeout):
    """No cache is restored (the job starts cold, as a cache miss does on GitHub) and none is saved."""
    unknown = set(with_) - {"path", "key", "restore-keys", "enableCrossOsArchive", "upload-chunk-size"}
    if unknown:
        raise Refusal(f"{uses}: inputs {sorted(unknown)} are not supported locally")
    with open(logfile, "a") as f:
        f.write(f"cache {stringify(with_.get('key'))}: not restored (a cold start, as a miss is) and not saved\n")
    with open(run.host_path(jobdir, env["GITHUB_OUTPUT"]), "a") as f:
        f.write("cache-hit=false\n")
    return True, False


def setup_node(run, jobdir, uses, with_, env, workdir, logfile, timeout):
    unknown = set(with_) - {"node-version", "cache", "cache-dependency-path"}
    if unknown:
        raise Refusal(f"{uses}: inputs {sorted(unknown)} are not supported locally")
    want = int(str(with_.get("node-version", "")).split(".")[0])
    have = node_major()
    with open(logfile, "a") as f:
        f.write(f"node {have} from this machine, mounted at /usr/local/bin/node; the npm cache is not restored\n")
    if have != want:
        raise Refusal(f"{uses}: the job wants node {want}, this machine has node {have}")
    return True, False


def run_inline(run, jobdir, script, env, workdir, logfile, timeout):
    host_script = run.host_path(jobdir, f"{TEMP}/{uuid.uuid4()}.sh")
    with open(host_script, "w") as f:
        f.write(script)
    argv = [a.replace("{0}", run.container_path(jobdir, host_script)) for a in SHELLS["bash"]]
    with open(logfile, "a") as f:
        f.write(f"$ {' '.join(argv[:-1])} <<'SCRIPT'\n{script}\nSCRIPT\n")
    return run.exec_script(argv, env, workdir, logfile, timeout)


def same_fs(a, b):
    return os.stat(a).st_dev == os.stat(b if os.path.isdir(b) else os.path.dirname(b)).st_dev


ACTIONS = {
    "actions/checkout@v4": checkout,
    "dtolnay/rust-toolchain@stable": rust_toolchain,
    "dtolnay/rust-toolchain@master": rust_toolchain,
    "taiki-e/install-action@nextest": install_nextest,
    "Swatinem/rust-cache@v2": rust_cache,
    "actions/upload-artifact@v4": upload_artifact,
    "actions/download-artifact@v4": download_artifact,
    "actions/setup-node@v4": setup_node,
    "actions/cache@v4": cache,
}


# ───────────────────────────── commands ─────────────────────────────

def interpolate_everywhere(wf, event, ref_name, inputs):
    """Every string the workflow holds, interpolated against what is known before a run (the rest reads as empty), so
    an expression the runner would fail to read or evaluate fails here."""
    def walk(where, value, ctx):
        if isinstance(value, dict):
            for k, v in value.items():
                walk(where, v, ctx)
        elif isinstance(value, list):
            for v in value:
                walk(where, v, ctx)
        elif isinstance(value, str) and "${{" in value:
            try:
                interpolate(value, ctx)
            except Refusal as r:
                raise Refusal(f"{where}: {r}")
            except Exception as e:
                raise Refusal(f"{where}: `{value}` does not evaluate ({type(e).__name__}: {e})")
    try:
        plan_ctx(wf, event, ref_name, inputs, {}, None)
    except Refusal as r:
        raise Refusal(f"workflow env: {r}")
    for name, job in (wf.get("jobs") or {}).items():
        for leg in matrix_legs(job):
            label = as_list(interpolate(job.get("runs-on"), plan_ctx(wf, event, ref_name, inputs, leg, None)))[0]
            ctx = plan_ctx(wf, event, ref_name, inputs, leg, label)
            walk(f"job `{name}`", {k: v for k, v in job.items() if k not in ("steps", "if")}, ctx)
            for i, step in enumerate(job.get("steps") or []):
                walk(f"job `{name}` step {i + 1}", {k: v for k, v in step.items() if k != "if"}, ctx)


def given_inputs(args):
    out = {}
    for item in args.input or []:
        k, sep, v = item.partition("=")
        if not sep:
            raise SystemExit(f"run-ci-locally: --input takes NAME=VALUE, not `{item}`")
        out[k] = v
    return out


def cmd_plan(args):
    wf = load_workflow(args.workflow)
    order = check_workflow(wf)
    events = args.event_list or triggers(wf)
    for ev in events:
        if ev not in triggers(wf):
            raise Refusal(f"the workflow is not triggered by `{ev}`")
        inputs = dispatch_inputs(wf, ev, given_inputs(args))
        interpolate_everywhere(wf, ev, args.ref_name, inputs)
        print(f"{ev}:")
        verdicts = local_verdicts(wf, order, ev, args.ref_name, inputs)
        for name in order:
            for leg, label, verdict in verdicts[name]:
                print(f"  {job_key(name, leg):<16} {stringify(label):<16} {verdict}")
    return 0


def cmd_eval(args):
    ctx = {"github": {}, "env": {}, "matrix": {}, "needs": {}, "steps": {}, "job": {"status": "success"},
           "runner": runner_ctx(None), "strategy": {}, "inputs": {},
           "__workspace__": os.path.abspath(args.workspace) if args.workspace else None}
    print(stringify(evaluate(Parser(args.expression).parse(), ctx)))
    return 0


def needs_runtime(text):
    return bool(re.search(r"\b(needs|steps)\.", str(text)))


def cmd_run(args):
    if args.tree and args.source:
        raise SystemExit("run-ci-locally: give --tree or --source, not both")
    run_id = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S")
    run_dir = os.path.join(STATE, "runs", run_id)
    os.makedirs(run_dir)
    os.makedirs(os.path.join(LOG_ROOT, run_id))
    latest = os.path.join(LOG_ROOT, "latest")
    if os.path.islink(latest):
        os.remove(latest)
    os.symlink(run_id, latest)
    if args.tree:
        source = os.path.join(run_dir, "source")
        sha = snapshot_tree(os.path.abspath(args.tree), source)
    else:
        source = os.path.abspath(args.source or REPO_ROOT)
        sha = git("-C", source, "rev-parse", f"{args.ref}^{{commit}}", capture=True)
    wf_path = os.path.join(run_dir, "workflow.yml")
    with open(wf_path, "w") as f:
        f.write(subprocess.run(["git", "-C", source, "show", f"{sha}:{args.workflow}"], check=True, text=True,
                               stdout=subprocess.PIPE).stdout)
    wf = load_workflow(wf_path)
    order = check_workflow(wf)
    if args.event not in triggers(wf):
        raise Refusal(f"the workflow is not triggered by `{args.event}`")
    inputs = dispatch_inputs(wf, args.event, given_inputs(args))
    interpolate_everywhere(wf, args.event, args.ref_name, inputs)
    verdicts = local_verdicts(wf, order, args.event, args.ref_name, inputs)
    run = Run(args, wf, source, sha, run_id)
    run.toolchain, version = toolchain_store()
    run.state["rust"] = version
    run.log(f"run {run_id}: {args.workflow} at {sha[:12]} ({args.tree or source}), event {args.event}, "
            f"Rust stable {version}, state {run_dir}"
            + (f", timeouts x{args.timeout_scale:g}" if args.timeout_scale != 1 else ""))

    previous = {}
    if args.from_run:
        with open(os.path.join(STATE, "runs", args.from_run, "run.json")) as f:
            previous = json.load(f)
        prev_art = os.path.join(STATE, "runs", args.from_run, "artifacts")
        if os.path.isdir(prev_art):
            shutil.copytree(prev_art, run.artifacts, copy_function=os.link, dirs_exist_ok=True)

    selected = set(args.job or order)
    for j in selected:
        if j not in wf["jobs"]:
            raise SystemExit(f"run-ci-locally: no job `{j}` in {args.workflow}")
    if not args.from_run:
        stack = list(selected)
        while stack:
            for d in as_list(wf["jobs"][stack.pop()].get("needs")):
                if d not in selected:
                    selected.add(d)
                    stack.append(d)
    leg_filter = dict(m.split("=", 1) for m in args.matrix or [])

    def chosen(leg):
        return all(stringify(leg.get(k)) == v for k, v in leg_filter.items() if k in leg)

    for image in sorted({RUNNERS[label][0] for name in selected for leg, label, verdict in verdicts[name]
                         if verdict == "runs" and chosen(leg)}):
        ensure_image(image)
    run.state["not_run"] = []
    results = {}
    for name in order:
        if name not in selected:
            prev = [r for r in previous.get("jobs", {}).values() if r.get("job", r["key"].split(" (")[0]) == name]
            if prev:
                results[name] = {"result": "success" if all(r.get("result") == "success" for r in prev) else "failure",
                                 "outputs": {k: v for r in prev for k, v in r.get("outputs", {}).items()}}
            continue
        job = wf["jobs"][name]
        needs_ctx = {d: results.get(d, {"result": "skipped", "outputs": {}}) for d in as_list(job.get("needs"))}
        outcome = []
        for leg, label, verdict in verdicts[name]:
            if not chosen(leg):
                continue
            if verdict.startswith("not run locally"):
                run.log(f"── job {job_key(name, leg)}: {verdict}")
                run.state["not_run"].append({"key": job_key(name, leg), "why": verdict})
                outcome.append("not run")
                continue
            ctx = dict(plan_ctx(wf, args.event, args.ref_name, inputs, leg, label), needs=needs_ctx)
            # What success() and failure() see: every needed job passed, one failed, or one was skipped (which
            # skips this job too unless its condition asks for always()).
            needed = [n["result"] for n in needs_ctx.values()]
            ctx["job"] = {"status": "failure" if "failure" in needed else "skipped" if "skipped" in needed
                          else "success"}
            cond = condition(job.get("if", "success()"))
            if not truthy(evaluate(cond, ctx)):
                run.log(f"── job {job_key(name, leg)}: skipped")
                outcome.append("skipped")
                continue
            outcome.append(run.run_job(name, job, leg, needs_ctx))
        merged = {}
        for r in run.state["jobs"].values():
            if r["job"] == name:
                merged.update(r.get("outputs", {}))
        results[name] = {"result": "failure" if "failure" in outcome else "success" if "success" in outcome
                         else "skipped", "outputs": merged}
    run.state["results"] = {n: r["result"] for n, r in results.items()}
    run.save()
    run.log("")
    run.log(f"run {run_id} summary:")
    unconstrained = []
    for r in run.state["jobs"].values():
        steps_failed = [s["name"] for s in r["steps"] if s["result"] in ("failure", "timed out")]
        margin = r["timeout_minutes"] * 60 - r.get("seconds", 0)
        run.log(f"  {r.get('result', '?'):<8} {r['key']:<16} {fmt_secs(r.get('seconds', 0)):>7} of "
                f"{r['timeout_minutes']:g} min, {r.get('disk_gb', 0):6.1f} GB"
                + (f"   failed: {'; '.join(steps_failed)}" if steps_failed else "")
                + ("   (under 20% of the timeout left)" if margin < 0.2 * r["timeout_minutes"] * 60 else ""))
        if r["profile"] is None:
            unconstrained.append(f"{r['key']} ({r['label']})")
    if unconstrained:
        run.log(f"  unconstrained, timing not representative: {', '.join(unconstrained)} — give --profile LABEL=CPUSxGB")
    if args.timeout_scale != 1:
        run.log(f"  every timeout scaled x{args.timeout_scale:g}: a job that passed here may still time out on GitHub")
    failed = [n for n, r in results.items() if r["result"] == "failure"]
    elsewhere = [n for n in run.state["not_run"] if "write permission" not in n["why"]]
    for n in run.state["not_run"]:
        run.log(f"  not run  {n['key']}: {n['why'].removeprefix('not run locally: ')}")
    if failed:
        run.log(f"RESULT: RED — {', '.join(failed)}")
    elif elsewhere:
        run.log(f"RESULT: GREEN FOR THE LEGS RUN HERE — {len(elsewhere)} legs need what this machine is not (listed "
                "above); only a run on GitHub covers them")
    else:
        run.log("RESULT: GREEN")
    return 1 if failed else 0


def cmd_exec(args):
    with open(os.path.join(STATE, "runs", args.run, "run.json")) as f:
        state = json.load(f)
    rec = state["jobs"].get(args.job) or next((r for r in state["jobs"].values() if slug(r["key"]) == args.job), None)
    if not rec:
        raise SystemExit(f"run-ci-locally: no job {args.job} in run {args.run}: {sorted(state['jobs'])}")
    jobdir = rec["dir"]
    if rec.get("pruned"):
        raise SystemExit(f"run-ci-locally: job {rec['key']} passed and its workspace was given back; rerun it with "
                         "--keep-workspaces to exec in it")
    repo = state.get("repository", "Brahmastra-Labs/logicaffeine").split("/")[-1]
    env = dict(rec.get("env", {}))
    cmd = ["docker", "run", "--rm", "-i", "--init", "--user", "runner", "--shm-size", "2g",
           "--ulimit", "nofile=65536:65536", "--security-opt", "seccomp=unconfined", "--cap-add", "SYS_PTRACE",
           "-w", env.get("GITHUB_WORKSPACE", f"{WORK}/{repo}/{repo}"),
           "-v", f"{jobdir}/workdir:{WORK}", "-v", f"{jobdir}/tmp:/tmp",
           "-v", f"{jobdir}/rustup:{HOME}/.rustup", "-v", f"{jobdir}/cargo:{HOME}/.cargo",
           "-v", f"{STATE}/cache/cargo-registry:{HOME}/.cargo/registry",
           "-v", f"{STATE}/cache/cargo-git:{HOME}/.cargo/git",
           "-v", f"{os.path.realpath(shutil.which('node'))}:/usr/local/bin/node:ro",
           "-v", "/usr/lib/node_modules:/usr/local/lib/node_modules:ro",
           "-v", "/opt/google/chrome:/opt/google/chrome:ro"]
    cmd += resource_flags(rec.get("profile"))
    for k, v in env.items():
        if not k.startswith("GITHUB_") or k in ("GITHUB_WORKSPACE", "GITHUB_REPOSITORY", "GITHUB_ACTIONS"):
            cmd += ["-e", f"{k}={v}"]
    cmd += [image_tag(rec.get("image", "ubuntu-24.04")), "bash", "-c", " ".join(shlex.quote(c) for c in args.command)]
    return subprocess.run(cmd).returncode


def cmd_list(args):
    root = os.path.join(STATE, "runs")
    for rid in sorted(os.listdir(root)) if os.path.isdir(root) else []:
        p = os.path.join(root, rid, "run.json")
        if os.path.exists(p):
            with open(p) as f:
                s = json.load(f)
            print(f"{rid}  {s.get('sha', '')[:12]}  {s.get('event')}  "
                  + "  ".join(f"{n}={r}" for n, r in s.get("results", {}).items()))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("plan", "run"):
        p = sub.add_parser(name)
        p.add_argument("--workflow", default=".github/workflows/test.yml")
        p.add_argument("--ref-name", default="refs/heads/main", help="github.ref (default refs/heads/main)")
        p.add_argument("--input", action="append", help="NAME=VALUE, a workflow_dispatch input")
        if name == "plan":
            p.add_argument("--event", dest="event_list", action="append", help="default: every trigger")
        else:
            p.add_argument("--event", default="push")
            p.add_argument("--tree", help="run the working tree of this git checkout, uncommitted changes included")
            p.add_argument("--source", help="a git repository to check out (default: this one)")
            p.add_argument("--ref", default="HEAD", help="the revision of --source to check out")
            p.add_argument("--repository", default="Brahmastra-Labs/logicaffeine")
            p.add_argument("--job", action="append", help="run this job (and what it needs, unless --from-run)")
            p.add_argument("--matrix", action="append", help="only the matrix legs with this KEY=VALUE")
            p.add_argument("--from-run", help="take the outputs and artifacts of jobs not run here from this run")
            p.add_argument("--profile", action="append", help="LABEL=CPUSxGB, the size of a runner label")
            p.add_argument("--keep", action="store_true", help="leave each job's container for inspection")
            p.add_argument("--timeout-scale", type=float, default=1.0,
                           help="multiply every timeout-minutes, for a machine slower or busier than the runner")
            p.add_argument("--keep-workspaces", action="store_true",
                           help="keep a passing job's workspace too (a failing job's is always kept, for exec)")
    p = sub.add_parser("exec")
    p.add_argument("--run", required=True)
    p.add_argument("--job", required=True)
    p.add_argument("command", nargs=argparse.REMAINDER)
    sub.add_parser("list")
    p = sub.add_parser("eval")
    p.add_argument("--workspace", help="the directory hashFiles() reads")
    p.add_argument("expression")
    args = ap.parse_args()
    if args.cmd == "exec" and args.command[:1] == ["--"]:
        args.command = args.command[1:]

    def stopped(signum, _frame):
        # A job's container belongs to dockerd, not to this process: unwinding runs each job's cleanup, which removes it.
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGTERM, stopped)
    signal.signal(signal.SIGHUP, stopped)
    if args.cmd == "plan" and not os.path.isabs(args.workflow) and not os.path.exists(args.workflow):
        args.workflow = os.path.join(REPO_ROOT, args.workflow)
    try:
        return {"plan": cmd_plan, "run": cmd_run, "exec": cmd_exec, "list": cmd_list, "eval": cmd_eval}[args.cmd](args)
    except Refusal as r:
        print(f"run-ci-locally: {r}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
