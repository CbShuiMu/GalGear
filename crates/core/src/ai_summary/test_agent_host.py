"""Tests for generic tool boundaries; contains no game-format decoder."""
import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('agent_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


def review_narration(agent):
    """A fixture's only script is narration; perform the actual inventory gate."""
    arguments = dict(root='game', path='.')
    result = agent.dispatch('list_files', arguments)
    record = agent.research.observe('narration-inventory-' + str(len(agent.research.records)),
        dict(name='list_files', arguments=arguments), result, inputs=agent.input_description('list_files', arguments))
    agent.dispatch('review_speakers', dict(reason='The complete fixture inventory contains no character/name resources; the script is narration.',
                                          evidence_refs=[record['ref']]))


class ParserBoundaryTests(unittest.TestCase):
    def test_data_only_parser_runs_on_supplied_bytes(self):
        result = host.run_parser('import struct\ndef extract(data, metadata):\n return {"value":struct.unpack("<I",data)[0],"name":metadata["name"]}', b'\x07\0\0\0', {'name': 'sample'})
        self.assertEqual(result, {'value': 7, 'name': 'sample'})

    def test_files_network_and_import_escape_are_rejected(self):
        for source in ('import os', 'from re import __builtins__ as b', 'from json import scanner', 'x = open("secret")', 'x = ().__class__', 'x = getattr(1,"real")', 'x = "{0.__globals__}".format(print)'):
            with self.subTest(source=source), self.assertRaises(ValueError):
                host.validate_program(source)

    def test_paths_cannot_escape_named_roots(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'game.exe').write_bytes(b'fixture')
            agent = host.Agent(dict(game=temp, save=temp, skill=temp, work=temp))
            with self.assertRaises(ValueError):
                agent.path('game', '../elsewhere')
            with self.assertRaises(ValueError):
                agent.path('game', str(root))

    def test_empty_catalog_folder_is_rejected_before_preparation(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(ValueError):
                host.Agent(dict(game=temp, save=temp, skill=temp, work=temp))

    def test_negative_control_samples_do_not_block_two_grounded_progress_saves(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'game.exe').write_bytes(b'fixture')
            agent = host.Agent(dict(game=temp, save=temp, skill=temp, work=temp, provider='deepseek', model='test', language='en-US', save_samples=2))
            agent.read_skill = agent.read_contract = agent.used_skill = True
            agent.programs['save'] = 'def extract(data, metadata): return {}'
            agent.programs['script'] = 'def extract(data, metadata): return []'
            agent.parts['script'] = [{'source': 'sample', 'position': 1, 'text': 'Current', 'speaker': None}]
            for name in ('save_extractor.py', 'script_extractor.py', 'dialogue.jsonl'):
                (root / name).write_text('fixture', encoding='utf-8')
            for name in ('one', 'two', 'control'):
                path = root / name
                path.write_bytes(b'fixture')
                agent.samples[str(path)] = 'control' if name == 'control' else 'progress'
                agent.tests[str(path)] = dict(matching_anchors=[] if name == 'control' else ['Current'], sha256=host.hashlib.sha256(b'fixture').hexdigest(), save_verification_version=host.SAVE_VERIFICATION_VERSION)
            review_narration(agent)
            result = agent.dispatch('finish', dict(engine='fixture', script_language='en-US', speaker_resolution='unavailable', notes='Generic test'))
            self.assertEqual(set(result['validated_saves']), {str(root / 'one'), str(root / 'two')})
            self.assertEqual(result['rejected_samples'], [str(root / 'control')])

    def test_companion_name_table_is_parsed_and_speakers_survive_jsonl(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'game.exe').write_bytes(b'fixture')
            (root / 'names.json').write_text('{"7":"Alice"}', encoding='utf-8')
            (root / 'scene.txt').write_bytes(b'7|Alice|Hello\n0||Narration')
            agent = host.Agent(dict(game=temp, save=temp, skill=temp, work=temp, language='en-US'))
            load = dict(root='game', path='names.json', key='names', source='import json\ndef extract(data, metadata): return json.loads(data.decode("utf-8"))')
            agent.dispatch('load_script_metadata', load)
            agent.dispatch('verification_contract', {})
            agent.dispatch('write_program', dict(kind='script', source='def extract(data, metadata):\n rows = []\n offset = 0\n for line in data.decode("utf-8").splitlines():\n  who, shown, text = line.split("|")\n  position = offset + len(who) + len(shown) + 2\n  proof = {"offset":position,"length":len(text),"encoding":"utf-8"}\n  if shown: proof["speaker"] = {"offset":offset+len(who)+1,"length":len(shown),"encoding":"utf-8"}\n  rows.append({"position":position,"text":text,"speaker":metadata["resources"]["names"].get(who),"evidence":proof})\n  offset += len(line) + 1\n return rows'))
            agent.dispatch('test_program', dict(kind='script', root='game', path='scene.txt'))
            rows = [json.loads(line) for line in (root / 'dialogue.jsonl').read_text(encoding='utf-8').splitlines()]
            self.assertEqual([row['speaker'] for row in rows], ['Alice', None])
            self.assertIn(str(root / 'names.json'), agent.sources)
            # Changed companion resources must invalidate rows and save validations.
            agent.tests['old-save'] = {'matching_anchors': ['Hello']}
            (root / 'names.json').write_text('{"7":"Beth"}', encoding='utf-8')
            agent.dispatch('load_script_metadata', load)
            self.assertEqual(agent.dialogue(), [])
            self.assertEqual(agent.tests, {})
            (root / 'scene.txt').write_bytes(b'7|Beth|Hello\n0||Narration')
            agent.dispatch('test_program', dict(kind='script', root='game', path='scene.txt'))
            self.assertEqual(agent.dialogue()[0]['speaker'], 'Beth')

    def test_external_only_name_is_verified_preserved_and_classified_on_finish(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'game.exe').write_bytes(b'fixture')
            (root / 'names.bin').write_bytes(b'\x07\0Alice')
            (root / 'scene.txt').write_bytes(b'\x07\0Hello')
            agent = host.Agent(dict(game=temp, save=temp, skill=temp, work=str(root / 'work'),
                language='en-US', provider='fixture', model='fixture'))
            agent.read_skill = agent.read_contract = agent.used_skill = True
            load = dict(root='game', path='names.bin', key='names', source='''def extract(data, metadata):
 return {'values': {'7':data[2:].decode('ascii')},'speakers': {'alice': {'id':int.from_bytes(data[:2],'little'),'text':data[2:].decode('ascii'),'evidence':{'offset':2,'length':len(data)-2,'encoding':'ascii'},'id_evidence':{'offset':0,'width':2}}}}''')
            agent.dispatch('load_script_metadata', load)
            agent.dispatch('write_program', dict(kind='script', source='''def extract(data, metadata):
 return [{'position':2,'text':data[2:].decode('ascii'),'speaker':metadata['speaker_resources']['names']['alice']['text'],'evidence':{'offset':2,'length':len(data)-2,'encoding':'ascii','speaker':{'resource':'names','entry':'alice','reference':{'offset':0,'width':2}}}}]'''))
            agent.dispatch('test_program', dict(kind='script', root='game', path='scene.txt'))
            agent.dispatch('write_program', dict(kind='save', source='''def extract(data, metadata):
 return {'source':'scene.txt','line':2,'anchors':[data[:5].decode('ascii')],'evidence':{'anchor':{'offset':0,'length':5,'encoding':'ascii'},'source':{'offset':6,'length':9,'encoding':'ascii'},'position':{'offset':15,'width':4,'endian':'little'}}}'''))
            for name in ('one.sav', 'two.sav'):
                (root / name).write_bytes(b'Hello|scene.txt' + (2).to_bytes(4, 'little'))
                agent.dispatch('test_program', dict(kind='save', root='save', path=name))
            before = (dict(agent.parts), dict(agent.tests))
            self.assertTrue(agent.dispatch('load_script_metadata', load)['reused'])
            self.assertEqual((agent.parts, agent.tests), before)
            result = agent.dispatch('finish', dict(engine='fixture', script_language='en-US', speaker_resolution='external', notes='Byte verified companion name.'))
            self.assertEqual(result['speaker_resolution'], 'external')
            self.assertEqual(result['speaker_verification']['version'], 1)
            self.assertEqual(result['speaker_verification']['external_rows'], 1)
            self.assertEqual(json.loads((agent.work / 'dialogue.jsonl').read_text('utf-8'))['speaker'], 'Alice')
            agent.checkpoint([])
            checkpoint = json.loads((agent.work / 'checkpoint.json').read_text('utf-8'))
            original = (root / 'names.bin').stat()
            (root / 'names.bin').write_bytes(b'\x07\0Other')
            os.utime(root / 'names.bin', ns=(original.st_atime_ns, original.st_mtime_ns))
            with self.assertRaisesRegex(ValueError, 'resource changed'):
                agent.dispatch('test_program', dict(kind='save', root='save', path='one.sav'))
            restored = host.Agent(agent.config)
            restored.programs = dict(agent.programs)
            self.assertFalse(restored.restore_checkpoint(checkpoint))
            self.assertEqual(restored.dialogue(), [])

    def test_all_null_speakers_cannot_waive_verified_companion_names(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'game.exe').write_bytes(b'fixture')
            (root / 'names.bin').write_bytes(b'\x07\0Alice')
            (root / 'scene.txt').write_bytes(b'Hello')
            agent = host.Agent(dict(game=temp, save=temp, skill=temp, work=str(root / 'work'), language='en-US'))
            agent.dispatch('load_script_metadata', dict(root='game', path='names.bin', key='names', source='''def extract(data, metadata):
 return {'values':{},'speakers':{'a':{'id':7,'text':data[2:].decode('ascii'),'evidence':{'offset':2,'length':5,'encoding':'ascii'},'id_evidence':{'offset':0,'width':2}}}}'''))
            agent.parts[str(root / 'scene.txt')] = [dict(source='scene.txt', position=0, text='Hello', speaker=None)]
            self.assertTrue(agent.preparation_status()['speaker_review_required'])
            with self.assertRaisesRegex(ValueError, 'cannot be waived'):
                agent.dispatch('review_speakers', dict(reason='No speaker in translated file.', evidence_refs=['latest']))

    def test_speaker_unavailability_requires_current_inventory_evidence(self):
        fixture = PreparationLifecycleTests('run')
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        fixture.agent.speaker_review = None
        self.assertTrue(fixture.agent.preparation_status()['speaker_review_required'])
        with self.assertRaisesRegex(ValueError, 'host-stamped'):
            fixture.agent.dispatch('review_speakers', dict(reason='No names.', evidence_refs=['invented']))
        review_narration(fixture.agent)
        for name in ('one.sav', 'two.sav'):
            fixture.validate_save(name)
        self.assertTrue(fixture.agent.preparation_status()['ready_for_finish'])
        fixture.agent.checkpoint([])
        checkpoint = json.loads((fixture.agent.work / 'checkpoint.json').read_text('utf-8'))
        checkpoint.pop('speaker_review')
        restored = host.Agent(fixture.config)
        restored.programs = dict(fixture.agent.programs)
        self.assertTrue(restored.restore_checkpoint(checkpoint))
        self.assertTrue(restored.preparation_status()['speaker_review_required'])
        self.assertFalse(restored.preparation_status()['ready_for_finish'])
        self.assertEqual(len(restored.tests), 2)

    def test_missing_or_invalid_speaker_fields_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'game.exe').write_bytes(b'fixture')
            (root / 'scene.txt').write_bytes(b'fixture')
            agent = host.Agent(dict(game=temp, save=temp, skill=temp, work=temp, language='en-US'))
            for field in ('', ',"speaker":7', ',"speaker":" "'):
                with self.subTest(field=field):
                    agent.programs['script'] = 'def extract(data, metadata): return [{"position":0,"text":"Hello"' + field + '}]'
                    with self.assertRaises(ValueError):
                        agent.dispatch('test_program', dict(kind='script', root='game', path='scene.txt'))

    def test_save_validation_requires_the_exact_current_source_position_and_anchor(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'game.exe').write_bytes(b'fixture')
            (root / 'slot.sav').write_bytes(b'Currentscene' + (10).to_bytes(4, 'little'))
            agent = host.Agent(dict(game=temp, save=temp, skill=temp, work=temp, language='en-US'))
            agent.parts['script'] = [dict(source='scene', position=10, text='Current', speaker=None), dict(source='scene', position=20, text='Future', speaker=None)]
            for source, line in [('other', 10), ('scene', 20)]:
                agent.programs['save'] = 'def extract(data, metadata): return ' + repr(dict(source=source, line=line, anchors=['Current'], evidence=dict(anchor=dict(offset=0,length=7,encoding='utf-8'), source=dict(offset=7,length=5,encoding='utf-8'), position=dict(offset=12,width=4,endian='little'))))
                with self.subTest(source=source, line=line), self.assertRaises((ValueError, RuntimeError)):
                    agent.dispatch('test_program', dict(kind='save', root='save', path='slot.sav'))
            agent.programs['save'] = 'def extract(data, metadata): return {"source":"scene","line":10,"anchors":["Current"],"evidence":{"anchor":{"offset":0,"length":7,"encoding":"utf-8"},"source":{"offset":7,"length":5,"encoding":"utf-8"},"position":{"offset":12,"width":4,"endian":"little"}}}'
            result = agent.dispatch('test_program', dict(kind='save', root='game', path='slot.sav'))
            self.assertEqual(result['matching_anchors'], ['Current'])
            self.assertNotIn(str(root / 'slot.sav'), agent.sources)
            agent.programs['save'] = 'def extract(data, metadata): return {"anchors":["Current"]}'
            with self.assertRaises((ValueError, RuntimeError)):
                agent.dispatch('test_program', dict(kind='save', root='save', path='slot.sav'))
            self.assertNotIn(str(root / 'slot.sav'), agent.tests)

    def test_old_results_and_checkpoints_cannot_skip_speaker_reextraction(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'game.exe').write_bytes(b'fixture')
            host.atomic_json(root / 'conversation.json', [{'role': 'system', 'content': 'Old prompt'}])
            host.atomic_json(root / 'checkpoint.json', dict(parts={'old': [dict(source='scene', position=1, text='No speaker')]}, tests={'old-save': {'matching_anchors': ['No speaker']}}, read_skill=True, used_skill=True, sources=[]))
            host.atomic_json(root / 'result.json', dict(language='en-US', artifacts={}, source_stamps={}))
            agent = host.Agent(dict(game=temp, save=temp, skill=temp, work=temp, title='Fixture', language='en-US', provider='deepseek', max_turns=0))
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                agent.run()
            self.assertEqual(agent.dialogue(), [])
            self.assertEqual(agent.tests, {})
            self.assertFalse((root / 'result.json').exists())


class PreparationLifecycleTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.scene = self.root / 'scene.txt'
        self.scene.write_text('Current', encoding='utf-8')
        self.saves = self.root / 'saves'
        self.saves.mkdir()
        for name in ('one.sav', 'two.sav'):
            (self.saves / name).write_bytes(b'Current|first|scene.txt' + (0).to_bytes(4, 'little'))
        self.config = dict(game=str(self.root), save=str(self.saves), skill=str(self.root),
                           work=str(self.root / 'work'), title='Fixture', language='en-US',
                           provider='deepseek', model='test', max_turns=4, save_samples=2)
        self.agent = host.Agent(self.config)
        for name in ('one.sav', 'two.sav'):
            self.agent.dispatch('record_save_preflight', dict(path=name, classification='progress_candidate',
                explanation='Fixture current-caption bytes are available before transcript work.',
                observations=[dict(role='current_caption', value='Current',
                                   proof=dict(offset=0, length=7, encoding='utf-8'))]))
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
        row = dict(source='scene.txt', position=0, text='Current', speaker=None)
        self.agent.parts[str(self.scene)] = [row]
        self.agent.sources.add(str(self.scene))
        self.agent.programs = dict(
            save='def extract(data, metadata): return {"source":"scene.txt","line":0,"anchors":["Current"],"evidence":{"anchor":{"offset":0,"length":7,"encoding":"utf-8"},"source":{"offset":14,"length":9,"encoding":"utf-8"},"position":{"offset":23,"width":4,"endian":"little"}}}',
            script='def extract(data, metadata): return [{"position":0,"text":"Current","speaker":None,"evidence":{"offset":0,"length":7,"encoding":"utf-8"}}]')
        for kind, source in self.agent.programs.items():
            (self.agent.work / (kind + '_extractor.py')).write_text(source, encoding='utf-8')
        (self.agent.work / 'dialogue.jsonl').write_text(json.dumps(row) + '\n', encoding='utf-8')
        self.finish = dict(engine='Fixture', script_language='en-US',
                           speaker_resolution='unavailable', notes='Narration fixture; no speaker table.')
        review_narration(self.agent)

    def validate_save(self, name):
        self.agent.dispatch('test_program', dict(kind='save', root='save', path=name))

    def reply(self, name, args):
        return dict(role='assistant', content=None, reasoning_content='fixture',
                    tool_calls=[dict(id='fixture-' + name, type='function',
                                     function=dict(name=name, arguments=json.dumps(args)))])

    def save_checkpoint(self):
        self.agent.checkpoint([dict(role='system', content='Fixture'), dict(role='user', content='Prepare')])

    def test_last_regular_request_can_validate_then_finish_in_reserved_phase(self):
        self.validate_save('one.sav')
        replies = [self.reply('test_program', dict(kind='save', root='save', path='two.sav')),
                   self.reply('finish', self.finish)]
        with mock.patch.object(self.agent, 'request_completion', side_effect=replies) as request:
            result = self.agent.run()
        self.assertEqual(len(result['validated_saves']), 2)
        self.assertEqual([tool['function']['name'] for tool in request.call_args.args[1]], ['finish'])
        self.assertEqual([event['phase'] for event in self.agent.events], ['prepare', 'finish'])
        self.assertTrue((self.agent.work / 'result.json').is_file())

    def test_regular_phase_preserves_state_without_automatic_continuation(self):
        self.agent.config["max_turns"] = 5
        self.validate_save('one.sav')
        replies = [self.reply('list_files', dict(root='save', path='.')),
                   self.reply('test_program', dict(kind='save', root='save', path='two.sav')),
                   self.reply('finish', self.finish)]
        prompts = []
        def request(messages, tools):
            prompts.append(json.loads(json.dumps(messages)))
            return replies.pop(0)
        with mock.patch.object(self.agent, 'request_completion', side_effect=request):
            result = self.agent.run()
        self.assertEqual(len(result['validated_saves']), 2)
        self.assertIn('"remaining_requests": 5', prompts[0][-1]['content'])
        self.assertEqual(prompts[0][0], prompts[1][0])
        self.assertEqual(prompts[1][0], prompts[2][0])
        self.assertEqual(prompts[1][:len(prompts[0])], prompts[0])
        self.assertEqual(prompts[2][:len(prompts[1])], prompts[1])
        self.assertFalse(any('Continue automatically' in (item.get('content') or '') for prompt in prompts for item in prompt))
        self.assertEqual([event['turn'] for event in self.agent.events], [0, 1, 2])

    def test_budget_exhaustion_does_not_continue_or_finish_without_evidence(self):
        reply = self.reply('list_files', dict(root='save', path='.'))
        prompts = []
        def request_reply(messages, tools):
            prompts.append(json.loads(json.dumps(messages)))
            return reply
        with mock.patch.object(self.agent, 'request_completion', side_effect=request_reply) as request:
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                self.agent.run()
        self.assertEqual(request.call_count, self.config['max_turns'] - host.RESERVED_FINISH_TURNS)
        self.assertFalse((self.agent.work / 'result.json').exists())
        self.assertTrue((self.agent.work / 'checkpoint.json').is_file())
        self.assertEqual(len(self.agent.dialogue()), 1)
        progress = [json.loads(prompt[-1]['content'].split('\n')[0].removeprefix('Host progress: ')) for prompt in prompts]
        self.assertEqual([item['remaining_work_requests'] for item in progress], [3, 2, 1])
        self.assertEqual([item['remaining_requests'] for item in progress], [4, 3, 2])
        self.assertEqual(json.loads((self.agent.work / 'session.json').read_text('utf-8'))['request_limit'], 4)

    def test_last_available_work_request_can_validate_and_use_the_120th_to_finish(self):
        self.agent.config['max_turns'] = 120
        self.validate_save('one.sav')
        replies = [self.reply('list_files', dict(root='save', path='.'))] * 118
        replies += [self.reply('test_program', dict(kind='save', root='save', path='two.sav')), self.reply('finish', self.finish)]
        progress = []
        def request_reply(messages, tools):
            progress.append(json.loads(messages[-1]['content'].split('\n')[0].removeprefix('Host progress: ')))
            return replies.pop(0)
        with mock.patch.object(self.agent, 'request_completion', side_effect=request_reply), mock.patch.object(host.sys, 'stderr'):
            result = self.agent.run()
        self.assertEqual(len(result['validated_saves']), 2)
        self.assertEqual(self.agent.requests_sent, 120)
        self.assertEqual(progress[-2]['remaining_work_requests'], 1)
        self.assertEqual(progress[-2]['remaining_requests'], 2)
        self.assertEqual(progress[-1]['phase'], 'finish')
        self.assertEqual(progress[-1]['remaining_finish_requests'], 1)

    def test_one_request_budget_is_reserved_for_evidence_backed_finish(self):
        self.agent.config['max_turns'] = 1
        with mock.patch.object(self.agent, 'request_completion') as request:
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                self.agent.run()
        request.assert_not_called()
        fresh = host.Agent(dict(self.config, max_turns=1))
        fresh.programs = dict(self.agent.programs)
        fresh.restore_checkpoint(json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8')))
        for name in ('one.sav', 'two.sav'):
            fresh.dispatch('test_program', dict(kind='save', root='save', path=name))
        fresh.checkpoint(json.loads((fresh.work / 'conversation.json').read_text('utf-8')))
        with mock.patch.object(fresh, 'request_completion', return_value=self.reply('finish', self.finish)) as request:
            fresh.run()
        self.assertEqual(request.call_count, 1)

    def test_ready_checkpoint_resumes_directly_into_finish(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        self.agent.events = [dict(turn=69, tool='test_program')]
        self.save_checkpoint()
        restored = host.Agent(self.config)
        with mock.patch.object(restored, 'request_completion', return_value=self.reply('finish', self.finish)) as request:
            restored.run()
        self.assertEqual(request.call_count, 1)
        self.assertEqual([tool['function']['name'] for tool in request.call_args.args[1]], ['finish'])
        self.assertEqual(restored.events[-1]['turn'], 70)
        self.assertEqual(restored.dialogue(), self.agent.dialogue())

    def test_unchanged_retry_preserves_the_bounded_request_prefix(self):
        self.agent.config['max_turns'] = 2
        self.validate_save('one.sav')
        with mock.patch.object(self.agent, 'request_completion', return_value=self.reply('list_files', dict(root='save', path='.'))):
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                self.agent.run()
        active = json.loads((self.agent.work / 'request-context.json').read_text('utf-8'))['messages']
        restored = host.Agent(dict(self.config, max_turns=4))
        replies = [self.reply('test_program', dict(kind='save', root='save', path='two.sav')), self.reply('finish', self.finish)]
        prompts = []
        def request_reply(messages, tools):
            prompts.append(json.loads(json.dumps(messages)))
            return replies.pop(0)
        with mock.patch.object(restored, 'request_completion', side_effect=request_reply):
            restored.run()
        self.assertEqual(prompts[0][:len(active)], active)
        self.assertEqual(restored.context_resets, 0)

    def test_overwritten_save_invalidates_validation_even_with_same_size_and_timestamp(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        self.save_checkpoint()
        slot = self.saves / 'two.sav'
        old = slot.stat()
        slot.write_bytes(b'other')
        os.utime(slot, ns=(old.st_atime_ns, old.st_mtime_ns))
        restored = host.Agent(self.config)
        restored.programs = dict(self.agent.programs)
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text(encoding='utf-8'))
        self.assertTrue(restored.restore_checkpoint(saved))
        self.assertEqual(set(restored.tests), {str(self.saves / 'one.sav')})
        self.assertFalse(restored.preparation_status()['ready_for_finish'])

    def test_changed_resources_invalidate_transcript_and_companion_mappings(self):
        self.agent.script_resources = {'names': {'7': 'Alice'}}
        self.save_checkpoint()
        self.scene.write_text('Changed resource', encoding='utf-8')
        restored = host.Agent(self.config)
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text(encoding='utf-8'))
        self.assertFalse(restored.restore_checkpoint(saved))
        self.assertEqual(restored.dialogue(), [])
        self.assertEqual(restored.script_resources, {})
        self.assertFalse(restored.used_skill)

    def test_changed_program_or_checkpoint_context_cannot_reuse_validations(self):
        self.save_checkpoint()
        for change in ('language', 'inventory', 'program'):
            with self.subTest(change=change):
                saved = json.loads((self.agent.work / 'checkpoint.json').read_text(encoding='utf-8'))
                if change == 'program':
                    saved['artifacts']['save_extractor.py'] = 'stale hash'
                else:
                    saved['context'][change] = 'changed'
                restored = host.Agent(self.config)
                self.assertFalse(restored.restore_checkpoint(saved))
                self.assertFalse(restored.preparation_status()['ready_for_finish'])

    def test_legacy_checkpoint_keeps_programs_but_refreshes_unstamped_evidence(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        self.save_checkpoint()
        path = self.agent.work / 'checkpoint.json'
        saved = json.loads(path.read_text(encoding='utf-8'))
        for name in ('context', 'source_stamps', 'artifacts'):
            saved.pop(name)
        host.atomic_json(path, saved)
        restored = host.Agent(dict(self.config, max_turns=0))
        with mock.patch.object(restored, 'request_completion') as request:
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                restored.run()
        request.assert_not_called()
        self.assertEqual(restored.programs, self.agent.programs)
        self.assertEqual(restored.dialogue(), [])
        self.assertEqual(restored.tests, {})

    def test_finish_rechecks_save_content_before_committing_success(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        (self.saves / 'two.sav').write_bytes(b'other')
        with self.assertRaisesRegex(ValueError, 'current progress saves'):
            self.agent.dispatch('finish', self.finish)
        self.assertFalse((self.agent.work / 'result.json').exists())

    def test_finish_argument_errors_can_be_corrected_without_rewriting_artifacts(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        replies = [self.reply('finish', dict(self.finish, speaker_resolution='external')),
                   self.reply('finish', self.finish)]
        with mock.patch.object(self.agent, 'request_completion', side_effect=replies):
            result = self.agent.run()
        self.assertIn('Resolved speakers', self.agent.events[0]['error'])
        self.assertEqual(result['artifacts'], self.agent.artifact_hashes())

    def test_reserved_phase_cannot_execute_other_tools_or_loop_forever(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        reply = self.reply('write_program', dict(kind='save', source='invalid Python'))
        before = self.agent.artifact_hashes()
        with mock.patch.object(self.agent, 'request_completion', return_value=reply) as request:
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                self.agent.run()
        self.assertEqual(request.call_count, host.FINISH_TURNS)
        self.assertEqual(before, self.agent.artifact_hashes())
        self.assertFalse((self.agent.work / 'result.json').exists())

    def test_http_failures_are_not_retried_as_budget_continuations(self):
        with mock.patch.object(self.agent, 'request_completion', side_effect=RuntimeError('AI HTTP status 401')) as request:
            with self.assertRaisesRegex(RuntimeError, 'AI HTTP status 401'):
                self.agent.run()
        self.assertEqual(request.call_count, 1)

    def test_total_budget_includes_finish_and_ignores_continuation_overrides(self):
        self.agent.config.update(max_turns=1000, max_continuations=1000)
        replies = [self.reply('list_files', dict(root='save', path='.'))] * 116
        replies += [self.reply('test_program', dict(kind='save', root='save', path='two.sav'))]
        replies += [self.reply('write_program', dict(kind='save', source='invalid'))] * 3
        self.validate_save('one.sav')
        with mock.patch.object(self.agent, 'request_completion', side_effect=replies) as request, mock.patch.object(host.sys, 'stderr'):
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                self.agent.run()
        self.assertEqual(request.call_count, 120)
        self.assertTrue(self.agent.session_closed)
        session = json.loads((self.agent.work / 'session.json').read_text('utf-8'))
        self.assertEqual(session['requests_sent'], 120)
        self.assertEqual(session['status'], 'budget_exhausted')

    def test_completed_task_closes_response_and_blocks_more_requests(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        response = mock.Mock()
        self.agent.active_response = response
        with mock.patch.object(self.agent, 'request_completion', return_value=self.reply('finish', self.finish)):
            self.agent.run()
        response.close.assert_called_once()
        self.assertTrue(self.agent.session_closed)
        self.assertIsNone(self.agent.active_response)
        session = json.loads((self.agent.work / 'session.json').read_text('utf-8'))
        self.assertEqual(session['status'], 'completed')
        with self.assertRaisesRegex(RuntimeError, 'closed'):
            self.agent.request_completion([], [])

    def test_failed_and_cancelled_tasks_close_the_session(self):
        for error, status in [(RuntimeError('AI HTTP status 401'), 'failed'), (KeyboardInterrupt(), 'cancelled')]:
            with self.subTest(status=status):
                agent = host.Agent(dict(self.config))
                response = mock.Mock()
                agent.active_response = response
                with mock.patch.object(agent, 'run_session', side_effect=error):
                    with self.assertRaises(type(error)):
                        agent.run()
                response.close.assert_called_once()
                session = json.loads((agent.work / 'session.json').read_text('utf-8'))
                self.assertTrue(session['closed'])
                self.assertEqual(session['status'], status)

    def test_long_context_starts_a_bounded_segment_and_still_finishes(self):
        self.agent.config['max_turns'] = 6
        self.validate_save('one.sav')
        first = self.reply('list_files', dict(root='save', path='.'))
        first['reasoning_content'] = 'old-reasoning-' * (host.CONTEXT['MAX_CONTEXT_BYTES'] // len('old-reasoning-') + 1)
        replies = [first, self.reply('test_program', dict(kind='save', root='save', path='two.sav')),
                   self.reply('finish', self.finish)]
        prompts = []
        def request(messages, tools):
            prompts.append(json.loads(json.dumps(messages)))
            return replies.pop(0)
        before = self.agent.artifact_hashes()
        with mock.patch.object(self.agent, 'request_completion', side_effect=request):
            result = self.agent.run()
        self.assertEqual(len(result['validated_saves']), 2)
        self.assertEqual(before, self.agent.artifact_hashes())
        self.assertEqual(self.agent.context_resets, 1)
        self.assertTrue(all(len(host.CONTEXT['encode'](prompt)) <= host.CONTEXT['MAX_CONTEXT_BYTES'] for prompt in prompts))
        self.assertTrue(all(message['role'] != 'assistant' for message in prompts[1]))
        self.assertIn('verified_host_checkpoint', prompts[1][2]['content'])
        self.assertIn('save_extractor.py', prompts[1][2]['content'])
        audit = json.loads((self.agent.work / 'conversation.json').read_text('utf-8'))
        self.assertIn(first, audit)
        active = json.loads((self.agent.work / 'request-context.json').read_text('utf-8'))['messages']
        self.assertNotIn(first, active)

    def test_legacy_large_history_is_archived_without_replaying_it_on_resume(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        old = [dict(role='system', content='Old mutable system'), dict(role='user', content='Old task'),
               dict(role='assistant', content='Completed inspection', reasoning_content='legacy-thought-' * 10000)]
        self.agent.checkpoint(old)
        # Legacy versions had no separately stored active context.
        (self.agent.work / 'request-context.json').unlink()
        restored = host.Agent(self.config)
        prompts = []
        def request(messages, tools):
            prompts.append(json.loads(json.dumps(messages)))
            return self.reply('finish', self.finish)
        with mock.patch.object(restored, 'request_completion', side_effect=request):
            restored.run()
        self.assertEqual(restored.context_resets, 1)
        self.assertLess(len(host.CONTEXT['encode'](prompts[0])), host.CONTEXT['MAX_CONTEXT_BYTES'])
        audit = json.loads((restored.work / 'conversation.json').read_text('utf-8'))
        self.assertEqual(audit[:len(old)], old)
        self.assertEqual(restored.dialogue(), self.agent.dialogue())
        self.assertEqual(restored.preparation_status()['validated_saves'], 2)

    def test_resume_restores_concrete_evidence_schema_even_when_contract_was_already_read(self):
        for name in ('one.sav', 'two.sav'):
            self.validate_save(name)
        old = [dict(role='system', content='Old system'),
               dict(role='user', content=json.dumps(dict(task='Legacy task without evidence fields')))]
        self.agent.checkpoint(old)
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text(encoding='utf-8'))
        self.assertTrue(saved['read_contract'])
        active = json.loads((self.agent.work / 'request-context.json').read_text(encoding='utf-8'))
        self.assertEqual(active['version'], host.CONTEXT['VERSION'])
        restored = host.Agent(self.config)
        prompts = []
        def request(messages, tools):
            prompts.append(json.loads(json.dumps(messages)))
            return self.reply('finish', self.finish)
        with mock.patch.object(restored, 'request_completion', side_effect=request):
            restored.run()
        self.assertTrue(restored.read_contract)
        self.assertEqual(restored.context_resets, 1)
        task = json.loads(prompts[0][1]['content'])
        contract = task['byte_evidence_contract']
        self.assertIn('verification_contract', contract['tool'])
        self.assertIn('not a file', contract['tool'])
        self.assertIn('offset,length,encoding', contract['script'])
        self.assertIn('table,index,reference', contract['script'])
        self.assertIn('operand_offset', contract['script'])
        self.assertIn('anchor:{offset,length,encoding}', contract['save'])
        self.assertIn('utf-8', contract['encodings'])
        self.assertTrue(all(event['tool'] == 'finish' for event in restored.events))

    def test_missing_script_evidence_points_to_the_contract_tool_and_specific_output_fields(self):
        source = 'def extract(data, metadata): return [{"position":0,"text":"Current","speaker":None}]'
        self.agent.dispatch('write_program', dict(kind='script', source=source))
        messages = [dict(role='system', content='Fixed'), dict(role='user', content='Test existing parser')]
        reply = self.reply('test_program', dict(kind='script', root='game', path='scene.txt'))
        with mock.patch.object(self.agent, 'request_completion', return_value=reply):
            self.agent.run_turn(messages, host.TOOLS, 0, 'prepare')
        result = json.loads(messages[-1]['content'])
        self.assertIn('Each script row needs evidence', result['error'])
        self.assertIn('verification_contract TOOL', result['error'])
        self.assertIn('do not repeat binary scans', result['error'])
        help_schema = result['verification_help']
        self.assertIn('not a file', help_schema['tool'])
        self.assertIn('offset,length,encoding', help_schema['script'])
        self.assertIn('table,index,reference', help_schema['script'])
        self.assertTrue(self.agent.read_contract)
        self.assertEqual(self.agent.dialogue(), [])
        self.assertEqual([event['tool'] for event in self.agent.events], ['test_program'])

    def test_large_tool_output_is_bounded_but_full_result_stays_in_audit(self):
        messages = [dict(role='system', content='Fixed'), dict(role='user', content='Inspect')]
        self.agent.audit_messages = list(messages)
        value = dict(stdout=('目录"\\\n' * 20000), exit_code=1, error='Inspection failed')
        with mock.patch.object(self.agent, 'request_completion', return_value=self.reply('inspect_data', dict(root='game', path='scene.txt', source='fixture'))), mock.patch.object(self.agent, 'dispatch', return_value=value):
            self.agent.run_turn(messages, host.TOOLS, 0, 'prepare')
        result = json.loads(messages[-1]['content'])
        self.assertLessEqual(len(messages[-1]['content'].encode('utf-8')), host.CONTEXT['MAX_TOOL_BYTES'])
        self.assertTrue(result['truncated'])
        self.assertEqual(result['error'], value['error'])
        complete = json.loads((self.agent.work / result['full_output']).read_text('utf-8'))
        self.assertEqual({key: complete[key] for key in value}, value)
        self.assertTrue(complete['observation_ref'].startswith('O'))
        audit = json.loads((self.agent.work / 'conversation.json').read_text('utf-8'))
        self.assertEqual(json.loads(audit[-1]['content']), value)
        index = [json.loads(line) for line in (self.agent.work / 'tool-results/calls.jsonl').read_text('utf-8').splitlines()]
        record = json.loads((self.agent.work / index[-1]['full_record']).read_text('utf-8'))
        self.assertEqual(record['result'], value)
        self.assertEqual(json.loads(record['function']['arguments'])['source'], 'fixture')

    def test_tool_pages_preserve_access_to_omitted_data(self):
        text = '文本' * 3000
        (self.root / 'long.txt').write_text(text, encoding='utf-8')
        pages = []
        offset = 0
        while offset is not None:
            page = self.agent.dispatch('read_text', dict(root='game', path='long.txt', offset=offset, length=100000))
            pages.append(page['text'])
            self.assertLessEqual(len(page['text']), 8192)
            offset = page['next_offset']
        self.assertEqual(''.join(pages), text)
        for index in range(61):
            (self.saves / f'{index:03d}.bin').write_bytes(b'fixture')
        page = self.agent.dispatch('list_files', dict(root='save', path='.'))
        self.assertEqual(len(page['entries']), 50)
        second = self.agent.dispatch('list_files', dict(root='save', path='.', offset=page['next_offset']))
        self.assertEqual(len(page['entries']) + len(second['entries']), page['total_entries'])
        output = 'Header\n' + 'archive-member\n' * 5000
        completed = mock.Mock(returncode=0, stdout=output.encode(), stderr=b'')
        with mock.patch.object(host.subprocess, 'run', return_value=completed):
            page = self.agent.dispatch('run_skill', dict(command='l', archive='scene.txt'))
            second = self.agent.dispatch('run_skill', dict(command='l', archive='scene.txt', offset=page['next_offset']))
        self.assertLessEqual(len(page['stdout']), 2048)
        self.assertEqual(page['stdout'] + second['stdout'], output[:4096])
        full = json.loads((self.agent.work / page['full_output']).read_text('utf-8'))
        self.assertEqual(full['stdout'], output)

    def test_long_chinese_reference_pages_reach_api_before_document_ranges_advance(self):
        chinese = ''.join('第%04d条：字段偏移、字节长度、空串索引和实际操作数必须一致。\n' % index
                          for index in range(400))
        self.agent.request_limit = self.agent.request_ceiling = self.config['max_turns'] = 8
        for filename, text in (('long-chinese.md', chinese), ('four-byte-unicode.md', '🧭' * 9000)):
            with self.subTest(filename=filename):
                relative = 'reference/' + filename
                path = self.root / relative
                path.parent.mkdir(exist_ok=True)
                self.assertGreater(len(text), 8192)
                self.assertLess(len(text), 16384)
                path.write_text(text, encoding='utf-8')
                messages = [dict(role='system', content='Fixed'), dict(role='user', content='Read the complete reference')]
                self.agent.audit_messages = list(messages)
                self.agent.context_system, self.agent.context_task = 'Fixed', 'Read the complete reference'
                prompts, results, payload_sizes = [], [], []
                def request(current, tools):
                    prompts.append(json.loads(json.dumps(current)))
                    index = len(prompts) - 1
                    if index < 2:
                        args = dict(root='skill', path=relative, length=100000,
                                    offset=0 if index == 0 else results[0]['next_offset'])
                        reply = self.reply('read_text', args)
                        reply['tool_calls'][0]['id'] = 'reference-read-' + str(index)
                        return reply
                    return self.reply('inspect_data', dict(root='game', path='scene.txt',
                        source='def extract(data, metadata): return {"text":"较长分析输出" * 2000}'))
                with mock.patch.object(self.agent, 'request_completion', side_effect=request):
                    for turn in range(3):
                        self.agent.prepare_request(messages, dict(phase='prepare', remaining_work_requests=3 - turn),
                                                   'Read the next reference page, then inspect compactly.')
                        self.agent.run_turn(messages, host.TOOLS, turn, 'prepare')
                        results.append(json.loads(messages[-1]['content']))
                        payload_sizes.append(len(messages[-1]['content'].encode('utf-8')))
                        if turn < 2:
                            page = results[-1]
                            self.assertNotIn('truncated', page)
                            self.assertLessEqual(payload_sizes[-1], 40960)
                            delivered_end = page['offset'] + len(page['text'])
                            document = self.agent.documents[relative]
                            self.assertEqual(document['ranges'], [[0, delivered_end]])
                            self.assertEqual(document['next_unread_offset'], delivered_end)
                            self.assertEqual(page['document_complete'], delivered_end == len(text))
                            self.assertEqual(page['next_offset'], delivered_end if delivered_end < len(text) else None)
                first, second, inspected = results
                self.assertEqual(len(first['text']), 8192)
                self.assertGreater(len(first['text'].encode('utf-8')), host.CONTEXT['MAX_TOOL_BYTES'])
                if filename == 'four-byte-unicode.md':
                    self.assertEqual(len(first['text'].encode('utf-8')), 32768)
                    self.assertGreater(payload_sizes[0], 32768)
                self.assertEqual(second['offset'], 8192)
                self.assertEqual(first['text'] + second['text'], text)
                # The next API request receives both complete pages, rather
                # than links that would claim unread characters were delivered.
                delivered = [json.loads(item['content']) for item in prompts[2]
                             if item['role'] == 'tool' and item['tool_call_id'].startswith('reference-read-')]
                self.assertEqual(''.join(page['text'] for page in delivered), text)
                self.assertTrue(self.agent.documents[relative]['complete'])
                self.assertTrue(inspected['truncated'])
                self.assertLessEqual(payload_sizes[2], host.CONTEXT['MAX_TOOL_BYTES'])
                full = json.loads((self.agent.work / inspected['full_output']).read_text(encoding='utf-8'))
                self.assertEqual(full['text'], '较长分析输出' * 2000)
                self.assertEqual(host.CONTEXT['MAX_CONTEXT_BYTES'], 480000)
                self.assertTrue(all(len(host.CONTEXT['encode'](prompt)) <= host.CONTEXT['MAX_CONTEXT_BYTES'] for prompt in prompts))
                self.assertLessEqual(len(host.CONTEXT['encode'](messages)), host.CONTEXT['MAX_CONTEXT_BYTES'])

    def test_api_tool_schema_stays_fixed_in_the_finish_phase(self):
        response = mock.MagicMock()
        response.read.return_value = json.dumps(dict(choices=[dict(finish_reason='tool_calls', message=self.reply('finish', self.finish))])).encode()
        opener = mock.Mock()
        opener.open.return_value.__enter__ = mock.Mock(return_value=response)
        opener.open.return_value.__exit__ = mock.Mock(return_value=False)
        self.agent.config['token'] = 'fixture-token'
        with mock.patch.object(host.urllib.request, 'build_opener', return_value=opener):
            self.agent.request_completion([dict(role='user', content='Prepare')], host.TOOLS)
            finish_only = [tool for tool in host.TOOLS if tool['function']['name'] == 'finish']
            self.agent.request_completion([dict(role='user', content='Finish')], finish_only)
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        self.assertEqual(payloads[0]['tools'], payloads[1]['tools'])
        self.assertEqual(payloads[0]['tools'], host.TOOLS)

    def test_resource_verification_reads_members_beyond_the_visible_page(self):
        for name in ('a.noa', 'b.noa'):
            (self.root / name).write_bytes(b'archive fixture')
        (self.root / 'search.ini').write_text('a.noa\nb.noa', encoding='utf-8')
        self.agent.config['inventory'] = [dict(path=str(self.root / name)) for name in ('a.noa', 'b.noa')]
        listing = ' [00000001] 1 unrelated\n' * 500 + ' [00000002] 2 script.csx\n'
        completed = mock.Mock(returncode=0, stdout=listing.encode(), stderr=b'')
        with mock.patch.object(host.subprocess, 'run', return_value=completed):
            selection = self.agent.verify_resource_selection(dict(script_name='script.csx', selected='a.noa', configuration='search.ini', priority='first'))
        self.assertEqual(selection['selected'], str(self.root / 'a.noa'))
        self.assertEqual(selection['candidates'], [str(self.root / 'a.noa'), str(self.root / 'b.noa')])

    def test_compaction_keeps_failed_targets_and_indexes_earlier_analysis_code(self):
        self.validate_save('one.sav')
        (self.saves / 'two.sav').write_bytes(b'Unknown caption')
        self.agent.required.add(str(self.saves / 'two.sav'))
        with self.assertRaises((ValueError, RuntimeError)):
            self.validate_save('two.sav')
        audit = [dict(role='system', content='Fixed'), dict(role='user', content='Prepare')]
        for index in range(10):
            reply = self.reply('inspect_data', dict(root='game', path='scene.txt', source=f'analysis-{index}'))
            reply['tool_calls'][0]['id'] = f'inspect-{index}'
            audit += [reply, dict(role='tool', tool_call_id=f'inspect-{index}', content=json.dumps(dict(observation=index)))]
        audit.append(dict(role='assistant', content='',
                          reasoning_content='old' * (host.CONTEXT['MAX_CONTEXT_BYTES'] // len('old') + 1)))
        self.agent.audit_messages = list(audit)
        self.agent.context_system, self.agent.context_task = 'Fixed', 'Prepare'
        messages = list(audit)
        before = dict(hashes=self.agent.artifact_hashes(), status=self.agent.preparation_status(), samples=dict(self.agent.samples))
        self.agent.prepare_request(messages, dict(phase='prepare', remaining_requests=2), 'Validate targets')
        self.assertEqual(self.agent.artifact_hashes(), before['hashes'])
        self.assertEqual(self.agent.preparation_status(), before['status'])
        self.assertEqual(self.agent.samples, before['samples'])
        self.assertIn(str(self.saves / 'two.sav'), self.agent.preparation_status()['pending_targets'])
        index = [json.loads(line) for line in (self.agent.work / 'tool-results/calls.jsonl').read_text('utf-8').splitlines()]
        self.assertEqual(len(index), 10)
        first = json.loads((self.agent.work / index[0]['full_record']).read_text('utf-8'))
        self.assertEqual(json.loads(first['function']['arguments'])['source'], 'analysis-0')
        self.assertEqual(first['result'], dict(observation=0))
        with self.assertRaises(ValueError):
            self.agent.dispatch('finish', self.finish)


if __name__ == '__main__':
    unittest.main()
