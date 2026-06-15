#!/bin/bash
# Helper: create a fake Blender executable for smoke testing.
# Usage: scripts/create_fake_blender.sh <output_path>
set -euo pipefail

out="$1"
cat > "$out" <<'ENDOFSCRIPT'
#!/bin/bash
job_json="${@: -2:1}"
result_json="${@: -1}"
echo "Fake Blender: running" 1>&2
mkdir -p "$(dirname "$result_json")"
cat > "$result_json" <<'RESULT'
{"job_id":"fake","status":"completed","message":"Fake Blender success","artifacts":[{"path":"blender/scenes/smoke.blend","type":"blend","size_bytes":1024}],"scene_summary":{"objects":1,"meshes":1,"triangles":12,"vertices":24}}
RESULT
exit 0
ENDOFSCRIPT

chmod 755 "$out"
