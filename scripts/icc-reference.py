"""Development interoperability check: native ICC files against Pillow/LittleCMS.

Uses the engineering chart, not a camera corpus. Python is not a runtime dependency.
Run after building: python scripts/icc-reference.py
"""
import io
import json
import subprocess
from pathlib import Path
from PIL import Image, ImageCms, __version__ as pillow_version

root = Path(__file__).resolve().parents[1]
executable = root / "target/release/tinge.exe"
artifacts = root / "artifacts"


def run(request):
    result = subprocess.run([str(executable), "run", "-"], input=json.dumps(request),
                            text=True, capture_output=True, cwd=root, check=True)
    envelope = json.loads(result.stdout)
    assert envelope["ok"], envelope
    return envelope["data"]


cases = []
for primaries in ["srgb", "display_p3", "rec2020", "aces_cg"]:
    source = artifacts / f"icc-{primaries}-8.png"
    output = artifacts / f"icc-{primaries}-native-srgb.png"
    run({"command": "grade", "input": str(artifacts / "test-chart.png"),
         "recipe": {}, "output": str(source), "bit_depth": 8, "overwrite": True,
         "output_space": {"primaries": primaries, "transfer": "srgb"}})
    run({"command": "grade", "input": str(source), "recipe": {},
         "output": str(output), "bit_depth": 8, "overwrite": True})
    with Image.open(source) as image:
        assert image.info.get("icc_profile")
        reference = ImageCms.profileToProfile(
            image.convert("RGB"), ImageCms.ImageCmsProfile(io.BytesIO(image.info["icc_profile"])),
            ImageCms.createProfile("sRGB"), renderingIntent=ImageCms.Intent.RELATIVE_COLORIMETRIC,
            outputMode="RGB")
    with Image.open(output) as native:
        errors = [abs(a-b) for a, b in zip(reference.tobytes(), native.convert("RGB").tobytes())]
    maximum = max(errors)
    assert maximum <= 2, (primaries, maximum)
    cases.append({"primaries": primaries, "channels": len(errors),
                  "max_error_8bit_steps": maximum,
                  "different_channels": sum(e != 0 for e in errors)})

report = {"oracle": "Pillow ImageCms / LittleCMS", "pillow": pillow_version,
          "lcms": ImageCms.core.littlecms_version, "tolerance_8bit_steps": 2, "cases": cases,
          "limitations": "8-bit SDR RGB exchange only; not floating HDR or display calibration"}
(artifacts / "icc-interoperability.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps(report, indent=2))
