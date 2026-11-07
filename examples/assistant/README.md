# Assistant fixtures (Stage D2)

Tiny, synthetic fixtures of the bounded assistant surface. They carry no
dataset content: every mesh is a box of a few centimetres.

| Path | Content |
|---|---|
| `task.json` | a bounded assistant task (`michelangelo.assistant-task/0`) of the run `run-0001` |
| `candidate-valid/candidate.json` | an edit candidate (`michelangelo.edit-candidate/0`) that replaces the lid inside the region of the task |
| `candidate-valid/geometry/lid.obj` | the geometry of the candidate |

The task and the candidate belong to the run of `examples/edit/run-0001`
(package `demo-lantern`, components `base` and `lid`). The task allows the
lid, protects the base and bounds the change to the bounding box of the lid
with a margin of 5 %.

## The demo of the surface

Copy the run directory into a scratch directory first: the commands write the
scene store, the task and the audit log into the run directory.

The name of the run directory is the identity of the run: keep the
name `run-0001`, otherwise the candidate of the example belongs to another run.

```text
copy -r examples/edit/run-0001 /tmp/run-0001          # Windows: xcopy
michelangelo edit init   --run /tmp/run-0001 --protect base
michelangelo assistant task   --run /tmp/run-0001 --allowed lid --json
michelangelo assistant submit --run /tmp/run-0001 \
    --task /tmp/run-0001/assistant/tasks/asst-0001.json \
    --candidate examples/assistant/candidate-valid --json
michelangelo edit show --run /tmp/run-0001 --json
```

The submission is always `proposed`: the assistant never applies a
candidate. The new scene revision is published by an explicit acceptance:

```text
michelangelo edit accept --run /tmp/run-0001 --candidate cand-0001 --by alice
```

## The demo of the MCP transport

The transport serves the same surface to a local LLM client over stdio:

```text
michelangelo mcp serve --workspace <workspace>
```

The client writes one JSON-RPC 2.0 object per line into the stdin of the
process and reads the responses from its stdout. The workspace must contain
the run (`<workspace>/recon/<revision>/<run>`).

The transcript of the acceptance run (initialize, tools/list, get_task,
submit_candidate) is in `docs/specs/ASSISTANT_MCP.md`.
