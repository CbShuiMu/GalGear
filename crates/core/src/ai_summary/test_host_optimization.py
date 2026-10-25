"""Offline integration checks for concrete resume and optimization edge cases."""
import hashlib
import importlib.util
import json
import os
import pathlib
import struct
import tempfile
import unittest
from unittest import mock
from test_agent_host import review_narration


spec = importlib.util.spec_from_file_location('optimization_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class HostOptimizationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'fixture.exe').write_bytes(b'fixture')
        (self.root / 'scene.txt').write_bytes(b'Current')
        self.skill, self.saves = self.root / 'skill', self.root / 'saves'
        self.skill.mkdir()
        self.saves.mkdir()
        for name in ('one.sav', 'two.sav'):
            (self.saves / name).write_bytes(b'Current|scene.txt' + struct.pack('<I', 0))
        self.config = dict(game=str(self.root), save=str(self.saves), skill=str(self.skill),
                           work=str(self.root / 'work'), title='Fixture', language='en-US',
                           provider='deepseek', model='fixture', max_turns=150,
                           required_saves=[str(self.saves / 'two.sav')])
        self.serial = 0

    def agent(self):
        agent = host.Agent(self.config)
        agent.dispatch('record_save_preflight', dict(path='two.sav', classification='progress_candidate',
            explanation='The optimization fixture already has a native current caption.',
            observations=[dict(role='current_caption', value='Current',
                               proof=dict(offset=0, length=7, encoding='ascii'))]))
        agent.audit_messages = []
        agent.context_system = 'Use checked artifacts and actual tool evidence.'
        agent.context_task = json.dumps(dict(task='Fixture optimization'))
        return agent

    def call(self, agent, name, args):
        self.serial += 1
        reply = dict(role='assistant', content='', reasoning_content='fixture', tool_calls=[dict(
            id='optimization-call-' + str(self.serial), type='function',
            function=dict(name=name, arguments=json.dumps(args)))])
        messages = []
        with mock.patch.object(agent, 'request_completion', return_value=reply):
            agent.run_turn(messages, host.TOOLS, self.serial, 'prepare')
        return json.loads(messages[-1]['content'])

    def prepare(self):
        agent = self.agent()
        agent.read_skill = agent.read_contract = agent.used_skill = True
        script = 'def extract(data, metadata): return [{"position":0,"text":data.decode("ascii"),"speaker":None,"evidence":{"offset":0,"length":len(data),"encoding":"ascii"}}]'
        save = '''import struct
def extract(data, metadata):
 separator = data.index(b'|')
 position_offset = len(data) - 4
 return {"source":data[separator+1:position_offset].decode("ascii"),
  "line":struct.unpack_from("<I",data,position_offset)[0],
  "anchors":[data[:separator].decode("ascii")],
  "evidence":{"anchor":{"offset":0,"length":separator,"encoding":"ascii"},
   "source":{"offset":separator+1,"length":position_offset-separator-1,"encoding":"ascii"},
   "position":{"offset":position_offset,"width":4,"endian":"little"}}}
'''
        self.call(agent, 'read_bytes', dict(root='game', path='scene.txt', offset=0, length=7))
        agent.dispatch('write_program', dict(kind='script', source=script))
        agent.dispatch('test_program', dict(kind='script', root='game', path='scene.txt'))
        agent.dispatch('write_program', dict(kind='save', source=save))
        for name in ('one.sav', 'two.sav'):
            agent.dispatch('test_program', dict(kind='save', root='save', path=name))
        review_narration(agent)
        self.assertTrue(agent.preparation_status()['ready_for_finish'])
        return agent, script, save

    def gate(self, agent):
        agent.work_guidance()
        agent.workflow.inspection_turns = set(range(1, 9))
        self.assertTrue(agent.work_guidance()['exploration_gated'])

    def test_same_source_write_preserves_validated_artifacts(self):
        agent, script, save = self.prepare()
        before = (dict(agent.tests), dict(agent.parts), agent.artifact_hashes())
        for kind, source in (('script', script), ('save', save)):
            result = agent.dispatch('write_program', dict(kind=kind, source=source))
            self.assertTrue(result['reused'])
            self.assertEqual((agent.tests, agent.parts, agent.artifact_hashes()), before)
        self.assertTrue(agent.preparation_status()['ready_for_finish'])

    def test_same_source_write_does_not_count_as_a_new_exploration_milestone(self):
        agent = self.agent()
        agent.read_skill = agent.read_contract = agent.used_skill = True
        script = 'def extract(data, metadata): return []'
        agent.dispatch('write_program', dict(kind='script', source=script))
        self.gate(agent)
        result = self.call(agent, 'write_program', dict(kind='script', source=script))
        self.assertTrue(result['reused'])
        self.assertTrue(agent.work_guidance()['exploration_gated'])

    def test_empty_batch_cannot_reset_exploration_or_claim_recent_verification(self):
        agent = self.agent()
        agent.read_skill = agent.read_contract = agent.used_skill = True
        script = 'def extract(data, metadata): return [{"position":0,"text":data.decode("ascii"),"speaker":None,"evidence":{"offset":0,"length":len(data),"encoding":"ascii"}}]'
        agent.dispatch('write_program', dict(kind='script', source=script))
        agent.dispatch('test_program', dict(kind='script', root='game', path='scene.txt'))
        agent.dispatch('write_program', dict(kind='save', source='def extract(data, metadata): return {}'))
        # This is a resumed nonempty transcript; no test has made new progress in
        # the current workflow session.
        self.assertIsNone(agent.workflow.verified_at)
        (agent.work / 'empty').mkdir()
        self.gate(agent)
        result = self.call(agent, 'test_program_batch', dict(root='work', path='empty', filter='*.txt'))
        self.assertIn('GLOB matched no files', result['error'])
        self.assertNotIn('files', result)
        self.assertIsNone(agent.workflow.verified_at)
        self.assertTrue(agent.work_guidance()['exploration_gated'])
        self.assertFalse(agent.workflow.extension_allowed(agent.requests_sent, agent.programs, agent.preparation_status()))

    def test_changed_script_hash_invalidates_ready_artifacts_even_with_same_mtime(self):
        agent, _, _ = self.prepare()
        agent.checkpoint([])
        checkpoint = json.loads((agent.work / 'checkpoint.json').read_text('utf-8'))
        path = self.root / 'scene.txt'
        info = path.stat()
        path.write_bytes(b'Changed')
        os.utime(path, ns=(info.st_atime_ns, info.st_mtime_ns))
        restored = host.Agent(self.config)
        restored.audit_messages = list(agent.audit_messages)
        restored.programs = dict(agent.programs)
        restored.restore_checkpoint(checkpoint)
        self.assertTrue(restored.research.resolve('O1')['stale'])
        self.assertFalse(restored.preparation_status()['ready_for_finish'])
        self.assertEqual(restored.parts, {})
        self.assertEqual(restored.tests, {})
        # After re-extraction and validation, historical stale observations must
        # not invalidate the newly grounded artifacts on every later retry.
        self.assertEqual(set(restored.programs), {'script', 'save'})
        restored.used_skill = True
        restored.dispatch('test_program', dict(kind='script', root='game', path='scene.txt'))
        for name in ('one.sav', 'two.sav'):
            (self.saves / name).write_bytes(b'Changed|scene.txt' + struct.pack('<I', 0))
            restored.dispatch('test_program', dict(kind='save', root='save', path=name))
        review_narration(restored)
        self.assertTrue(restored.preparation_status()['ready_for_finish'])
        restored.checkpoint([])
        current_checkpoint = json.loads((restored.work / 'checkpoint.json').read_text('utf-8'))
        again = host.Agent(self.config)
        again.audit_messages = list(restored.audit_messages)
        again.programs = dict(restored.programs)
        self.assertTrue(again.restore_checkpoint(current_checkpoint))
        self.assertTrue(again.preparation_status()['ready_for_finish'])

    def test_json_projection_and_pagination_keep_only_requested_fields(self):
        agent = self.agent()
        path = agent.work / 'structure.json'
        path.write_text(json.dumps({'tables': [{'offset': index, 'text': 'UNNEEDED GAME TEXT'}
                                              for index in range(12)]}), encoding='utf-8')
        result = self.call(agent, 'read_json', dict(root='work', path='structure.json', pointer='/tables',
                                                 fields=['offset'], offset=4, limit=3))
        self.assertEqual(result['items'], [{'offset': 4}, {'offset': 5}, {'offset': 6}])
        self.assertEqual(result['next_offset'], 7)
        self.assertNotIn('UNNEEDED GAME TEXT', json.dumps(result))
        self.assertEqual(result['source_context']['sha256'], hashlib.sha256(path.read_bytes()).hexdigest())

    def test_previous_session_files_are_archived_before_new_research_persistence(self):
        work = self.root / 'work'
        work.mkdir()
        previous = dict(session_id='previous-run', status='failed', requests_sent=50, closed=True)
        (work / 'session.json').write_text(json.dumps(previous), encoding='utf-8')
        old_files = {
            'conversation.json': [dict(role='system', content='previous system')],
            'events.json': [dict(turn=49, tool='inspect_data')],
            'checkpoint.json': dict(analysis_notes={'old': 'keep'}),
            'analysis-notes.json': dict(findings={}, next_steps=['Resume the current-save parser.'])}
        for name, value in old_files.items():
            (work / name).write_text(json.dumps(value), encoding='utf-8')
        agent = host.Agent(self.config)
        archived = work / 'sessions' / 'previous-run'
        self.assertNotEqual(agent.session_dir, archived)
        self.assertEqual(json.loads((archived / 'session.json').read_text('utf-8')), previous)
        for name, value in old_files.items():
            self.assertEqual(json.loads((archived / name).read_text('utf-8')), value)

    def test_archive_selection_change_invalidates_derived_state_but_preserves_programs(self):
        agent, _, _ = self.prepare()
        for name in ('original.noa', 'translated.noa'):
            (self.root / name).write_bytes(b'archive fixture')
        (self.root / 'resources.xml').write_text(
            '<resources><archive>original.noa</archive><archive>translated.noa</archive></resources>',
            encoding='utf-8')
        agent.config['inventory'] = [dict(path=str(self.root / name))
                                     for name in ('original.noa', 'translated.noa')]
        agent.script_resources = {'display_names': {'count': 2}}
        original = dict(script_name='script.csx', selected='original.noa',
                        priority='first', configuration='resources.xml')
        translated = dict(original, selected='translated.noa', priority='last')
        before = dict(parts=dict(agent.parts), tests=dict(agent.tests), resources=dict(agent.script_resources),
                      programs=dict(agent.programs), artifacts=agent.artifact_hashes())
        original_dispatch = agent.dispatch

        def with_fake_archive_listing(name, args):
            if name == 'run_skill':
                return dict(exit_code=0, stdout='[00000010] 7 script.csx\n', stderr='')
            return original_dispatch(name, args)

        with mock.patch.object(agent, 'dispatch', side_effect=with_fake_archive_listing):
            agent.verify_resource_selection(original)
            agent.verify_resource_selection(original)
            self.assertEqual(agent.parts, before['parts'])
            self.assertEqual(agent.tests, before['tests'])
            self.assertEqual(agent.script_resources, before['resources'])
            agent.verify_resource_selection(translated)
        self.assertEqual(agent.parts, {})
        self.assertEqual(agent.tests, {})
        self.assertEqual(agent.script_resources, {})
        self.assertEqual(agent.programs, before['programs'])
        self.assertEqual(agent.artifact_hashes(), before['artifacts'])
        self.assertEqual(agent.resource_selections['script.csx']['selected'], str(self.root / 'translated.noa'))
        self.assertFalse(agent.preparation_status()['ready_for_finish'])

    def test_catalog_indexed_policy_rejects_direct_spans_accepts_operands_and_keeps_plain_scripts(self):
        # A synthetic TLV format exercises the declarative policy through actual
        # host dispatch, without adding a game parser or mocking verification.
        header = b'IDXFIX01'
        code = struct.pack('<I', 2)
        strings = [b'', b'Prior', b'Current']
        pool = struct.pack('<I', len(strings)) + b''.join(
            struct.pack('<I', len(text)) + text for text in strings)
        container = header + struct.pack('<4sI', b'CODE', len(code)) + code
        region = dict(offset=len(container) + 8, length=len(pool))
        container += struct.pack('<4sI', b'STRS', len(pool)) + pool
        (self.root / 'indexed.bin').write_bytes(container)
        reference = self.skill / 'reference'
        reference.mkdir()
        catalog = dict(formats=[dict(id='synthetic-indexed', verification_policy=dict(
            header=dict(offset=0, ascii='IDXFIX01'),
            chunks=dict(offset=8, tag_width=4, length_width=4, endian='little',
                        tag_ascii='STRS', max_chunks=8)))])
        (reference / 'format-index.json').write_text(json.dumps(catalog), encoding='utf-8')
        prefix = '''import struct
def extract(data, metadata):
 position = 8
 while position < len(data):
  tag, length = struct.unpack_from('<4sI', data, position)
  body = position + 8
  if tag == b'CODE': code = body
  if tag == b'STRS': table, table_length = body, length
  position = body + length
 count = struct.unpack_from('<I', data, table)[0]
 position = table + 4
 records = []
 for index in range(count):
  length = struct.unpack_from('<I', data, position)[0]
  position += 4
  records.append({'offset':position,'length':length,'text':data[position:position+length].decode('ascii')})
  position += length
'''
        direct = prefix + ''' return [{'position':row['offset'],'text':row['text'],'speaker':None,'evidence':{'offset':row['offset'],'length':row['length'],'encoding':'ascii'}} for row in records if row['text']]
'''
        indexed = prefix + ''' index = struct.unpack_from('<I', data, code)[0]
 return {'rows':[{'position':0,'text':records[index]['text'],'speaker':None,'evidence':{'table':'strings','index':index,'reference':{'base':code,'operand_offset':0,'width':4,'endian':'little'}}}], 'tables':{'strings':{'offset':table,'length':table_length,'count_width':4,'endian':'little','fields':[{'kind':'text','length_width':4,'encoding':'ascii'}]}}}
'''
        agent = self.agent()
        agent.read_skill = agent.read_contract = agent.used_skill = True
        # The old generic span check accepted this output, even though it has
        # no instruction binding and changes the coordinate to a text offset.
        old_output = host.run_parser(direct, container, {})
        self.assertEqual([row['text'] for row in host.EVIDENCE['script_rows'](container, old_output)],
                         ['Prior', 'Current'])
        agent.dispatch('write_program', dict(kind='script', source=direct))
        rejected = self.call(agent, 'test_program', dict(kind='script', root='game', path='indexed.bin'))
        self.assertIn('Known indexed script regions', rejected['error'])
        self.assertIn('verification_contract TOOL', rejected['error'])
        self.assertEqual(json.loads(rejected['error'].split('Required original table regions: ', 1)[1]), [region])
        self.assertIn('operand', rejected['verification_help']['script'])
        self.assertEqual(agent.dialogue(), [])
        agent.dispatch('write_program', dict(kind='script', source=indexed))
        accepted = self.call(agent, 'test_program', dict(kind='script', root='game', path='indexed.bin'))
        self.assertNotIn('error', accepted)
        self.assertEqual(accepted['count'], 1)
        self.assertEqual([(row['position'], row['text']) for row in agent.dialogue()], [(0, 'Current')])
        plain = 'def extract(data, metadata): return [{"position":0,"text":data.decode("ascii"),"speaker":None,"evidence":{"offset":0,"length":len(data),"encoding":"ascii"}}]'
        agent.dispatch('write_program', dict(kind='script', source=plain))
        ordinary = self.call(agent, 'test_program', dict(kind='script', root='game', path='scene.txt'))
        self.assertNotIn('error', ordinary)
        self.assertEqual(ordinary['count'], 1)
        self.assertEqual(agent.dialogue()[0]['source'], 'scene.txt')


if __name__ == '__main__':
    unittest.main()
