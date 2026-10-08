"""Pinned ASC file oracle from the official OCIO 2.5.2 Python wheel (development only)."""
import json
import tempfile
from pathlib import Path
import PyOpenColorIO as ocio

assert ocio.GetVersion() == "2.5.2"
root = Path(__file__).resolve().parents[1]
corrections = [
    '<ColorCorrection id="1"><Description>Warm &amp; 雪</Description><SOPNode><Description>SOP &lt;one&gt;</Description><Slope>1.05 0.94 1.1</Slope><Offset>-0.01 0.02 0</Offset><Power>0.95 1.1 1.02</Power></SOPNode><SatNode><Description>SAT first</Description><Saturation>0.8</Saturation></SatNode></ColorCorrection>',
    '<ColorCorrection id="cool&amp;&quot;雪"><Description>Cool</Description><InputDescription>Log &amp; camera</InputDescription><ViewingDescription>Reference view</ViewingDescription><SOPNode><Slope>0.85 1.12 1.0</Slope><Offset>0.02 -0.01 0.0</Offset><Power>1.1 0.9 1.0</Power></SOPNode><SatNode><Saturation>1.2</Saturation></SatNode></ColorCorrection>',
]
ids = ['1', 'cool&"雪']
inputs = [[0., 0., 0., 1.], [.18, .4, .06, .25], [.6, .3, .1, .75],
          [16., 4., .1, 1.], [-.05, .2, .4, 1.], [1., 1., 1., 0.]]
files = []
with tempfile.TemporaryDirectory() as folder:
    for extension, xml in [
        ('cc', corrections[0]),
        ('ccc', '<ColorCorrectionCollection xmlns="urn:ASC:CDL:v1.01"><Description>Collection one</Description><Description>Collection two</Description>' + ''.join(corrections) + '</ColorCorrectionCollection>'),
        ('cdl', '<ColorDecisionList xmlns="urn:ASC:CDL:v1.2"><Description>Decisions</Description>' + ''.join('<ColorDecision>' + c + '</ColorDecision>' for c in corrections) + '</ColorDecisionList>'),
    ]:
        path = Path(folder) / f'reference.{extension}'
        xml = '<?xml version="1.0" encoding="UTF-8"?>\n' + xml
        path.write_text(xml, encoding='utf-8')
        cases = []
        for index, id in enumerate(ids[:1] if extension == 'cc' else ids):
            transform = ocio.CDLTransform.CreateFromFile(str(path), id)
            parameters = {'id': transform.getID(), 'slope': list(transform.getSlope()),
                          'offset': list(transform.getOffset()), 'power': list(transform.getPower()),
                          'saturation': transform.getSat()}
            for style, native_style in [('asc', ocio.CDL_ASC), ('no_clamp', ocio.CDL_NO_CLAMP)]:
                for inverse in [False, True]:
                    transform.setStyle(native_style)
                    transform.setDirection(ocio.TRANSFORM_DIR_INVERSE if inverse else ocio.TRANSFORM_DIR_FORWARD)
                    cpu = ocio.Config.CreateRaw().getProcessor(transform).getDefaultCPUProcessor()
                    cases.append({'index': index, 'parameters': parameters, 'style': style, 'inverse': inverse,
                                  'input': inputs, 'expected': [cpu.applyRGBA(p) for p in inputs]})
        files.append({'format': extension, 'xml': xml, 'cases': cases})
fixture = {'oracle': 'Official OpenColorIO 2.5.2 Python wheel', 'files': files}
path = root / 'crates/ocio/tests/fixtures/cdl-exchange-ocio-2.5.2.json'
path.write_text(json.dumps(fixture, indent=2) + '\n', encoding='utf-8')
print(f'Wrote {sum(len(f["cases"]) for f in files)} file/style/direction cases, 6 RGBA vectors each')
