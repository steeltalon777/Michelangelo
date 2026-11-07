# Edit fixtures (Stage C)

Tiny, synthetic fixtures of the local edit boundary. They carry no dataset
content: every mesh is a box of a few centimetres.

| Path | Content |
|---|---|
| `base.obj` | a 0.1 m box centred at the origin |
| `lid.obj` | the lid of the box (0.08 x 0.02 x 0.08 m) |
| `lid-tall.obj` | a taller lid (the repair of the valid candidate) |
| `lid-out-of-region.obj` | a lid that leaves the allowed region |
| `run-0001/source/*.obj` | the components of the run (the base and the lid) |
| `run-0001/report.json` | a minimal reconstruction report of the run |
| `candidate-valid/` | a candidate that replaces the lid inside the allowed region |
| `candidate-out-of-region/` | a candidate whose geometry leaves the allowed region |
| `candidate-protected/` | a candidate that tries to change the protected base |

The run directory is shaped like an IT-5 run (`source/` with the OBJs of the
components and a `report.json`). A demo of the boundary:

```text
michelangelo edit init   --run examples/edit/run-0001
michelangelo edit submit --run examples/edit/run-0001 --candidate examples/edit/candidate-valid
michelangelo edit accept --run examples/edit/run-0001 --candidate cand-0001 --by alice
michelangelo edit show   --run examples/edit/run-0001 --json
```

The command writes the scene store into the run directory (`scene/`,
`candidates/` and `current.json`); copy the run directory to a scratch
directory before a demo if the repository must stay clean.
