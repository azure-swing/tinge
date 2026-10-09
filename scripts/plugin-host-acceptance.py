"""Ask the installed Codex host to discover Tinge; no model call or config edits."""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import threading


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--codex', type=Path, required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    options = {'creationflags': 0x08000000} if os.name == 'nt' else {}
    proc = subprocess.Popen([str(args.codex), 'app-server', '--stdio'],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, text=True, encoding='utf-8', **options)
    responses = queue.Queue()

    def read():
        for line in proc.stdout:
            responses.put(json.loads(line))
        responses.put(None)

    threading.Thread(target=read, daemon=True).start()

    def rpc(serial, method, params):
        proc.stdin.write(json.dumps({'id': serial, 'method': method, 'params': params}) + '\n')
        proc.stdin.flush()
        while True:
            response = responses.get(timeout=60)
            assert response is not None, 'Codex host exited before responding'
            if response.get('id') == serial:
                assert 'error' not in response, response
                return response['result']

    try:
        rpc(1, 'initialize', {'clientInfo': {'name': 'tinge-host-acceptance', 'version': '1'},
                              'capabilities': {'experimentalApi': True}})
        proc.stdin.write(json.dumps({'method': 'initialized'}) + '\n')
        proc.stdin.flush()
        result = rpc(2, 'mcpServerStatus/list', {'serverName': 'tinge', 'detail': 'full'})
        servers = [s for s in result['data'] if s['name'] == 'tinge']
        assert len(servers) == 1, result
        server = servers[0]
        report = {k: server.get(k) for k in ['name', 'pluginId', 'serverInfo', 'toolsError']}
        report['tool_count'] = len(server['tools'])
        if args.report:
            args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
        print(json.dumps(report, ensure_ascii=False, indent=2))
        assert not server.get('toolsError'), server.get('toolsError')
        assert server['pluginId'] == 'tinge@tinge-local', report
        assert server['serverInfo']['version'] == args.version, report
        assert report['tool_count'] == 42, report
        names = {tool['name'] for tool in server['tools'].values()}
        assert {'tinge_init', 'tinge_adjust', 'tinge_render'} <= names, names
    finally:
        proc.stdin.close()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.terminate()
            proc.wait(timeout=5)


if __name__ == '__main__':
    main()
