"""Development-only independent matting fixtures (PyMatting 1.1.16 + SciPy).

python -m pip install --target target/cutout-oracle --no-deps pymatting==1.1.16
python scripts/cutout-reference.py
No Python, NumPy, OpenCV or model dependency is added to the delivered CLI.
"""
from pathlib import Path
import json
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "target/cutout-oracle"))
import numpy as np
import scipy
from scipy.sparse import eye
from scipy.sparse.linalg import spsolve
import pymatting
from pymatting import cf_laplacian


def decode_srgb(values):
    return np.where(values <= 0.04045, values / 12.92, ((values + 0.055) / 1.055) ** 2.4)


def encode_srgb(values):
    return np.where(values <= 0.0031308, values * 12.92, 1.055 * values ** (1.0 / 2.4) - 0.055)


def main():
    cases = []
    rng = np.random.default_rng(81725)
    w, h = 9, 7
    for name in ["constant_harmonic", "color_mixture", "textured_seeds"]:
        x = np.tile(np.linspace(0, 1, w), (h, 1))
        if name == "constant_harmonic":
            encoded = np.full((h, w, 3), [0.3, 0.4, 0.6])
        elif name == "color_mixture":
            encoded = np.stack([0.1 + 0.7*x, 0.8 - 0.6*x, 0.2 + 0.2*x], axis=-1)
        else:
            encoded = rng.uniform(0.1, 0.9, (h, w, 3))
        linear = decode_srgb(encoded).astype(np.float32)
        # Independent sRGB roundtrip of the actual f32 Frame values.
        encoded = encode_srgb(linear.astype(np.float64))
        trimap = np.full((h, w), 0.5)
        if name == "textured_seeds":
            trimap[0, :] = trimap[-1, :] = 0
            trimap[:, 0] = trimap[:, -1] = 0
            trimap[2:5, 3:6] = 1
        else:
            trimap[:, 0] = 0
            trimap[:, -1] = 1
        laplacian = cf_laplacian(encoded, epsilon=1e-7, radius=1).tocsr()
        known = (trimap.ravel() == 0) | (trimap.ravel() == 1)
        unknown = ~known
        lhs = laplacian[unknown, :][:, unknown] + 1e-8 * eye(unknown.sum(), format="csr")
        rhs = -laplacian[unknown, :][:, known] @ trimap.ravel()[known] + 0.5e-8
        solved = spsolve(lhs, rhs)
        expected = trimap.ravel().copy()
        expected[unknown] = np.clip(solved, 0, 1)
        residual = np.linalg.norm(lhs @ solved - rhs) / max(np.linalg.norm(rhs), 1e-15)
        assert residual < 1e-10
        cases.append({"name": name, "width": w, "height": h,
                      "pixels": np.concatenate([linear, np.ones((h, w, 1))], axis=-1).reshape(-1, 4).tolist(),
                      "trimap": trimap.ravel().tolist(), "expected": expected.tolist(),
                      "absolute_alpha_tolerance": 0.0005,
                      "oracle_relative_residual": float(residual)})
    result = {"source": "https://pymatting.github.io/laplacian.html",
              "pymatting": pymatting.__version__, "numpy": np.__version__, "scipy": scipy.__version__,
              "method": "official cf_laplacian; SciPy direct Dirichlet solve; diagonal 1e-8 prior 0.5",
              "epsilon": 1e-7, "cases": cases}
    path = ROOT / "crates/core/tests/fixtures/matting-pymatting-1.1.16.json"
    path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"fixture": str(path.relative_to(ROOT)), "cases": len(cases),
                      "pymatting": result["pymatting"]}))


if __name__ == "__main__":
    main()
