"""Generate fixed independent CCT/uv fixtures from Colour Science 0.4.7.
Install development-only: pip install --target target/raw-oracle --no-deps colour-science==0.4.7
The Rust implementation uses analytical derivatives; this oracle uses finite differences.
"""
from pathlib import Path
import json
import sys
import numpy as np

root = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / 'target/raw-oracle'))
import colour

assert colour.__version__ == '0.4.7'


def point(kelvin, duv):
    xy = colour.temperature.CCT_to_xy_Kang2002(kelvin)
    uv = colour.xy_to_UCS_uv(xy)
    if kelvin in [2222, 4000, 25000]:
        tangent = (uv - colour.xy_to_UCS_uv(colour.temperature.CCT_to_xy_Kang2002(kelvin - 0.001))) / 0.001
    elif kelvin == 1667:
        tangent = (colour.xy_to_UCS_uv(colour.temperature.CCT_to_xy_Kang2002(kelvin + 0.001)) - uv) / 0.001
    else:
        delta = min([0.001] + [abs(kelvin - boundary) / 4 for boundary in [2222, 4000]])
        tangent = (colour.xy_to_UCS_uv(colour.temperature.CCT_to_xy_Kang2002(kelvin + delta)) -
                   colour.xy_to_UCS_uv(colour.temperature.CCT_to_xy_Kang2002(kelvin - delta))) / (2 * delta)
    normal = np.array([-tangent[1], tangent[0]])
    normal /= np.linalg.norm(normal)
    if normal[1] < 0:
        normal *= -1
    shifted = uv + duv * normal
    xy = colour.UCS_uv_to_xy(shifted)
    assert np.all(xy > 0) and sum(xy) < 1
    return {'kelvin': kelvin, 'tint_duv': duv, 'xy': xy.tolist(), 'uv_1960': shifted.tolist()}


cases = [point(t, d) for t in [1667, 2000, 2222, 3000, 3999.5, 4000, 4000.5, 4500, 6504, 10000, 25000]
         for d in [-0.005, 0, 0.005]]
fixture = {'oracle': 'Colour Science 0.4.7 CCT_to_xy_Kang2002 and CIE 1960 xy/uv converters',
           'tint_reference': 'Finite-difference tangent; positive normal v; one-sided derivative at polynomial boundaries/domain limits',
           'tolerance_absolute_xy': 3e-8, 'cases': cases}
path = root / 'crates/io/tests/fixtures/raw-temperature-colour-0.4.7.json'
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(fixture, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'cases': len(cases), 'fixture': str(path.relative_to(root))}))
