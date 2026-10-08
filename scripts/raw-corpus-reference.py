"""Limited real camera acceptance; Python/rawpy/Pillow are development tools only.
Install: python -m pip install --target target/raw-oracle --no-deps rawpy==0.27.1 OpenEXR==3.4.12
Run from any directory: python scripts/raw-corpus-reference.py [--download]
"""
import argparse
import hashlib
import json
import math
import shutil
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

root = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / 'target/raw-oracle'))
import numpy as np
import rawpy
import OpenEXR
from PIL import Image, ImageDraw

assert rawpy.__version__ == '0.27.1'
assert OpenEXR.__version__ == '3.4.12'
args = argparse.ArgumentParser()
args.add_argument('--download', action='store_true')
args = args.parse_args()
artifacts = root / 'artifacts'
exe = root / 'target/release/vibecolor.exe'
manifest = json.loads((artifacts / 'raw-corpus-manifest.json').read_text(encoding='utf-8'))
pipeline = json.loads((root / 'examples/aces2-srgb.json').read_text())


def run(request, success=True):
    result = subprocess.run([str(exe), 'run', '-'], input=json.dumps(request),
                            capture_output=True, text=True, encoding='utf-8', cwd=root)
    assert (result.returncode == 0) == success, result.stderr
    response = json.loads(result.stdout if success else result.stderr)
    assert response['ok'] == success, response
    return response['data'] if success else response['error']


def summary(analysis):
    return {k: v for k, v in analysis.items() if k != 'histogram'}


def fetch(sample):
    path = root / sample['path']
    if not path.exists():
        assert args.download, f'Missing {path}; rerun with --download'
        assert sample['license_url'] == 'https://creativecommons.org/publicdomain/zero/1.0/'
        path.parent.mkdir(parents=True, exist_ok=True)
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        url = urllib.parse.quote(sample['url'], safe=':/()')
        with opener.open(url, timeout=45) as response:
            data = response.read()
        assert hashlib.sha256(data).hexdigest() == sample['sha256']
        path.write_bytes(data)
    assert path.stat().st_size == sample['bytes']
    assert hashlib.sha256(path.read_bytes()).hexdigest() == sample['sha256']
    return path


records, tiles = [], []
for sample in manifest['samples']:
    path = fetch(sample)
    with rawpy.imread(str(path)) as reference:
        w, h = reference.sizes.raw_width, reference.sizes.raw_height
        points = [[int(x), int(y)] for y in np.linspace(0, h - 1, 16, dtype=int)
                  for x in np.linspace(0, w - 1, 16, dtype=int)]
        plan = run({'command': 'raw_plan', 'input': str(path), 'sensor_points': points})
        assert plan['sensor_dimensions'] == [w, h]
        assert plan['camera_planes'] == reference.num_colors
        expected = [int(reference.raw_image[y, x]) for x, y in points]
        actual = [p['native_values'][0] for p in plan['sensor_probes']]
        assert actual == expected, (sample['model'], max(abs(a - b) for a, b in zip(actual, expected)))
        reference_pattern = None
        if plan['cfa_pattern'] is not None:
            side = math.isqrt(len(plan['cfa_pattern']))
            colours = reference.raw_colors
            reference_pattern = ''.join(chr(reference.color_desc[colours[y, x]])
                                        for y in range(side) for x in range(side))
            assert reference_pattern == plan['cfa_pattern'], sample['model']
        ref_meta = {'sizes': reference.sizes._asdict(), 'color_desc': reference.color_desc.decode(),
                    'black_level_per_channel': reference.black_level_per_channel,
                    'white_level': reference.white_level,
                    'camera_whitebalance': reference.camera_whitebalance,
                    'sensor_origin_cfa_pattern': reference_pattern}
    baseline = run({'command': 'analyze', 'input': str(path), 'raw_develop': {}})
    boosted = run({'command': 'analyze', 'input': str(path), 'raw_develop': {'exposure_ev': 1}})
    for key in ['rgb_mean', 'rgb_min', 'rgb_max']:
        np.testing.assert_allclose(boosted[key], np.array(baseline[key]) * 2, rtol=2e-6, atol=1e-7)
    if plan['camera_planes'] == 1:
        assert baseline['rgb_mean'][0] == baseline['rgb_mean'][1] == baseline['rgb_mean'][2]
        changed = {'exposure_ev': 0.5}
    else:
        gains = list(plan['white_balance_gains'])
        gains[0] *= 1.12
        gains[2] *= 0.88
        changed = {'white_balance': {'type': 'camera_gains', 'gains': gains}, 'exposure_ev': 0.25}
    width = 640
    height = round(width * baseline['height'] / baseline['width'])
    recipe = {'nodes': [{'id': 'preview-size', 'op': {'type': 'resize', 'width': width, 'height': height}}], 'output': 'preview-size'}
    image_reports = []
    for label, options in [('baseline', {}), ('adjusted', changed)]:
        output = artifacts / f'raw-{path.stem}-{label}.png'
        image_reports.append(run({'command': 'grade', 'input': str(path), 'raw_develop': options,
                                  'recipe': recipe, 'color_pipeline': pipeline, 'output': str(output),
                                  'bit_depth': 8, 'overwrite': True}))
        tiles.append((f'{sample["make"]} {sample["model"]}: {label}', Image.open(output).convert('RGB')))
    plan['sensor_probes'] = {'count': len(points), 'max_native_value_difference': 0,
                            'coordinate_grid': '16x16 including sensor boundaries',
                            'native_sample_values': expected}
    records.append({'sample': sample, 'plan': plan, 'independent_libraw_metadata': ref_meta,
                    'analysis': summary(baseline), 'exposure_plus_one': summary(boosted),
                    'custom_options': changed, 'preview_exports': image_reports,
                    'metadata_comparison_scope': 'Full sensor dimensions/plane count and 256 native samples exact. Crop/levels are recorded independently, not asserted equal; no developed-image parity claim.'})
    print(json.dumps({'camera': sample['model'], 'native_samples_exact': len(points),
                      'developed_dimensions': [baseline['width'], baseline['height']],
                      'wb_source': plan['white_balance_source']}, ensure_ascii=False), flush=True)

# A real project keeps RAW controls revision-local and source independent of the import path.
project = artifacts / 'raw-corpus-demo.vcolor'
if not project.exists():
    with tempfile.TemporaryDirectory(prefix='vibecolor-raw-') as temp:
        input_copy = Path(temp) / 'source.cr2'
        shutil.copyfile(root / manifest['samples'][0]['path'], input_copy)
        run({'command': 'init', 'input': str(input_copy), 'project': str(project),
             'raw_develop': {}, 'color_pipeline': pipeline})
        input_copy.unlink()
state = run({'command': 'show', 'project': str(project)})
revision = state['revision']
before_output = artifacts / 'raw-project-before.exr'
run({'command': 'render', 'project': str(project), 'output': str(before_output), 'max_edge': 256, 'overwrite': True})
updated = run({'command': 'apply', 'project': str(project), 'expect_revision': revision,
               'edits': [{'type': 'set_raw_develop', 'options': {'exposure_ev': 1}}]})
before_invalid = project.read_bytes()
run({'command': 'apply', 'project': str(project), 'expect_revision': updated['revision'],
     'edits': [{'type': 'set_raw_develop', 'options': {'white_levels': {'repeat': [1, 1], 'values': [0]}}}]}, False)
assert project.read_bytes() == before_invalid
after_output = artifacts / 'raw-project-adjusted.exr'
run({'command': 'render', 'project': str(project), 'output': str(after_output), 'max_edge': 256, 'overwrite': True})
with OpenEXR.File(str(before_output)) as image:
    before_pixels = image.channels()['RGBA'].pixels.copy()
with OpenEXR.File(str(after_output)) as image:
    adjusted_pixels = image.channels()['RGBA'].pixels.copy()
np.testing.assert_allclose(adjusted_pixels[:, :, :3], before_pixels[:, :, :3] * 2, rtol=2e-6, atol=1e-7)
np.testing.assert_array_equal(adjusted_pixels[:, :, 3], before_pixels[:, :, 3])
restored = run({'command': 'restore', 'project': str(project), 'expect_revision': updated['revision'], 'revision': revision})
restored_output = artifacts / 'raw-project-restored.exr'
run({'command': 'render', 'project': str(project), 'output': str(restored_output), 'max_edge': 256, 'overwrite': True})
with OpenEXR.File(str(restored_output)) as image:
    restored_pixels = image.channels()['RGBA'].pixels.copy()
np.testing.assert_array_equal(before_pixels, restored_pixels)
exr_bytes_equal = before_output.read_bytes() == restored_output.read_bytes()

row_heights = [max(tiles[i][1].height, tiles[i + 1][1].height) + 42 for i in range(0, len(tiles), 2)]
sheet = Image.new('RGB', (1300, sum(row_heights)), (24, 24, 24))
draw = ImageDraw.Draw(sheet)
y = 0
for row, height in enumerate(row_heights):
    for col in range(2):
        label, tile = tiles[row * 2 + col]
        draw.text((col * 650 + 5, y + 8), label, fill=(240, 240, 240))
        sheet.paste(tile, (col * 650 + 5, y + 32))
    y += height
sheet.save(artifacts / 'raw-camera-contact-sheet.png')
report = {'oracle': f'rawpy {rawpy.__version__}, LibRaw {rawpy.libraw_version}',
          'scene_pixel_oracle': f'Official OpenEXR {OpenEXR.__version__} Python wheel',
          'camera_files': len(records), 'native_sensor_samples_exact': len(records) * 256,
          'samples': records, 'project': {'path': str(project.relative_to(root)),
              'start_revision': revision, 'adjusted_revision': updated['revision'], 'restored_revision': restored['revision'],
              'import_path_removed': True, 'restore_scene_exr_pixels_exact': True,
              'restored_rgba_pixel_count': int(before_pixels.shape[0] * before_pixels.shape[1]),
              'restore_scene_exr_bytes_equal': exr_bytes_equal,
              'exr_file_layout_scope': 'Parallel EXR chunk order may vary; restored decoded float32 RGBA pixels are exact, not a promise of identical file bytes.',
              'invalid_edit_project_unchanged': True},
          'scope': 'Four selected cameras; full sensor dimensions/plane count and sparse native values verified independently, finite RAW renders, EV doubling and real project restore. No full camera/codec coverage or Lightroom/Resolve appearance certification.'}
(artifacts / 'raw-corpus-reference.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'camera_files': len(records), 'native_samples_exact': len(records) * 256,
                  'project_restore_pixels_exact': True, 'exr_bytes_equal': exr_bytes_equal,
                  'report': 'artifacts/raw-corpus-reference.json'}), flush=True)
