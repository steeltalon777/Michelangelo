"""Bundle writer: asset_digest по правилу 4 README схем GLM-1, статус proposed."""

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from helpers import CANDIDATE_BUNDLE_SCHEMA

from runner import bundle_writer, errors


def make_artifact(artifact_id="variant_glb", path="outputs/v0/variant.glb",
                  kind="scene_layout", sha="b" * 64, size=100):
    return {
        "artifact_id": artifact_id,
        "path": path,
        "sha256": sha,
        "size_bytes": size,
        "content_kind": kind,
        "source": "generated",
    }


class TestAssetDigest(unittest.TestCase):
    def test_asset_digest_matches_readme_rule_4(self):
        """SHA-256 над '<id>\\0<path>\\0<kind>\\0<sha>\\n' по (artifact_id, path)."""
        artifacts = [
            make_artifact(artifact_id="variant_glb", path="outputs/a.glb", sha="a" * 64),
            make_artifact(artifact_id="variant_blend", path="outputs/a.blend",
                          kind="other", sha="c" * 64),
        ]
        expected_lines = sorted(
            [
                f"variant_blend\u0000outputs/a.blend\u0000other\u0000{'c' * 64}\n",
                f"variant_glb\u0000outputs/a.glb\u0000scene_layout\u0000{'a' * 64}\n",
            ]
        )
        expected = hashlib.sha256("".join(expected_lines).encode("utf-8")).hexdigest()
        self.assertEqual(expected, bundle_writer.compute_asset_digest(artifacts))

    def test_status_is_constant_proposed(self):
        bundle = bundle_writer.build_bundle(
            candidate_id="cand-v0-brick-dark",
            asset_id="v0-house-master-palette",
            item_id="item-v0-brick-dark",
            artifacts=[make_artifact()],
            dependency_digests={"source_house_master": "a" * 64},
            base_ref={"kind": "source_snapshot", "id": "house_master", "sha256": "d" * 64},
            provenance_chain=[
                {
                    "stage": "variant_generation",
                    "ref": {"path": "docs/adr5-streams/blender/TEST_EVIDENCE.md",
                            "sha256": "e" * 64},
                }
            ],
            unresolved_issues=["no UE-ready claim"],
        )
        self.assertEqual("proposed", bundle["status"])
        self.assertEqual(
            "michelangelo.candidate-asset-bundle/draft-0.1", bundle["schema"]
        )
        self.assertEqual(64, len(bundle["asset_digest"]))

    def test_unknown_content_kind_refused(self):
        with self.assertRaises(errors.PoCError) as ctx:
            bundle_writer.build_bundle(
                candidate_id="cand-x",
                asset_id="asset-x",
                item_id="item-x",
                artifacts=[make_artifact(kind="ue_ready_magic")],
                dependency_digests={"dep": "a" * 64},
                base_ref=None,
                provenance_chain=[{"stage": "variant_generation",
                                   "ref": {"path": "x.md", "sha256": "e" * 64}}],
                unresolved_issues=[],
            )
        self.assertEqual(errors.INVALID_PARAMETER, ctx.exception.code)

    def test_path_escape_in_artifact_refused(self):
        with self.assertRaises(errors.PoCError) as ctx:
            bundle_writer.build_bundle(
                candidate_id="cand-x",
                asset_id="asset-x",
                item_id="item-x",
                artifacts=[make_artifact(path="../outside.glb")],
                dependency_digests={"dep": "a" * 64},
                base_ref=None,
                provenance_chain=[{"stage": "variant_generation",
                                   "ref": {"path": "x.md", "sha256": "e" * 64}}],
                unresolved_issues=[],
            )
        self.assertEqual(errors.INVALID_OUTPUT_PATH, ctx.exception.code)

    def test_write_bundle_sorted_lf_utf8(self):
        bundle = bundle_writer.build_bundle(
            candidate_id="cand-x",
            asset_id="asset-x",
            item_id="item-x",
            artifacts=[make_artifact()],
            dependency_digests={"dep": "a" * 64},
            base_ref=None,
            provenance_chain=[{"stage": "variant_generation",
                               "ref": {"path": "x.md", "sha256": "e" * 64}}],
            unresolved_issues=[],
        )
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "bundle.json"
            sha = bundle_writer.write_bundle(bundle, out)
            data = out.read_bytes()
            self.assertEqual(sha, hashlib.sha256(data).hexdigest())
            self.assertNotIn(b"\r\n", data)
            self.assertFalse(data.startswith(b"\xef\xbb\xbf"))
            json.loads(data.decode("utf-8"))

    @unittest.skipUnless(
        CANDIDATE_BUNDLE_SCHEMA.is_file(),
        "GLM-1 contracts worktree is not available (read-only reference)",
    )
    def test_bundle_validates_against_glm1_schema(self):
        from runner.bundle_writer import validate_bundle_against_schema

        bundle = bundle_writer.build_bundle(
            candidate_id="cand-v0-brick-dark",
            asset_id="v0-house-master-palette",
            item_id="item-v0-brick-dark",
            artifacts=[
                make_artifact(
                    artifact_id="variant_glb",
                    path="outputs/v0_brick_dark/variant.glb",
                    kind="scene_layout",
                )
            ],
            dependency_digests={
                "source_house_master": "a" * 64,
                "template_v0_palette": "b" * 64,
            },
            base_ref={"kind": "source_snapshot", "id": "house_master", "sha256": "c" * 64},
            provenance_chain=[
                {
                    "stage": "variant_generation",
                    "ref": {
                        "path": "docs/adr5-streams/blender/TEST_EVIDENCE.md",
                        "sha256": "d" * 64,
                    },
                }
            ],
            unresolved_issues=["appearance checked via glTF baseColorFactor only"],
        )
        errors_list = validate_bundle_against_schema(bundle, CANDIDATE_BUNDLE_SCHEMA)
        self.assertEqual([], errors_list, msg=json.dumps(errors_list, indent=2))


if __name__ == "__main__":
    unittest.main()
