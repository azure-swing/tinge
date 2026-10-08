"""Native MCP acceptance; Python stdlib only, no plugin runtime dependency."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import queue
import struct
import subprocess
import tempfile
import threading
import time
import zlib


def png(path):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    data = b''.join(b'\0' + bytes([80, 110, 140, 255]) * 8 for _ in range(6))
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 8, 6, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(data)) + chunk(b'IEND', b''))


def working_set(pid):
    if os.name != 'nt':
        return None
    from ctypes import wintypes
    class Counters(ctypes.Structure):
        _fields_ = [('cb', wintypes.DWORD), ('faults', wintypes.DWORD)] + [(key, ctypes.c_size_t) for key in ('peak_ws', 'ws', 'peak_page', 'page', 'peak_nonpage', 'nonpage', 'pagefile', 'peak_pagefile')]
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    psapi = ctypes.WinDLL('psapi', use_last_error=True)
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    handle = kernel.OpenProcess(0x410, False, pid)
    if not handle:
        return None
    try:
        info = Counters()
        info.cb = ctypes.sizeof(info)
        return info.ws if psapi.GetProcessMemoryInfo(handle, ctypes.byref(info), info.cb) else None
    finally:
        kernel.CloseHandle(handle)


class MCP:
    def __init__(self, exe):
        options = {'creationflags': 0x08000000} if os.name == 'nt' else {}
        self.proc = subprocess.Popen([str(exe), 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, encoding='utf-8', **options)
        self.responses = queue.Queue()
        self.serial = 0
        def reader():
            for line in self.proc.stdout:
                self.responses.put(json.loads(line))
            self.responses.put(None)
        threading.Thread(target=reader, daemon=True).start()
        self.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'agent-acceptance', 'version': '1'}})

    def rpc(self, method, params=None):
        self.serial += 1
        body = {'jsonrpc': '2.0', 'id': self.serial, 'method': method}
        if params is not None:
            body['params'] = params
        self.proc.stdin.write(json.dumps(body) + '\n')
        self.proc.stdin.flush()
        response = self.responses.get(timeout=90)
        assert response and response['id'] == self.serial, response
        assert 'error' not in response, response
        return response['result']

    def call(self, request, full=False, allow_error=False):
        request = dict(request)
        command = request.pop('command')
        if command == 'job_submit':
            nested = dict(request.pop('request'))
            command = 'submit_' + nested.pop('command')
            nested.update(request)
            request = nested
        elif command == 'show' and not full:
            command = 'project_info'
        if full:
            request['_response'] = 'full'
            request['_inline_image'] = True
        response = self.rpc('tools/call', {'name': 'vibecolor_' + command, 'arguments': request})
        assert allow_error or not response.get('isError'), response
        return response

    def data(self, request):
        return self.call(request)['structuredContent']

    def done(self, job, timeout=90):
        deadline = time.monotonic() + timeout
        while True:
            response = self.call({'command': 'job_status', 'job': job}, allow_error=True)
            data = response['structuredContent']
            if data['status'] in ('completed', 'failed', 'cancelled'):
                return data
            assert time.monotonic() < deadline, data
            time.sleep(0.025)

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=20)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--raw-project', type=Path)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    report = {'executable': str(args.executable.resolve()), 'binary_bytes': args.executable.stat().st_size}
    mcp = MCP(args.executable.resolve())
    try:
        with tempfile.TemporaryDirectory(prefix='vibecolor-agent-') as root:
            root = Path(root)
            source, project = root / 'source.png', root / 'test.vcolor'
            png(source)
            mcp.data({'command': 'init', 'input': str(source), 'project': str(project)})
            tools = mcp.rpc('tools/list')
            report['tool_discovery_bytes'] = len(json.dumps(tools, separators=(',', ':')).encode())
            report['preview_schema_bytes'] = len(json.dumps(mcp.data({'command': 'schema', 'target': 'preview'}), separators=(',', ':')).encode())
            report['idle_working_set_bytes'] = working_set(mcp.proc.pid)
            names = {t['name'] for t in tools['tools']}
            assert 'vibecolor_agent' not in names and 'vibecolor_run' not in names
            assert 'vibecolor_job_submit' not in names and 'vibecolor_batch' not in names
            assert 'vibecolor_submit_edit_preview' in names
            for tool in tools['tools']:
                assert tool['inputSchema']['additionalProperties'] is False
                assert 'command' not in tool['inputSchema']['properties']
                assert all(type(tool['annotations'][k]) is bool for k in ('readOnlyHint', 'destructiveHint', 'openWorldHint'))
            body = {'command': 'job_submit', 'idempotency_key': 'one-edit', 'request': {'command': 'edit_preview', 'project': str(project), 'expect_revision': 0, 'output': str(root / 'edit.png'), 'edits': [{'type': 'upsert_node', 'node': {'id': 'light', 'op': {'type': 'exposure', 'stops': 0.2}}}, {'type': 'set_output', 'id': 'light'}]}}
            first = mcp.data(body)
            assert mcp.data(body)['job'] == first['job']
            result = mcp.done(first['job'])
            assert result['status'] == 'completed' and result['commit']['revision'] == 1, result
            resource = mcp.rpc('resources/read', {'uri': result['result']['preview']['resource_uri']})
            assert resource['contents'][0]['blob'].startswith('iVBOR')
            for _ in range(65):
                job = mcp.data({'command': 'job_submit', 'request': {'command': 'stats', 'project': str(project)}})['job']
                assert mcp.done(job)['status'] == 'completed'
            retry = mcp.data(body)
            assert retry['status'] == 'expired' and retry['commit']['revision'] == 1
            assert mcp.data({'command': 'project_info', 'project': str(project)})['revision'] == 1
            report['idempotent_edit_after_eviction'] = True
            legacy = mcp.call({'command': 'show', 'project': str(project)}, full=True)
            compact = mcp.call({'command': 'show', 'project': str(project)})
            report['full_show_response_bytes'] = len(json.dumps(legacy).encode())
            report['compact_show_response_bytes'] = len(json.dumps(compact).encode())
            viewer = mcp.data({'command': 'viewer_open', 'project': str(project)})
            reused = mcp.data({'command': 'viewer_open', 'project': str(project)})
            assert reused['pid'] == viewer['pid'] and reused['url'] == viewer['url'] and reused['reused']
            assert len(mcp.data({'command': 'viewer_status'})['viewers']) == 1
            assert mcp.data({'command': 'viewer_close', 'project': str(project)})['closed']
            assert not mcp.data({'command': 'viewer_status'})['viewers']
            report['managed_viewer_reuse_close'] = True
            mcp.data({'command': 'configure', 'cache_budget_mib': 512, 'idle_seconds': 2})
            if args.raw_project:
                raw = args.raw_project.resolve()
                original_hash = hashlib.sha256(raw.read_bytes()).hexdigest()
                active = mcp.data({'command': 'job_submit', 'request': {'command': 'preview', 'project': str(raw), 'output': str(root / 'cancel.png'), 'max_edge': 800}})['job']
                queued_body = json.loads(json.dumps(body))
                queued_body['idempotency_key'] = 'cancel-before-commit'
                queued_body['request']['expect_revision'] = 1
                queued_body['request']['output'] = str(root / 'cancel-edit.png')
                queued = mcp.data(queued_body)['job']
                start = time.monotonic()
                mcp.rpc('ping')
                report['ping_during_raw_ms'] = round((time.monotonic() - start) * 1000, 2)
                assert report['ping_during_raw_ms'] < 1000
                cancelled = mcp.data({'command': 'job_cancel', 'job': queued})
                assert cancelled['status'] == 'cancelled', cancelled
                mcp.data({'command': 'job_cancel', 'job': active})
                assert mcp.done(active)['status'] == 'cancelled'
                assert mcp.data({'command': 'project_info', 'project': str(project)})['revision'] == 1
                start = time.monotonic()
                job = mcp.data({'command': 'job_submit', 'request': {'command': 'preview', 'project': str(raw), 'output': str(root / 'raw.png'), 'max_edge': 800}})['job']
                done = mcp.done(job)
                assert done['status'] == 'completed', done
                assert 'analysis' not in done['result']
                report['raw_preview_seconds'] = round(time.monotonic() - start, 3)
                report['raw_cancel_and_queued_mutation_cancel'] = True
                report['cached_bytes_after_raw'] = mcp.data({'command': 'cache_info'})['job_cached_bytes']
                report['working_set_after_raw_bytes'] = working_set(mcp.proc.pid)
                assert report['cached_bytes_after_raw'] > 0
                time.sleep(3)
                report['cached_bytes_after_idle'] = mcp.data({'command': 'cache_info'})['job_cached_bytes']
                report['working_set_after_idle_bytes'] = working_set(mcp.proc.pid)
                assert report['cached_bytes_after_idle'] == 0
                assert hashlib.sha256(raw.read_bytes()).hexdigest() == original_hash
            report['passed'] = True
    finally:
        mcp.close()
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + '\n'
    if args.report:
        args.report.write_text(rendered, encoding='utf-8')
    print(rendered)


if __name__ == '__main__':
    main()
