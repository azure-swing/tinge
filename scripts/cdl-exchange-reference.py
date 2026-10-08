"""Release file interoperability with official OCIO 2.5.2 and a real project.
Development only: PYTHONPATH=target/ocio-oracle. No Python runtime dependency.
"""
import hashlib
import json
import subprocess
from pathlib import Path
import PyOpenColorIO as ocio

assert ocio.GetVersion() == '2.5.2'
root = Path(__file__).resolve().parents[1]
artifacts = root / 'artifacts'
exe = root / 'target/release/vibecolor.exe'


def run(request):
    result = subprocess.run([str(exe), 'run', '-'], input=json.dumps(request), text=True, encoding='utf-8',
                            capture_output=True, cwd=root, check=True)
    envelope = json.loads(result.stdout)
    assert envelope['ok'], envelope
    return envelope['data']


fixture = json.loads((root / 'crates/ocio/tests/fixtures/cdl-exchange-ocio-2.5.2.json').read_text())
cases = []
for source in fixture['files']:
    extension = source['format']
    path = artifacts / f'cdl-input.{extension}'
    path.write_text(source['xml'], encoding='utf-8')
    inspected = run({'command': 'cdl_inspect', 'input': str(path)})
    output = artifacts / f'cdl-roundtrip.{extension}'
    exported = run({'command': 'cdl_export', 'source': {'type': 'document', 'document': inspected['document']},
                    'output': str(output), 'overwrite': True})
    group = ocio.CDLTransform.CreateGroupFromFile(str(output))
    assert len(group) == len(inspected['document']['corrections'])
    for transform, correction in zip(group, inspected['document']['corrections']):
        assert transform.getID() == correction['id']
        assert list(transform.getSlope()) == correction['slope']
        assert list(transform.getOffset()) == correction['offset']
        assert list(transform.getPower()) == correction['power']
        assert transform.getSat() == correction['saturation']
    again = run({'command': 'cdl_inspect', 'input': str(output)})
    assert again['document'] == inspected['document']
    cases.append({'input': str(path.relative_to(root)), 'output': str(output.relative_to(root)),
                  'hash': exported['hash'], 'bytes': exported['bytes'],
                  'official_parameter_and_id_equality': True, 'native_metadata_equality': True})

# Preserve precision beyond OCIO's own XML writer's 16 significant digits.
precise = json.loads((root / 'examples/cdl/document.json').read_text())
precise['corrections'][0]['slope'][0] = 1.2345678901234567
precise['corrections'][0]['offset'][0] = -0.12345678901234566
precise['corrections'][0]['metadata']['descriptions'] = ['literal &amp; and & < > " 雪']
precision_path = artifacts / 'cdl-precise.cc'
run({'command': 'cdl_export', 'source': {'type': 'document', 'document': precise},
     'output': str(precision_path), 'overwrite': True})
native = ocio.CDLTransform.CreateFromFile(str(precision_path), 'balance')
assert native.getSlope()[0] == precise['corrections'][0]['slope'][0]
assert native.getOffset()[0] == precise['corrections'][0]['offset'][0]
assert run({'command': 'cdl_inspect', 'input': str(precision_path)})['document']['corrections'][0]['metadata']['descriptions'] == precise['corrections'][0]['metadata']['descriptions']

project = artifacts / 'cdl-exchange-demo.vcolor'
imported = run({'command': 'cdl_import', 'input': str(artifacts / 'cdl-input.ccc'),
                'selector': {'type': 'id', 'id': '1'}, 'color_space': 'ACEScct', 'style': 'no_clamp'})
pipeline = json.loads((root / 'examples/aces2-srgb.json').read_text())
if not project.exists():
    run({'command': 'init', 'input': str(artifacts / 'test-chart.png'), 'project': str(project),
         'color_pipeline': pipeline})
    run({'command': 'apply', 'project': str(project), 'expect_revision': 0,
         'label': 'Import ASC parameters in ACEScct with source descriptions',
         'edits': [{'type': 'upsert_node', 'node': {'id': 'imported_cdl', 'op': imported['op'], 'mix': 0.65}},
                   {'type': 'set_output', 'id': 'imported_cdl'}]})
before = project.read_bytes()
exported = run({'command': 'cdl_export', 'source': {'type': 'project', 'project': str(project), 'revision': 1,
                'nodes': ['imported_cdl']}, 'output': str(artifacts / 'cdl-exchange-demo.cdl'), 'overwrite': True})
assert project.read_bytes() == before
assert exported['source_context']['nodes'][0]['mix'] != 1.
assert exported['source_context']['nodes'][0]['op']['grade']['style'] == 'no_clamp'
transform = ocio.CDLTransform.CreateFromFile(str(artifacts / 'cdl-exchange-demo.cdl'), '1')
assert list(transform.getSlope()) == imported['grade']['slope']
rendered = run({'command': 'render', 'project': str(project), 'output': str(artifacts / 'cdl-exchange-render.png'),
                'bit_depth': 8, 'overwrite': True})
scene = run({'command': 'render', 'project': str(project), 'output': str(artifacts / 'cdl-exchange-scene.exr'),
             'overwrite': True})
comparison = run({'command': 'compare', 'project': str(project), 'output': str(artifacts / 'cdl-exchange-compare.png'),
                  'overwrite': True})
report = {'oracle': 'Official OpenColorIO 2.5.2 Python wheel', 'files': cases,
          'full_f64_and_xml_entity_roundtrip': True, 'project_json_unchanged_on_export': True,
          'project_sha256': hashlib.sha256(before).hexdigest(), 'import': imported,
          'project_export': exported, 'render': rendered, 'scene': scene, 'compare': comparison,
          'scope': 'ASC forward parameter exchange and standard descriptions; source_context records omitted runtime interpretation; no Resolve application certification'}
(artifacts / 'cdl-exchange-reference.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'file_cases': len(cases), 'full_f64_and_xml_entities': True,
                  'clipped_channels': rendered['export']['clipped_channels'],
                  'report': 'artifacts/cdl-exchange-reference.json'}, indent=2))
