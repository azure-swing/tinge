"""Pinned official OCIO oracle for cube ranges, combined shapers and interpolation.
The native executable does not use Python. PYTHONPATH=target/ocio-oracle.
"""
import itertools
import json
import tempfile
from pathlib import Path
import PyOpenColorIO as ocio

assert ocio.GetVersion() == "2.5.2"
root = Path(__file__).resolve().parent.parent
inputs = [[*p, (i % 5) / 4] for i, p in enumerate(itertools.permutations([.11, .31, .79]))]
inputs += [[-.8, 0., .3, 1.], [0., .2, .8, .25], [1., 1., 1., 0.],
           [4., -2., .4, .75], [.5, .5, .5, 1.]]
inputs += [[((i * 37) % 101) / 100, ((i * 61) % 103) / 102,
            ((i * 79) % 107) / 106, 1.] for i in range(1, 129)]
values = []
for b, g, r in itertools.product(range(3), repeat=3):
    r, g, b = r / 2, g / 2, b / 2
    values.append([r * r + .3 * g * b - .05, g * g + .2 * r * b,
                   b * b + .4 * r * g + .03])
rows3d = "".join(" ".join(format(v, ".9g") for v in p) + "\n" for p in values)
rows1d = "0 0 0\n0.05 0.1 0.02\n0.25 0.45 0.3\n0.8 0.7 0.9\n1.4 1.2 1.3\n"
texts = [
    ("Iridas 1D per-channel domain", "TITLE \"shaper # reference\"\nLUT_1D_SIZE 5\nDOMAIN_MIN -1 -0.5 0\nDOMAIN_MAX 3 2 1.5\n" + rows1d, ["trilinear"]),
    ("Iridas 3D per-channel domain", "LUT_3D_SIZE 3\nDOMAIN_MIN -1 -0.5 0\nDOMAIN_MAX 3 2 1.5\n" + rows3d, ["trilinear", "tetrahedral"]),
    ("Resolve combined independent ranges", "LUT_1D_SIZE 5\nLUT_1D_INPUT_RANGE -1 3\nLUT_3D_SIZE 3\nLUT_3D_INPUT_RANGE -0.2 1.4\n" + rows1d + rows3d, ["trilinear", "tetrahedral"]),
    ("Resolve 1D scalar input range", "LUT_1D_SIZE 5\nLUT_1D_INPUT_RANGE -1 3\n" + rows1d, ["trilinear"]),
    ("Resolve 3D scalar input range", "LUT_3D_SIZE 3\nLUT_3D_INPUT_RANGE -0.2 1.4\n" + rows3d, ["trilinear", "tetrahedral"]),
]
cases = []
with tempfile.TemporaryDirectory() as directory:
    for label, text, interpolations in texts:
        path = Path(directory) / "reference.cube"
        path.write_text(text, encoding="utf-8")
        for interpolation in interpolations:
            ocio.ClearAllCaches()
            transform = ocio.FileTransform(src=str(path), interpolation=ocio.INTERP_LINEAR if interpolation == "trilinear" else ocio.INTERP_TETRAHEDRAL)
            cpu = ocio.Config.CreateRaw().getProcessor(transform).getDefaultCPUProcessor()
            cases.append({"label": label, "cube": text, "interpolation": interpolation,
                          "input": inputs, "expected": [cpu.applyRGBA(p) for p in inputs]})
fixture = {"oracle": "Official OpenColorIO 2.5.2 Python wheel", "engine_version": ocio.GetVersion(), "cases": cases}
path = root / "crates/core/tests/fixtures/cube-ocio-2.5.2.json"
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(fixture, indent=2) + "\n", encoding="utf-8")
print(f"Wrote {len(cases)} cube cases, {len(inputs)} vectors each, to {path}")
