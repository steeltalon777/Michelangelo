"""The uncertainty policy of an input package for the Stage B solver (IT-8C).

The solver reads the policy of the manifest and never weakens it.  The
precedence of the contract `docs/specs/VERIFICATION_STATE.md` section 8 is:

1. the author decisions that are accepted inside the package,
2. the `uncertainty_policy` of the package,
3. the policy of the workspace,
4. the flags of the candidate or of the task.

The solver only sees the package, so it enforces the levels 1 and 2.  A
package decision never weakens a higher level; the security constraints
(file and network access, budgets, the protected scope, the sandbox and the
secrets) are independent and are never touched by this module.

The module is standard library only and never writes into a package.
"""

from __future__ import annotations

#: The default of `require_author_acceptance` inside an
#: `uncertainty_policy` object.
DEFAULT_REQUIRE_AUTHOR_ACCEPTANCE = True

#: The prefix of the acceptance target of a supplement.
SUPPLEMENT_TARGET_PREFIX = "supplements:"
#: The prefix of the acceptance target of an unknown.
UNKNOWN_TARGET_PREFIX = "unknowns:"
#: The prefix of the acceptance target of a part.
PART_TARGET_PREFIX = "parts:"
#: The prefix of the acceptance target of a measurement.
MEASUREMENT_TARGET_PREFIX = "measurements:"


def _text(value):
    """The trimmed text of a value, or None when it is not a string."""
    if isinstance(value, str):
        text = value.strip()
        return text or None
    return None


class PackagePolicy:
    """The uncertainty policy of a package manifest.

    A manifest that carries no `uncertainty_policy` object keeps the
    behaviour of the stage B solver: every supplement is applied and a stub
    is allowed.  An object that omits `require_author_acceptance` uses the
    default of the schema (`true`).
    """

    def __init__(self, manifest):
        raw = manifest.get("uncertainty_policy")
        self.present = isinstance(raw, dict)
        self.require_author_acceptance = DEFAULT_REQUIRE_AUTHOR_ACCEPTANCE
        self.allowed_completion_rules = []
        self.stub_policy = "allowed"
        if self.present:
            value = raw.get("require_author_acceptance")
            if isinstance(value, bool):
                self.require_author_acceptance = value
            rules = raw.get("allowed_completion_rules")
            if isinstance(rules, list):
                self.allowed_completion_rules = [
                    text.lower()
                    for text in (_text(rule) for rule in rules)
                    if text is not None
                ]
            stub = _text(raw.get("stub_policy"))
            if stub is not None:
                self.stub_policy = stub.lower()
        self.accepted_targets = set()
        entries = manifest.get("acceptance")
        if isinstance(entries, list):
            for entry in entries:
                if not isinstance(entry, dict):
                    continue
                status = _text(entry.get("status"))
                if status is None or status.lower() != "accepted":
                    continue
                target = _text(entry.get("target"))
                if target is not None:
                    self.accepted_targets.add(target)

    # -- accepted author decisions ------------------------------------------
    def accepts_target(self, target):
        """Whether the package carries an accepted decision for a target."""
        return str(target).strip() in self.accepted_targets

    def accepts_supplement(self, supplement_id):
        """Whether the package accepts the target of a supplement."""
        return self.accepts_target(SUPPLEMENT_TARGET_PREFIX + str(supplement_id))

    def accepts_unknown(self, unknown_id):
        """Whether the package accepts the target of an unknown."""
        return self.accepts_target(UNKNOWN_TARGET_PREFIX + str(unknown_id))

    # -- the policy itself ---------------------------------------------------
    def rule_is_allowed(self, kind):
        """Whether a completion rule may be applied.

        An empty list of allowed rules does not restrict the applied kinds.
        """
        if not self.allowed_completion_rules:
            return True
        return str(kind).strip().lower() in self.allowed_completion_rules

    def stubs_allowed(self):
        """Whether the package allows a geometric stub."""
        return self.stub_policy != "forbidden"

    def author_acceptance_required(self):
        """Whether an AI-authored supplement needs an accepted decision.

        A package that carries no `uncertainty_policy` object expresses no
        requirement: the stage B behaviour is kept.
        """
        return self.present and self.require_author_acceptance

    def summary(self):
        """A deterministic, human readable summary of the policy."""
        return (
            "require_author_acceptance: %s; allowed_completion_rules: %s; "
            "stub_policy: %s"
            % (
                "required" if self.author_acceptance_required() else "not required",
                ", ".join(self.allowed_completion_rules) or "-",
                self.stub_policy,
            )
        )


def supplement_origin(supplement):
    """The origin of a supplement (`human`, `ai_author` or `computed`)."""
    provenance = supplement.get("provenance")
    if not isinstance(provenance, dict):
        return "human"
    origin = _text(provenance.get("origin"))
    return origin.lower() if origin is not None else "human"


def supplement_is_ai_authored(supplement):
    """Whether a supplement was authored by an AI system."""
    return supplement_origin(supplement) == "ai_author"
