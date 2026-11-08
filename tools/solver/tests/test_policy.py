"""Policy tests of the Stage B solver (IT-8C, standard library only).

The tests cover the enforcement of the uncertainty policy of an input
package: an accepted AI supplement is applied, an unaccepted one is not
applied and reports a diagnostic, a forbidden stub is never produced, the
allowed completion rules limit the applied kinds and the build of a
synthetic package works without Blender.

Run them with:

    python -m pytest tools/solver/tests/test_policy.py
    python -m unittest discover -s tools/solver/tests -p "test_policy.py"
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import struct
import sys
import tempfile
import unittest
import zlib

_HERE = os.path.dirname(os.path.abspath(__file__))
_SOLVER = os.path.dirname(_HERE)
if _SOLVER not in sys.path:
    sys.path.insert(0, _SOLVER)

import policy as policymod  # noqa: E402
import solve as solvemod  # noqa: E402
import supplements as supp  # noqa: E402

WIDTH = 64
HEIGHT = 64


def write_gray_png(path, width, height, pixels):
    """Write a minimal 8 bit greyscale PNG (stdlib only)."""
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter: none
        raw.extend(bytes(pixels[y * width:(y + 1) * width]))

    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)
    with open(path, "wb") as handle:
        handle.write(b"\x89PNG\r\n\x1a\n")
        handle.write(chunk(b"IHDR", header))
        handle.write(chunk(b"IDAT", zlib.compress(bytes(raw))))
        handle.write(chunk(b"IEND", b""))


def mask_pixels():
    """A centred square mask: the silhouette of a box."""
    pixels = bytearray(WIDTH * HEIGHT)
    for y in range(16, 48):
        for x in range(16, 48):
            pixels[y * WIDTH + x] = 255
    return pixels


def view_entry(view_id, location, path, sha256):
    """A calibrated orthographic view of the synthetic package."""
    return {
        "view_id": view_id,
        "kind": "calibrated",
        "image": {
            "path": path,
            "sha256": sha256,
            "width_px": WIDTH,
            "height_px": HEIGHT,
        },
        "mask": {
            "path": path,
            "sha256": sha256,
            "format": "grayscale8",
            "foreground": "white",
        },
        "projection": {
            "type": "orthographic",
            "location_m": list(location),
            "look_at_m": [0.0, 0.0, 0.0],
            "up_world": [0.0, 0.0, 1.0],
            "ortho_scale_m": 2.0,
        },
    }


def build_package(directory, uncertainty_policy=None, acceptance=None,
                  supplements=None, unknowns=None):
    """Write a synthetic, valid input package into a directory."""
    images = os.path.join(directory, "images")
    os.makedirs(images, exist_ok=True)
    pixels = mask_pixels()
    views = []
    for view_id, location in (
        ("front", [0.0, -4.0, 0.0]),
        ("right", [4.0, 0.0, 0.0]),
    ):
        relative = "images/%s.png" % view_id
        absolute = os.path.join(directory, relative.replace("/", os.sep))
        write_gray_png(absolute, WIDTH, HEIGHT, pixels)
        with open(absolute, "rb") as handle:
            digest = hashlib.sha256(handle.read()).hexdigest()
        views.append(view_entry(view_id, location, relative, digest))

    manifest = {
        "schema": "michelangelo.input-package/0",
        "package_id": "policy-fixture",
        "object": {"name": "policy fixture"},
        "revision": 1,
        "units": "m",
        "coordinate_system": {
            "handedness": "right",
            "up": "+Z",
            "right": "+X",
            "forward": "-Y",
        },
        "views": views,
        "parts": [{"part_id": "body", "label": "body"}],
        "supplements": supplements if supplements is not None else [],
        "unknowns": unknowns if unknowns is not None else [],
        "measurements": [],
        "provenance": {"origin": "human", "detail": "synthetic fixture"},
        "acceptance": acceptance if acceptance is not None else [],
    }
    if uncertainty_policy is not None:
        manifest["uncertainty_policy"] = uncertainty_policy
    with open(os.path.join(directory, "package.json"), "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=True)
        handle.write("\n")
    return manifest


def ai_supplement(supplement_id="lid_thickness", kind="thickness"):
    """An AI authored supplement of the synthetic package."""
    return {
        "supplement_id": supplement_id,
        "part_id": "body",
        "kind": kind,
        "geometry": {"value_m": 0.02},
        "provenance": {"origin": "ai_author", "detail": "test fixture"},
    }


def rule_unknown(unknown_id="u_body"):
    """An unknown that a rule may close."""
    return {
        "id": unknown_id,
        "description": "the hidden shape of the body",
        "affected_parts": ["body"],
        "policy": "rule_allowed",
    }


def stub_unknown(unknown_id="u_flame_shape"):
    """An unknown that a stub may close."""
    return {
        "id": unknown_id,
        "description": "the hidden shape of the flame",
        "affected_parts": ["body"],
        "policy": "stub_allowed",
    }


def codes(report):
    """The diagnostic codes of a build report."""
    return [entry["code"] for entry in report.get("diagnostics", [])]


class PolicyParsingTest(unittest.TestCase):
    """The policy of a manifest is read without touching the resources."""

    def test_a_manifest_without_a_policy_keeps_the_stage_b_behaviour(self):
        policy = policymod.PackagePolicy({"acceptance": []})
        self.assertFalse(policy.present)
        self.assertFalse(policy.author_acceptance_required())
        self.assertTrue(policy.stubs_allowed())
        self.assertTrue(policy.rule_is_allowed("thickness"))

    def test_the_policy_object_defaults_to_the_author_acceptance(self):
        policy = policymod.PackagePolicy({"uncertainty_policy": {}})
        self.assertTrue(policy.present)
        self.assertTrue(policy.author_acceptance_required())
        self.assertEqual(policy.allowed_completion_rules, [])
        self.assertTrue(policy.stubs_allowed())

    def test_the_policy_reads_its_fields(self):
        policy = policymod.PackagePolicy(
            {
                "uncertainty_policy": {
                    "require_author_acceptance": False,
                    "allowed_completion_rules": ["thickness"],
                    "stub_policy": "forbidden",
                }
            }
        )
        self.assertFalse(policy.author_acceptance_required())
        self.assertEqual(policy.allowed_completion_rules, ["thickness"])
        self.assertFalse(policy.stubs_allowed())
        self.assertTrue(policy.rule_is_allowed("thickness"))
        self.assertFalse(policy.rule_is_allowed("cavity"))

    def test_only_accepted_decisions_count(self):
        policy = policymod.PackagePolicy(
            {
                "acceptance": [
                    {"target": "supplements:lid_thickness", "status": "accepted",
                     "by": "owner", "at": "2026-01-01T00:00:00Z"},
                    {"target": "supplements:other", "status": "proposed",
                     "by": "owner", "at": "2026-01-01T00:00:00Z"},
                ]
            }
        )
        self.assertTrue(policy.accepts_supplement("lid_thickness"))
        self.assertFalse(policy.accepts_supplement("other"))
        self.assertFalse(policy.accepts_unknown("u_body"))


class SupplementPolicyTest(unittest.TestCase):
    """The solver refuses a supplement the policy of the package forbids."""

    def _refusal(self, manifest, supplement):
        policy = policymod.PackagePolicy(manifest)
        return supp._policy_refusal(
            policy, supplement, supplement["supplement_id"], supplement["kind"]
        )

    def test_an_unaccepted_ai_supplement_is_refused(self):
        refusal = self._refusal(
            {"uncertainty_policy": {"require_author_acceptance": True}},
            ai_supplement(),
        )
        self.assertIsNotNone(refusal)
        self.assertEqual(refusal[0], "SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED")

    def test_an_accepted_ai_supplement_is_not_refused(self):
        refusal = self._refusal(
            {
                "uncertainty_policy": {"require_author_acceptance": True},
                "acceptance": [
                    {"target": "supplements:lid_thickness", "status": "accepted",
                     "by": "owner", "at": "2026-01-01T00:00:00Z"}
                ],
            },
            ai_supplement(),
        )
        self.assertIsNone(refusal)

    def test_a_human_supplement_needs_no_decision(self):
        supplement = ai_supplement()
        supplement["provenance"] = {"origin": "human"}
        refusal = self._refusal(
            {"uncertainty_policy": {"require_author_acceptance": True}}, supplement
        )
        self.assertIsNone(refusal)

    def test_a_rule_that_the_package_does_not_allow_is_refused(self):
        refusal = self._refusal(
            {"uncertainty_policy": {"allowed_completion_rules": ["cavity"]}},
            ai_supplement(),
        )
        self.assertIsNotNone(refusal)
        self.assertEqual(refusal[0], "SUPPLEMENT_RULE_NOT_ALLOWED")

    def test_an_accepted_decision_overrides_the_allowed_rules(self):
        # Precedence: an accepted author decision of the package is stronger
        # than the list of the allowed completion rules.
        refusal = self._refusal(
            {
                "uncertainty_policy": {"allowed_completion_rules": ["cavity"]},
                "acceptance": [
                    {"target": "supplements:lid_thickness", "status": "accepted",
                     "by": "owner", "at": "2026-01-01T00:00:00Z"}
                ],
            },
            ai_supplement(),
        )
        self.assertIsNone(refusal)


class UnknownPolicyTest(unittest.TestCase):
    """The solver never closes an unknown the policy does not allow."""

    def test_a_forbidden_stub_is_not_produced(self):
        policy = policymod.PackagePolicy(
            {"uncertainty_policy": {"stub_policy": "forbidden"}}
        )
        stubs, results, diagnostics = supp.resolve_unknowns(
            [stub_unknown()], ((-0.5, -0.5, -0.5), (0.5, 0.5, 0.5)), {},
            package_policy=policy,
        )
        self.assertEqual(stubs, [])
        self.assertEqual(results[0]["status"], "unresolved")
        self.assertIn("STUB_FORBIDDEN", [item["code"] for item in diagnostics])
        self.assertEqual(diagnostics[0]["severity"], "error")

    def test_a_blocked_supplement_does_not_close_the_unknown(self):
        policy = policymod.PackagePolicy(
            {"uncertainty_policy": {"require_author_acceptance": True}}
        )
        blocked = {
            "body": [
                (
                    "lid_thickness",
                    "SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED",
                    "not accepted",
                )
            ]
        }
        stubs, results, diagnostics = supp.resolve_unknowns(
            [rule_unknown()], ((-0.5, -0.5, -0.5), (0.5, 0.5, 0.5)), {}, blocked,
            policy,
        )
        self.assertEqual(stubs, [])
        self.assertEqual(results[0]["status"], "unresolved")
        self.assertIn(
            "SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED",
            [item["code"] for item in diagnostics],
        )

    def test_an_applied_supplement_closes_the_unknown(self):
        policy = policymod.PackagePolicy(
            {"uncertainty_policy": {"require_author_acceptance": True}}
        )
        stubs, results, diagnostics = supp.resolve_unknowns(
            [rule_unknown()], ((-0.5, -0.5, -0.5), (0.5, 0.5, 0.5)),
            {"body": ["lid_thickness"]}, {}, policy,
        )
        self.assertEqual(results[0]["status"], "resolved")
        self.assertEqual(diagnostics, [])


class SolverBuildTest(unittest.TestCase):
    """The build of a synthetic package enforces the policy (no Blender)."""

    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="michelangelo-policy-")
        self.package = os.path.join(self.root, "package")
        self.out = os.path.join(self.root, "out")
        os.makedirs(self.package)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def _solve(self):
        report, code = solvemod.solve_package(
            self.package, self.out, resolution=32, print_only=True
        )
        self.assertEqual(code, 0)
        return report

    def test_an_unaccepted_ai_supplement_is_not_applied(self):
        build_package(
            self.package,
            uncertainty_policy={"require_author_acceptance": True},
            supplements=[ai_supplement()],
            unknowns=[rule_unknown()],
        )
        report = self._solve()
        supplement = report["supplements"][0]
        self.assertEqual(supplement["status"], "not_applied")
        self.assertIn("SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED", codes(report))
        unknown = report["unknowns"][0]
        self.assertEqual(unknown["status"], "unresolved")
        # The supplement produced no component of its own.
        names = [component["name"] for component in report["components"]]
        self.assertNotIn("supplement_lid_thickness", names)

    def test_an_accepted_ai_supplement_is_applied(self):
        build_package(
            self.package,
            uncertainty_policy={"require_author_acceptance": True},
            acceptance=[
                {"target": "supplements:lid_thickness", "status": "accepted",
                 "by": "owner", "at": "2026-01-01T00:00:00Z"}
            ],
            supplements=[ai_supplement()],
            unknowns=[rule_unknown()],
        )
        report = self._solve()
        supplement = report["supplements"][0]
        self.assertEqual(supplement["status"], "applied")
        self.assertNotIn("SUPPLEMENT_NOT_ACCEPTED_UNRESOLVED", codes(report))
        unknown = report["unknowns"][0]
        self.assertEqual(unknown["status"], "resolved")

    def test_a_forbidden_stub_is_reported_and_not_built(self):
        build_package(
            self.package,
            uncertainty_policy={"stub_policy": "forbidden"},
            unknowns=[stub_unknown()],
        )
        report = self._solve()
        self.assertIn("STUB_FORBIDDEN", codes(report))
        unknown = report["unknowns"][0]
        self.assertEqual(unknown["status"], "unresolved")
        names = [component["name"] for component in report["components"]]
        self.assertNotIn("stub_u_flame_shape", names)

    def test_the_allowed_rules_limit_the_applied_kinds(self):
        build_package(
            self.package,
            uncertainty_policy={"allowed_completion_rules": ["cavity"]},
            supplements=[ai_supplement()],
            unknowns=[rule_unknown()],
        )
        report = self._solve()
        supplement = report["supplements"][0]
        self.assertEqual(supplement["status"], "not_applied")
        self.assertIn("SUPPLEMENT_RULE_NOT_ALLOWED", codes(report))
        unknown = report["unknowns"][0]
        self.assertEqual(unknown["status"], "unresolved")

    def test_the_package_policy_is_reported(self):
        build_package(
            self.package,
            uncertainty_policy={"require_author_acceptance": True},
            supplements=[ai_supplement()],
        )
        report = self._solve()
        self.assertIn("PACKAGE_POLICY", codes(report))


if __name__ == "__main__":
    unittest.main()
