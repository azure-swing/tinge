"""Real release RAW temperature workflow with independent numeric fixtures.
Requires the existing licensed corpus and development OpenEXR 3.4.12 wheel.
"""
import hashlib
import json
import subprocess
import sys
from pathlib import Path

root=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(root/'target/raw-oracle'))
import OpenEXR
import numpy as np
from PIL import Image,ImageDraw

exe=root/'target/release/vibecolor.exe'
artifacts=root/'artifacts'
source=root/'target/raw-corpus/canon-400d.cr2'
pipeline=json.loads((root/'examples/aces2-srgb.json').read_text(encoding='utf-8'))


def run(request,success=True):
    r=subprocess.run([str(exe),'run','-'],input=json.dumps(request),capture_output=True,encoding='utf-8',cwd=root)
    assert (r.returncode==0)==success,r.stderr
    result=json.loads(r.stdout if success else r.stderr)
    return result['data'] if success else result['error']


fixture=json.loads((root/'crates/io/tests/fixtures/raw-temperature-colour-0.4.7.json').read_text())
cases=[]
for kelvin,duv in [(3500,0),(6504,0),(9000,0),(6504,-0.005),(6504,0.005)]:
    options={'white_balance':{'type':'temperature','kelvin':kelvin,'tint_duv':duv}}
    plan=run({'command':'raw_plan','input':str(source),'options':options})
    assert plan['white_balance_source']=='temperature_kang2002_duv'
    assert all(v>0 and np.isfinite(v) for v in plan['white_balance_gains'])
    if kelvin==6504:
        expected=next(c['xy'] for c in fixture['cases'] if c['kelvin']==kelvin and c['tint_duv']==duv)
        np.testing.assert_allclose(plan['white_balance_white_xy'],expected,atol=6e-8,rtol=0)
    output=artifacts/f'raw-temperature-{kelvin}-{duv:+.3f}.png'
    recipe={'nodes':[{'id':'size','op':{'type':'resize','width':640,'height':427}}],'output':'size'}
    image=run({'command':'grade','input':str(source),'raw_develop':options,'recipe':recipe,
               'color_pipeline':pipeline,'output':str(output),'bit_depth':8,'overwrite':True})
    cases.append({'options':options,'plan':plan,'export':image})

# Real project applies temperature, renders in a persistent session, and restores it.
project=artifacts/'raw-temperature-demo.vcolor'
if not project.exists():run({'command':'init','input':str(source),'project':str(project),'raw_develop':{},'color_pipeline':pipeline})
state=run({'command':'show','project':str(project)});start=state['revision']
edited=run({'command':'apply','project':str(project),'expect_revision':start,
            'edits':[{'type':'set_raw_develop','options':{'white_balance':{'type':'temperature','kelvin':6504,'tint_duv':0.005}}}]})
before_invalid=project.read_bytes()
run({'command':'apply','project':str(project),'expect_revision':edited['revision'],
     'edits':[{'type':'set_raw_develop','options':{'white_balance':{'type':'temperature','kelvin':1200}}}]},False)
assert before_invalid==project.read_bytes()
child=subprocess.Popen([str(exe),'serve'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                       encoding='utf-8',cwd=root)
outputs=[]
requests=[]
for i,revision in enumerate([start,edited['revision'],start,edited['revision']]):
    output=artifacts/f'raw-temperature-cache-{i}.exr';outputs.append(output)
    requests.append({'command':'render','project':str(project),'revision':revision,
                     'output':str(output),'max_edge':256,'overwrite':True})
stdout,stderr=child.communicate('\n'.join(json.dumps(r) for r in requests)+'\n')
responses=[json.loads(s) for s in stdout.splitlines()]
assert child.returncode==0 and len(responses)==4 and all(r['ok'] for r in responses),stderr
pixels=[]
for path in outputs:
    with OpenEXR.File(str(path)) as f:pixels.append(f.channels()['RGBA'].pixels.copy())
np.testing.assert_array_equal(pixels[0],pixels[2]);np.testing.assert_array_equal(pixels[1],pixels[3])
assert not np.array_equal(pixels[0],pixels[1])
restored=run({'command':'restore','project':str(project),'expect_revision':edited['revision'],'revision':start})
restored_state=run({'command':'show','project':str(project)})
assert restored_state['history'][-1]['recipe_hash']==next(h['recipe_hash'] for h in state['history'] if h['id']==start)

sheet=Image.new('RGB',(1300,3*470),(24,24,24));draw=ImageDraw.Draw(sheet)
for i,c in enumerate(cases):
    wb=c['options']['white_balance'];x=(i%2)*650+5;y=(i//2)*470
    draw.text((x,y+8),f"Kelvin {wb['kelvin']} / Duv {wb['tint_duv']:+.3f}",fill=(240,240,240))
    with Image.open(c['export']['path']) if 'path' in c['export'] else Image.open(c['export']['output']) as im:sheet.paste(im.convert('RGB'),(x,y+32))
sheet.save(artifacts/'raw-temperature-contact-sheet.png')
report={'method':'Kang 2002 fitted Planckian locus and analytic CIE 1960 uv branch-normal tint offset',
        'fixture_oracle':'Colour Science 0.4.7; 33 Rust-tested vectors, absolute xy tolerance 3e-8',
        'real_camera':'Canon EOS 400D CC0; source SHA256 '+hashlib.sha256(source.read_bytes()).hexdigest(),
        'examples':cases,'project':str(project.relative_to(root)),'start_revision':start,
        'edited_revision':edited['revision'],'restored_revision':restored['revision'],
        'persistent_session_restore_rgba_pixels_exact':True,'scene_pixel_oracle':'Official OpenEXR 3.4.12',
        'invalid_temperature_project_unchanged':True,'numeric_lightroom_tint_equivalence':False,
        'scope':'Defined mathematical white-point controls; not spectral reconstruction, Adobe slider equivalence or target software appearance certification.'}
(artifacts/'raw-temperature-reference.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
print(json.dumps({'images':len(cases),'persistent_session_pixels_exact':True,'report':'artifacts/raw-temperature-reference.json'}))
