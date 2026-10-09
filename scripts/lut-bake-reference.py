"""Release acceptance: exported cubes read by official OCIO and actual image reuse.

PYTHONPATH=target/ocio-oracle; Pillow required. Development tools only.
The executable has no Python runtime dependency.
"""
import hashlib
import json
import math
import struct
import subprocess
from pathlib import Path

import PyOpenColorIO as ocio
from PIL import Image

assert ocio.GetVersion() == "2.5.2"
root = Path(__file__).resolve().parents[1]
artifacts = root / "artifacts"
executable = root / "target/release/tinge.exe"


def run(request):
    result = subprocess.run([str(executable), "run", "-"], input=json.dumps(request),
                            text=True, capture_output=True, cwd=root, check=True)
    envelope = json.loads(result.stdout)
    assert envelope["ok"], envelope
    return envelope["data"]


def halton(index, base):
    fraction, value = 1., 0.
    while index:
        fraction /= base
        value += fraction * (index % base)
        index //= base
    return value


def file_cpu(path):
    return ocio.Config.CreateRaw().getProcessor(ocio.FileTransform(
        src=str(path), interpolation=ocio.INTERP_TETRAHEDRAL)).getDefaultCPUProcessor()


# Match the emitted Log-domain artifact to an independent native OCIO chain.
fixture = json.loads((root / "crates/ocio/tests/fixtures/grades-ocio-2.5.2.json").read_text())
case = fixture["cases"][0]
grade = case["transform"]["grade"]
pipeline = json.loads((root / "examples/aces2-srgb.json").read_text())
recipe = {"nodes": [{"id": "cdl", "op": {"type": "ocio_grade", "grade": grade}}], "output": "cdl"}
config = ocio.Config.CreateFromBuiltinConfig(case["config"]["name"])
linear, log = pipeline["working_space"] if "working_space" in pipeline else "Linear Rec.709 (sRGB)", "ACEScct"
decode = config.getProcessor(log, linear).getDefaultCPUProcessor()
encode = config.getProcessor(linear, log).getDefaultCPUProcessor()
cdl = ocio.CDLTransform(slope=grade["slope"], offset=grade["offset"], power=grade["power"], sat=grade["saturation"])
cdl.setStyle(ocio.CDL_NO_CLAMP)
group = ocio.GroupTransform([
    ocio.ColorSpaceTransform(src=linear, dst=log), cdl,
    ocio.ColorSpaceTransform(src=log, dst=linear)])
grading = config.getProcessor(group).getDefaultCPUProcessor()
log_cases = []
for size in [17, 33]:
    path = artifacts / f"acescct-cdl-{size}.cube"
    baked = run({"command": "lut_bake", "source": {"type": "recipe", "recipe": recipe, "color_pipeline": pipeline},
                 "output": str(path), "overwrite": True,
                 "options": {"size": size, "validation_samples": 4096,
                             "input_encoding": {"type": "ocio", "color_space": log},
                             "output_encoding": {"type": "ocio", "color_space": log}}})
    cpu = file_cpu(path)
    maximum = [0., 0., 0.]
    for i in range(1, 4097):
        # Explicit f32 input matches the native Frame, without generating its reference.
        rgb = [struct.unpack("f", struct.pack("f", halton(i, b)))[0] for b in [2, 3, 5]]
        expected = encode.applyRGBA(grading.applyRGBA(decode.applyRGBA([*rgb, 1.])))
        actual = cpu.applyRGBA([*rgb, 1.])
        assert actual[3] == expected[3] == 1.
        maximum = [max(m, abs(a - e)) for m, a, e in zip(maximum, actual, expected)]
    reported = baked["bake"]["sampled_error"]["max_absolute_rgb"]
    assert all(abs(a - b) < 3e-5 for a, b in zip(maximum, reported)), (maximum, reported)
    log_cases.append({"output": str(path.relative_to(root)), "bake": baked,
                      "official_ocio_file_vs_native_chain_max_absolute_rgb": maximum})


# A project revision with ordered Looks, encoded CDL and fractional node mix.
project = artifacts / "node-ocio-demo.tinge"
original_project = project.read_bytes()
cube = artifacts / "node-ocio-baked.cube"
options = json.loads((root / "examples/lut-display-options.json").read_text())
baked = run({"command": "lut_bake", "source": {"type": "project", "project": str(project), "revision": 1},
             "output": str(cube), "options": options, "overwrite": True})
assert project.read_bytes() == original_project
inspected = run({"command": "lut_inspect", "input": str(cube)})
assert inspected["lut"]["size_3d"] == 33
cpu = file_cpu(cube)
assert all(math.isfinite(v) for v in cpu.applyRGBA([.18, .4, .06, 1.]))
reused = artifacts / "baked-lut-demo.tinge"
if not reused.exists():
    run({"command": "init", "input": str(artifacts / "test-chart.png"), "project": str(reused)})
    run({"command": "apply", "project": str(reused), "expect_revision": 0,
         "edits": [{"type": "upsert_node", "node": {"id": "lut", "op": {"type": "lut", "path": str(cube),
                    "domain": "srgb", "interpolation": "tetrahedral"}}}, {"type": "set_output", "id": "lut"}]})
direct_path, reused_path = artifacts / "node-ocio-current-render.png", artifacts / "baked-lut-render.png"
direct = run({"command": "render", "project": str(project), "revision": 1, "output": str(direct_path), "bit_depth": 8, "overwrite": True})
rendered = run({"command": "render", "project": str(reused), "output": str(reused_path), "bit_depth": 8, "overwrite": True})
with Image.open(direct_path) as a, Image.open(reused_path) as b:
    a, b = a.convert("RGB"), b.convert("RGB")
    assert a.size == b.size
    errors = [abs(x - y) for x, y in zip(a.tobytes(), b.tobytes())]
    image_error = {"channels": len(errors), "max_8bit_steps": max(errors),
                   "mean_8bit_steps": sum(errors) / len(errors),
                   "rms_8bit_steps": math.sqrt(sum(e * e for e in errors) / len(errors))}
    comparison = Image.new("RGB", (a.width * 2, a.height))
    comparison.paste(a, (0, 0))
    comparison.paste(b, (a.width, 0))
    comparison.save(artifacts / "baked-lut-compare.png")
    # This is an image acceptance threshold, independent of the held-out estimator.
    assert image_error["max_8bit_steps"] <= 4, image_error

report = {"oracle": "Official OpenColorIO 2.5.2 Python wheel; emitted Iridas cubes via FileTransform",
          "scope": "engineering image and sampled opaque RGB; no Resolve application / camera-corpus acceptance",
          "log_cases": log_cases, "project_bake": baked, "project_json_unchanged": True,
          "project_before_sha256": hashlib.sha256(original_project).hexdigest(),
          "lut_inspect": inspected, "direct_render": direct, "reused_render": rendered,
          "direct_vs_baked_image": image_error,
          "comparison": "artifacts/baked-lut-compare.png (left original graph, right baked LUT)"}
(artifacts / "lut-bake-reference.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"log_cube_errors": [c["official_ocio_file_vs_native_chain_max_absolute_rgb"] for c in log_cases],
                  "image_error": image_error, "report": "artifacts/lut-bake-reference.json"}, indent=2))
