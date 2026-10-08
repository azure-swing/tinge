"""Check real MCP contracts/package and optional host-generated selection traces.

No LLM/API calls are made. Golden prompts require replay in the target host;
without --traces this reports selection evaluation as not_run.
Trace JSON is a list of {id, calls:[{name, arguments}]} records.
"""
import argparse
import json
from pathlib import Path

import importlib.util

spec = importlib.util.spec_from_file_location('agent_acceptance', Path(__file__).with_name('agent-acceptance.py'))
agent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent)


def refs(value, root):
    if isinstance(value, dict):
        if '$ref' in value:
            assert value['$ref'].startswith('#/'), value['$ref']
            target = root
            for part in value['$ref'][2:].split('/'):
                target = target[part.replace('~1', '/').replace('~0', '~')]
        for item in value.values():
            refs(item, root)
    elif isinstance(value, list):
        for item in value:
            refs(item, root)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--package', type=Path)
    parser.add_argument('--traces', type=Path)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    client = agent.MCP(args.executable.resolve())
    try:
        catalog = client.rpc('tools/list')['tools']
        tools = {t['name']: t for t in catalog}
        assert len(tools) == len(catalog)
        for tool in catalog:
            schema = tool['inputSchema']
            assert schema['type'] == 'object' and schema['additionalProperties'] is False
            assert not {'command', 'request', 'jobs'} & schema['properties'].keys()
            assert tool['description'].strip()
            assert all(type(tool['annotations'][k]) is bool for k in ('readOnlyHint', 'destructiveHint', 'openWorldHint'))
            if tool['annotations']['readOnlyHint']:
                assert not tool['annotations']['destructiveHint']
            refs(schema, schema)
        for name in ('vibecolor_run', 'vibecolor_agent', 'vibecolor_job_submit', 'vibecolor_batch'):
            assert name not in tools
            response = client.rpc('tools/call', {'name': name, 'arguments': {'command': 'capabilities'}})
            assert response['isError'], response
        response = client.rpc('tools/call', {'name': 'vibecolor_capabilities', 'arguments': {'command': 'finalize'}})
        assert response['isError'], response
        fixtures = json.loads((Path(__file__).resolve().parents[1] / 'tests/plugin-prompts.json').read_text(encoding='utf-8'))['cases']
        for case in fixtures:
            assert all(name in tools for name in case['expected_tools'] + case.get('forbidden_tools', [])), case['id']
        report = {'named_tools': len(tools), 'contract': 'passed', 'prompt_cases': len(fixtures), 'selection_evaluation': 'not_run'}
        if args.package:
            root = args.package.resolve()
            manifest = json.loads((root / '.codex-plugin/plugin.json').read_text(encoding='utf-8'))
            assert manifest['name'] == 'tinge'
            assert manifest['interface']['displayName'] == 'Tinge'
            assert manifest['version'] == client.data({'command': 'capabilities'})['version']
            wiring = json.loads((root / manifest['mcpServers']).read_text(encoding='utf-8'))['mcpServers']['tinge']
            executable = Path(wiring['command'].replace('${PLUGIN_ROOT}', str(root)))
            assert executable.is_file() and wiring['args'] == ['mcp']
            skill = root / manifest['skills'] / 'photo-workflow/SKILL.md'
            assert skill.is_file()
            import re
            for link in re.findall(r'\]\((references/[^)]+)\)', skill.read_text(encoding='utf-8')):
                assert (skill.parent / link).is_file(), link
            assert (root / 'LICENSE').is_file() and (root / 'THIRD_PARTY.md').is_file()
            report['package'] = 'passed'
        if args.traces:
            traces = {r['id']: r['calls'] for r in json.loads(args.traces.read_text(encoding='utf-8'))}
            assert set(traces) == {case['id'] for case in fixtures}, 'Replay every case'
            for case in fixtures:
                calls = [call for call in traces[case['id']] if call['name'].startswith('vibecolor_')]
                if not case['expected_tools']:
                    assert not calls, case['id']
                for group in case.get('required_any_groups', []):
                    assert any(call['name'] in group for call in calls), (case['id'], group)
                for call in calls:
                    assert call['name'] in tools and call['name'] not in case.get('forbidden_tools', []), case['id']
                    if case.get('read_only_only'):
                        assert tools[call['name']]['annotations']['readOnlyHint'], case['id']
                    if call['name'] in case['expected_tools']:
                        for key, value in case.get('required_arguments', {}).items():
                            assert call['arguments'].get(key) == value, (case['id'], key)
            report['selection_evaluation'] = 'passed_trace_checks; manually review visual quality, questions and receipts'
        print(json.dumps(report, ensure_ascii=False, indent=2))
        if args.report:
            args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    finally:
        client.close()


if __name__ == '__main__':
    main()
