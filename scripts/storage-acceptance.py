"""Real Windows recycle + native viewer parity. Only generated fixture files are moved."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.request
spec = importlib.util.spec_from_file_location('agent_acceptance', Path(__file__).with_name('agent-acceptance.py'))
agent_acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent_acceptance)
MCP, png = agent_acceptance.MCP, agent_acceptance.png


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--executable', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    report = {'executable': str(args.executable.resolve())}
    mcp = MCP(args.executable.resolve())
    try:
        with tempfile.TemporaryDirectory(prefix='tinge-storage-', dir=args.report.parent.resolve()) as folder:
            root = Path(folder)
            source, project = root / 'source.png', root / 'test.tinge'
            png(source)
            mcp.data({'command':'init','input':str(source),'project':str(project)})
            first = mcp.data({'command':'preview','project':str(project),'include_analysis':False})
            first_path = Path(first['preview']['path'])
            repeated = mcp.data({'command':'preview','project':str(project),'include_analysis':False})
            assert repeated['preview']['path'] == str(first_path)
            old_draft = root / 'old-draft.png'
            mcp.data({'command':'render','project':str(project),'output':str(old_draft),'bit_depth':8,'temporary':True})
            # Changed registered previews and unregistered images are protected.
            changed = root / 'user-changed.png'
            mcp.data({'command':'preview','project':str(project),'output':str(changed),'include_analysis':False})
            changed.write_bytes(changed.read_bytes()+b'user data')
            unregistered = root / 'unknown.png'
            unregistered.write_bytes(b'keep unrelated file')
            durable = root / 'keep-export.png'
            mcp.data({'command':'render','project':str(project),'output':str(durable),'bit_depth':8})
            mcp.data({'command':'apply','project':str(project),'expect_revision':0,'edits':[{'type':'upsert_node','node':{'id':'light','op':{'type':'exposure','stops':0.2}}},{'type':'set_output','id':'light'}]})
            final = root / 'final.png'
            mcp.data({'command':'render','project':str(project),'output':str(final),'bit_depth':16,'temporary':True})
            plan = mcp.data({'command':'cleanup_plan','project':str(project),'revision':1})
            assert len(plan['files']) == 2, plan
            assert len(plan['skipped']) == 1 and len(plan['retained']) == 2, plan
            project_before = project.read_bytes()
            source_before = source.read_bytes()
            frozen = root / json.loads(project_before)['source']['path']
            frozen_before = frozen.read_bytes()
            recycled_sha = hashlib.sha256(old_draft.read_bytes()).hexdigest().upper()
            done = mcp.data({'command':'finalize','project':str(project),'expect_revision':1,'revision':1})
            assert done['finalized'] and not done['failed'] and len(done['recycled']) == 2, done
            assert project.read_bytes() == project_before and source.read_bytes() == source_before and frozen.read_bytes() == frozen_before
            assert final.exists() and durable.exists() and changed.exists() and unregistered.exists()
            assert not old_draft.exists() and not first_path.exists()
            assert mcp.data({'command':'project_info','project':str(project)})['final_revision'] == 1
            assert mcp.data({'command':'finalize','project':str(project),'expect_revision':1,'revision':1})['recycled_bytes'] == 0
            if os.name == 'nt':
                probe = Path(__file__).with_name('recycle-probe.ps1').resolve()
                result = subprocess.check_output(['powershell','-NoProfile','-File',str(probe),'-Directory',str(root),'-Restore'], encoding='utf-8', creationflags=0x08000000)
                restored = json.loads(result)
                assert any(item['sha256'] == recycled_sha for item in restored), restored
                deadline = time.monotonic()+10
                while not old_draft.exists() and time.monotonic()<deadline:
                    time.sleep(0.05)
                assert old_draft.exists() and hashlib.sha256(old_draft.read_bytes()).hexdigest().upper() == recycled_sha
                report['system_bin_and_exact_restore_verified'] = True
            # Historical revision renders after recycling; no extra file is created by viewer.
            old_preview = mcp.data({'command':'preview','project':str(project),'revision':0,'include_analysis':False})
            temp_before = set(Path(tempfile.gettempdir()).glob('tinge-view-*'))
            viewer = mcp.data({'command':'viewer_open','project':str(project)})
            url = viewer['url']
            token = url.rstrip('/').split('/')[-1]
            body = json.dumps({'revision':0,'reference':None,'max_edge':1600,'client_id':'storage-acceptance'}).encode()
            req = urllib.request.Request(url+'api/preview',data=body,headers={'Content-Type':'application/json','X-Tinge':token})
            job = json.load(urllib.request.urlopen(req))['job']
            deadline = time.monotonic()+15
            while True:
                status = json.load(urllib.request.urlopen(url+f'api/job/{job}'))
                if status['status'] == 'ready': break
                assert time.monotonic()<deadline, status
                time.sleep(0.025)
            image = urllib.request.urlopen(url+status['result']['image']).read()
            assert image == Path(old_preview['preview']['path']).read_bytes()
            assert set(Path(tempfile.gettempdir()).glob('tinge-view-*')) == temp_before
            mcp.data({'command':'viewer_close','project':str(project)})
            report.update({'passed':True,'registered_files_recycled':2,'recycled_bytes':done['recycled_bytes'],'source_and_history_unchanged':True,'durable_and_final_exports_retained':True,'changed_and_unknown_files_retained':True,'historical_viewer_matches_cli_png_bytes':True,'viewer_creates_no_temp_directory':True,'managed_preview_reuses_filename':True})
    finally:
        mcp.close()
    args.report.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(report,ensure_ascii=False,indent=2))


if __name__ == '__main__':
    main()
