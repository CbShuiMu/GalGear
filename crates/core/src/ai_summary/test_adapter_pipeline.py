"""Verify persisted native coordinates survive the same worker path as runtime."""
import json
import pathlib
import runpy
import tempfile
import unittest

import test_adapter_evidence as fixtures

host = runpy.run_path(str(pathlib.Path(__file__).with_name('agent_host.py')))


class AdapterPipelineTests(unittest.TestCase):
    def test_verified_script_coordinates_persist_and_resolve_in_runtime_worker(self):
        data, rows = fixtures.AdapterEvidenceTests().fixture()
        strings = fixtures.u32(8)
        for index in range(8):
            text = b'Current' if index == 7 else b''
            strings += fixtures.u32(len(text)) + text
        table = dict(offset=len(data), length=len(strings), fields=[
            dict(kind='text', length_width=4, unit=1, encoding='utf-8')])
        data += strings
        output = dict(rows=rows, tables=dict(strings=table))
        script = 'import json\ndef extract(data, metadata):\n    return json.loads(' + repr(json.dumps(output)) + ')\n'
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            game, save, skill = (root / name for name in ('game', 'save', 'skill'))
            for path in (game, save, skill):
                path.mkdir()
            (game / 'fixture.exe').write_bytes(b'fixture')
            (game / 'chapter.script').write_bytes(data)
            agent = host['Agent'](dict(game=str(game), save=str(save), skill=str(skill),
                                       work=str(game / 'work'), provider='deepseek',
                                       model='fixture', token='fixture', title='Fixture', language='zh-CN',
                                       model_capabilities=dict(status='unavailable', kind='offline_fixture')))
            agent.read_contract = True
            agent.programs['script'] = script
            (agent.work / 'script_extractor.py').write_text(script, encoding='utf-8')
            result = agent.dispatch('test_program', dict(kind='script', root='game', path='chapter.script'))
            self.assertEqual(result['count'], 3)
            dialogue = [json.loads(line) for line in (agent.work / 'dialogue.jsonl').read_text(encoding='utf-8').splitlines()]
            self.assertEqual(dialogue[0]['_native'][0]['value'], 7)
            raw = b'Current|function_a|' + fixtures.u32(7)
            facts = dict(source='chapter.script', line=0, anchors=['Current'], evidence=dict(
                anchor=dict(offset=0, length=7, encoding='ascii'),
                node_link=dict(kind='call_return', scope=dict(offset=8, length=10, encoding='ascii'),
                               position=dict(offset=19, width=4, endian='little'))))
            parser = 'import json\ndef extract(data, metadata):\n    return json.loads(' + repr(json.dumps(facts)) + ')\n'
            verified = host['run_parser'](parser, raw, dict(dialogue=dialogue), verify_save=True)
            self.assertEqual((verified['source'], verified['line']), ('chapter.script', 0))


if __name__ == '__main__':
    unittest.main()
