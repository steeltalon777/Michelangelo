#!/bin/bash
# Helper: create a fake Blender executable for smoke testing.
# Usage: scripts/create_fake_blender.sh <output_path>
#
# The fake reads the job.json passed by the adapter (job id + workspace root),
# echoes the requested job id, creates the artifact it reports under the job's
# workspace root, and reports the real file size. This is required by the
# adapter identity/artifact validation contract (TZ 5.1 / 5.7): a fake that
# hardcodes a foreign job id or reports a non-existent artifact is rejected.
set -euo pipefail

out="$1"
mkdir -p "$(dirname "$out")"
cat > "$out" <<'ENDOFSCRIPT'
#!/bin/bash
job_json="${@: -2:1}"
result_json="${@: -1}"
echo "Fake Blender: running" 1>&2
if [ -z "$job_json" ] || [ -z "$result_json" ]; then
    echo "Fake Blender: missing job/result arguments" 1>&2
    exit 2
fi
job_id=$(sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p' "$job_json" | head -n 1)
workspace_root=$(sed -n 's/.*"workspace_root":"\([^"]*\)".*/\1/p' "$job_json" | head -n 1)
if [ -z "$job_id" ] || [ -z "$workspace_root" ]; then
    echo "Fake Blender: cannot parse job.json" 1>&2
    exit 3
fi
mkdir -p "$workspace_root/blender/scenes"
printf 'FAKE_BLEND_ARTIFACT_DATA' > "$workspace_root/blender/scenes/smoke.blend"
size=$(wc -c < "$workspace_root/blender/scenes/smoke.blend" | tr -d '[:space:]')
cat > "$result_json" <<RESULT
{"job_id":"$job_id","status":"completed","message":"Fake Blender success","artifacts":[{"path":"blender/scenes/smoke.blend","type":"blend","size_bytes":$size}],"scene_summary":{"objects":1,"meshes":1,"triangles":12,"vertices":24}}
RESULT
exit 0
ENDOFSCRIPT

chmod 755 "$out"
