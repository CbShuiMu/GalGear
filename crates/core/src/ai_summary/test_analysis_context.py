"""Regression tests for durable research, document resumption and actual work budgets."""
import importlib.util
import json
import os
import pathlib
import struct
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('analysis_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class AnalysisContextTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        (self.root / 'scene.txt').write_bytes(b'Current')
        self.skill = self.root / 'skill'
        self.skill.mkdir()
        self.saves = self.root / 'saves'
        self.saves.mkdir()
        for name in ('one.sav', 'two.sav'):
            (self.saves / name).write_bytes(b'Current|scene.txt' + struct.pack('<I', 0))
        self.config = dict(game=str(self.root), save=str(self.saves), skill=str(self.skill),
                           work=str(self.root / 'work'), title='Fixture', language='en-US',
                           provider='deepseek', model='test', max_turns=12,
                           required_saves=[str(self.saves / 'two.sav')])
        self.agent = host.Agent(self.config)
        self.agent.dispatch('record_save_preflight', dict(path='two.sav', classification='progress_candidate',
            explanation='The reset fixture already has a native current caption.',
            observations=[dict(role='current_caption', value='Current',
                               proof=dict(offset=0, length=7, encoding='ascii'))]))
        self.agent.context_system = 'Resume prerequisites from host state.'
        self.agent.context_task = json.dumps(dict(task='Initial preparation', roots=self.config))
        self.agent.audit_messages = []
        self.serial = 0

    def reply(self, *calls, oversized=False):
        self.serial += 1
        reasoning = ('reasoning-' * (host.CONTEXT['MAX_CONTEXT_BYTES'] // len('reasoning-') + 1)
                     if oversized else 'fixture')
        return dict(role='assistant', content='', reasoning_content=reasoning,
                    tool_calls=[dict(id=f'call-{self.serial}-{index}', type='function',
                                     function=dict(name=name, arguments=json.dumps(args)))
                                for index, (name, args) in enumerate(calls)])

    def compact(self):
        self.agent.reset_context = True
        messages = []
        self.agent.prepare_request(messages, dict(phase='prepare', remaining_work_requests=4), 'Continue')
        return messages, json.loads(messages[2]['content'])

    def record_inspection(self):
        source = 'def extract(data, metadata): return {"length":len(data),"offset":0,"encoding":"ascii"}'
        reply = self.reply(('inspect_data', dict(root='game', path='scene.txt', source=source)))
        with mock.patch.object(self.agent, 'request_completion', return_value=reply):
            self.agent.run_turn([], host.TOOLS, 0, 'prepare')
        return reply['tool_calls'][0]['id'], source

    def finding(self, call_id):
        return dict(findings=[dict(id='text-layout', status='observed', summary='The fixture text starts at byte zero.',
                                   evidence_call_ids=[call_id])], next_steps=['Write and test the byte-span parser.'])

    def tool_history(self, *entries):
        audit, call_ids = [], []
        for name, arguments, result in entries:
            reply = self.reply((name, arguments))
            call_id = reply['tool_calls'][0]['id']
            audit.extend([reply, dict(role='tool', tool_call_id=call_id, content=json.dumps(result))])
            call_ids.append(call_id)
        return audit, call_ids

    def test_partial_document_read_resumes_after_compaction_and_restart(self):
        (self.skill / 'SKILL.md').write_text('a' * 5000, encoding='utf-8')
        first = self.agent.dispatch('read_text', dict(root='skill', path='SKILL.md', length=2048))
        self.assertFalse(first['document_complete'])
        self.assertFalse(self.agent.read_skill)
        messages, checkpoint = self.compact()
        self.assertEqual(checkpoint['state']['documents']['SKILL.md']['next_unread_offset'], 2048)
        self.agent.checkpoint(messages)
        restored = host.Agent(self.config)
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))
        self.assertTrue(restored.restore_checkpoint(saved))
        middle = restored.dispatch('read_text', dict(root='skill', path='SKILL.md', offset=2048, length=2048))
        last = restored.dispatch('read_text', dict(root='skill', path='SKILL.md', offset=4096))
        self.assertEqual(first['text'] + middle['text'] + last['text'], 'a' * 5000)
        self.assertTrue(restored.read_skill)
        self.assertEqual(restored.documents['SKILL.md']['ranges'], [[0, 5000]])
        restored.checkpoint(messages)
        old = (self.skill / 'SKILL.md').stat()
        (self.skill / 'SKILL.md').write_text('b' * 5000, encoding='utf-8')
        os.utime(self.skill / 'SKILL.md', ns=(old.st_atime_ns, old.st_mtime_ns))
        changed = host.Agent(self.config)
        changed.restore_checkpoint(json.loads((restored.work / 'checkpoint.json').read_text('utf-8')))
        self.assertFalse(changed.read_skill)
        self.assertNotIn('SKILL.md', changed.documents)

    def test_findings_and_earlier_analysis_code_survive_unrelated_calls_and_restart(self):
        call_id, source = self.record_inspection()
        self.agent.dispatch('record_findings', self.finding(call_id))
        for index in range(12):
            reply = self.reply(('list_files', dict(root='game', path='.')))
            self.agent.audit_messages.extend([reply, dict(role='tool', tool_call_id=reply['tool_calls'][0]['id'], content='{}')])
        messages, checkpoint = self.compact()
        self.assertNotIn(call_id, [row['tool_call_id'] for row in checkpoint['recent_observations']])
        observation = next(row for row in checkpoint['structural_ledger']['observations'] if row['tool_call_id'] == call_id)
        self.assertNotIn(call_id, [row['tool_call_id'] for row in checkpoint['research']['observations']])
        record = json.loads((self.agent.work / observation['full_record']).read_text('utf-8'))
        self.assertEqual(json.loads(record['function']['arguments'])['source'], source)
        self.assertEqual(checkpoint['model_analysis']['next_steps'], ['Write and test the byte-span parser.'])
        self.assertTrue(checkpoint['model_analysis']['model_authored'])
        self.agent.checkpoint(messages)
        restored = host.Agent(self.config)
        self.assertTrue(restored.restore_checkpoint(json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))))
        self.assertEqual(restored.analysis_notes, self.agent.analysis_notes)
        self.assertFalse(restored.analysis_needs_recheck)
        self.assertFalse(restored.preparation_status()['ready_for_finish'])
        (self.root / 'scene.txt').write_bytes(b'Changed')
        changed = host.Agent(self.config)
        self.assertFalse(changed.restore_checkpoint(json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))))
        self.assertTrue(changed.analysis_needs_recheck)
        self.assertEqual(changed.analysis_notes['findings']['text-layout']['summary'], self.agent.analysis_notes['findings']['text-layout']['summary'])
        self.assertTrue(changed.analysis_notes['findings']['text-layout']['stale'])

    def test_notes_cannot_cite_missing_calls_or_waive_validation(self):
        result = self.agent.dispatch('record_findings', self.finding('invented-call'))
        self.assertFalse(result['accepted'])
        self.assertTrue(result['errors'])
        self.assertEqual(self.agent.analysis_notes['findings'], {})
        call_id, _ = self.record_inspection()
        self.agent.dispatch('record_findings', self.finding(call_id))
        with self.assertRaisesRegex(ValueError, 'current progress saves'):
            self.agent.dispatch('finish', dict(engine='Fixture', script_language='en-US', speaker_resolution='unavailable', notes='Notes exist'))

    def test_safe_inspection_helpers_and_actionable_errors(self):
        result = host.run_parser('import json\ndef extract(data, metadata): return {"hex":hex(data[0]),"padded":data.ljust(3,b"0").hex()}', b'A', {})
        self.assertEqual(result, dict(hex='0x41', padded='413030'))
        reply = self.reply(('inspect_data', dict(root='game', path='scene.txt', source='def extract(data, metadata): return re.findall("x", "x")')))
        messages = []
        with mock.patch.object(self.agent, 'request_completion', return_value=reply):
            self.agent.run_turn(messages, host.TOOLS, 0, 'prepare')
        error = json.loads(messages[-1]['content'])
        self.assertIn("name 're' is not defined", error['error'])
        self.assertIn('Explicitly import', error['hint'])
        _, checkpoint = self.compact()
        self.assertIn("name 're' is not defined", checkpoint['research']['known_errors'][0]['error'])

    def test_parsers_and_current_save_finish_across_multiple_context_resets(self):
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
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
        inspect = self.reply(('inspect_data', dict(root='game', path='scene.txt', source='def extract(data, metadata): return {"offset":0,"length":len(data)}')), oversized=True)
        notes = self.reply(('record_findings', self.finding(inspect['tool_calls'][0]['id'])), oversized=True)
        replies = [inspect, notes, self.reply(('write_program', dict(kind='script', source=script))),
                   self.reply(('test_program', dict(kind='script', root='game', path='scene.txt'))),
                   self.reply(('list_files', dict(root='game', path='.'))),
                   self.reply(('review_speakers', dict(reason='The fixture inventory contains only narration and no name tables.', evidence_refs=['latest']))),
                   self.reply(('write_program', dict(kind='save', source=save))),
                   self.reply(('test_program', dict(kind='save', root='save', path='one.sav')),
                              ('test_program', dict(kind='save', root='save', path='two.sav'))),
                   self.reply(('finish', dict(engine='Fixture', script_language='en-US', speaker_resolution='unavailable', notes='Narration fixture.')))]
        prompts = []
        def request(messages, tools):
            prompts.append(json.loads(json.dumps(messages)))
            return replies.pop(0)
        with mock.patch.object(self.agent, 'request_completion', side_effect=request):
            result = self.agent.run()
        self.assertEqual(set(result['validated_saves']), {str(self.saves / 'two.sav')})
        self.assertEqual(self.agent.context_resets, 2)
        self.assertTrue(all(len(host.CONTEXT['encode'](prompt)) <= host.CONTEXT['MAX_CONTEXT_BYTES'] for prompt in prompts))
        self.assertIn('text-layout', json.loads(prompts[2][2]['content'])['model_analysis']['findings'])
        self.assertTrue(json.loads(prompts[2][2]['content'])['state']['read_skill'])
        self.assertTrue(self.agent.session_closed)

    def test_large_resource_inventory_cannot_hide_current_state_or_companion_schema(self):
        selections = {f'scene-{index}.bin': dict(selected=f'game/archive-{index}.bin',
                      candidates=[f'game/patch-{candidate}/archive-{index}.bin' for candidate in range(20)],
                      priority='configuration', input_stamps={'config.ini': [42, 123]}) for index in range(45)}
        self.assertGreater(len(host.CONTEXT['encode'](selections)), 30000)
        schema = 'metadata["speaker_resources"][resource_key][entry_key]={id,text}'
        helpers = {'names': dict(path='metadata-helpers/names.py', sha256='a' * 64)}
        facts = dict(progress=dict(dialogue_count=169, ready_for_finish=False,
                     speaker_review_required=True, transcript_coverage=dict(expected_members=45, covered_members=1)),
                     resource_selections=selections,
                     companion_metadata={'scene-0': dict(entries=169, target_script='unpacked/localized/scene-0.bin')},
                     companion_input_schema=schema, metadata_helpers=helpers,
                     parser_files={'script': 'script_extractor.py'}, artifacts={'script_extractor.py': 'b' * 64},
                     next_action='Validate matching scoped companions, then the remaining script batch.',
                     current_request=dict(phase='prepare', remaining_work_requests=70),
                     workflow=dict(stage='script_validation'), read_skill=True, read_contract=True,
                     model_analysis=dict(findings={}, next_steps=['Old model guess']))
        messages = host.CONTEXT['reset_messages'](self.agent.work, 'system', '{}', facts, [])
        checkpoint = json.loads(messages[2]['content'])
        state = checkpoint['state']
        self.assertNotIn('truncated', state)
        self.assertEqual(state['progress'], facts['progress'])
        self.assertEqual(state['companion_input_schema'], schema)
        self.assertEqual(state['companion_metadata'], facts['companion_metadata'])
        self.assertEqual(state['metadata_helpers'], helpers)
        self.assertEqual(state['parser_files'], facts['parser_files'])
        self.assertEqual(state['next_action'], facts['next_action'])
        self.assertEqual(state['current_request']['remaining_work_requests'], 70)
        archived = self.agent.work / state['section_archives']['resource_selections']
        self.assertEqual(json.loads(archived.read_text('utf-8')), selections)
        self.assertLess(len(host.CONTEXT['encode'](checkpoint)), 12000)
        self.assertFalse(checkpoint['model_analysis']['host_verified'])

    def test_large_test_history_keeps_failed_and_limited_samples_visible(self):
        tests = {f'success-{index}.sav': dict(coverage='node', matching_anchors=['actual anchor'] * 40,
                 sha256='c' * 64, facts={'source': 'scene.bin', 'line': index}) for index in range(80)}
        tests['current.sav'] = dict(coverage='unresolved', error='Anchor does not uniquely match the current node',
                                    matching_anchors=[], sha256='d' * 64)
        tests['partial.sav'] = dict(coverage='scene_only', matching_anchors=[],
                                    facts=dict(scene='chapter-3', warnings=['No current-node byte evidence']), sha256='e' * 64)
        state = host.CONTEXT['checkpoint_state'](self.agent.work, dict(verified_tests=tests,
                                                  progress=dict(ready_for_finish=False)))
        visible = state['verified_tests'].get('entries', state['verified_tests'])
        self.assertEqual(visible['current.sav']['coverage'], 'unresolved')
        self.assertIn('uniquely match', visible['current.sav']['error'])
        self.assertEqual(visible['partial.sav']['coverage'], 'scene_only')
        self.assertEqual(visible['partial.sav']['facts']['warnings'], ['No current-node byte evidence'])
        self.assertEqual(state['test_outcome_counts'], dict(total=82, failed=1, scene_only=1, unresolved=1))
        archived = self.agent.work / state['section_archives']['verified_tests']
        self.assertEqual(json.loads(archived.read_text('utf-8')), tests)
        self.assertFalse(state['progress']['ready_for_finish'])

    def test_archived_exchange_read_does_not_expand_into_the_next_checkpoint(self):
        source = 'def extract(data, metadata): return {"offset": 64, "length": 16}'
        record = dict(tool_call_id='original', function=dict(name='inspect_data',
                      arguments=json.dumps(dict(root='game', path='scene.txt', source=source))),
                      result={'values': ['a "quoted" \\ string\n' * 200]})
        path, _ = host.CONTEXT['archive_result'](self.agent.work, record)
        audit = [dict(role='assistant', tool_calls=[dict(id='history-read', function=dict(name='read_text',
                 arguments=json.dumps(dict(root='work', path=path))))]),
                 dict(role='tool', tool_call_id='history-read', content=json.dumps(dict(text=json.dumps(record))))]
        messages = host.CONTEXT['reset_messages'](self.agent.work, 'system', '{}',
                                                   dict(parser_files={'script': 'script_extractor.py'}), audit)
        checkpoint = json.loads(messages[2]['content'])
        observation = checkpoint['recent_observations'][0]
        self.assertTrue(observation['result']['archived_read'])
        self.assertNotIn(source, messages[2]['content'])
        self.assertLess(len(host.CONTEXT['encode'](checkpoint)), 3500)
        full_record = json.loads((self.agent.work / observation['full_record']).read_text('utf-8'))
        self.assertEqual(json.loads(full_record['result']['text']), record)

    def test_bounded_tool_preview_retains_structured_json_and_exact_archive(self):
        value = dict(results=[dict(path=f'file-{index}', text='目录"\\\n' * 200) for index in range(50)],
                     error='Validation failed', observation_ref='O7')
        bounded = host.CONTEXT['bounded_result'](self.agent.work, value, 1200)
        self.assertLessEqual(len(host.CONTEXT['encode'](bounded)), 1200)
        self.assertIsInstance(bounded['preview'], dict)
        self.assertEqual(bounded['error'], value['error'])
        self.assertEqual(bounded['observation_ref'], 'O7')
        self.assertEqual(json.loads((self.agent.work / bounded['full_output']).read_text('utf-8')), value)

    def test_recent_window_deduplicates_without_backfilling_old_instructions(self):
        target = dict(kind='script', root='work', path='unpacked/scene.bin')
        audit, call_ids = self.tool_history(
            ('write_program', dict(kind='script', source='old source'),
             dict(requires_test='script', next_action='Old instruction: test the changed parser now')),
            ('test_program', target, dict(error='Old provenance failure')),
            ('verify_resource_selection', dict(script_name='scene.bin', selected='data.arc'), dict(selected='data.arc')),
            ('test_program', target, dict(count=169)),
            ('inspect_data', dict(root='work', path='unpacked/scene.bin'), dict(offset=0)),
            ('run_skill', dict(command='l', archive='data.arc'), dict(exit_code=0)))
        # The actual four most recent exchanges already appear in the ledger.
        ledger = dict(observations=[dict(tool_call_id=call_id, ref=f'O{index}')
                                   for index, call_id in enumerate(call_ids[-4:], 1)])
        messages = host.CONTEXT['reset_messages'](self.agent.work, 'system', '{}',
                    dict(structural_ledger=ledger, next_action='Continue complete transcript validation.'), audit)
        checkpoint = json.loads(messages[2]['content'])
        self.assertEqual(checkpoint['recent_observations'], [])
        self.assertEqual(checkpoint['research']['known_errors'], [])
        self.assertEqual(checkpoint['research']['resolved_errors_omitted'], 1)
        self.assertNotIn('Old instruction', messages[2]['content'])
        self.assertNotIn('Old provenance failure', messages[2]['content'])
        # A partial overlap also leaves only the actual remaining recent call.
        recent = host.CONTEXT['recent_observations'](self.agent.work, audit, call_ids[-3:])
        self.assertEqual([row['tool_call_id'] for row in recent], [call_ids[-4]])

    def test_historical_errors_resolve_only_after_same_tool_and_input_success(self):
        save = dict(kind='save', root='save', path='current.sav')
        audit, call_ids = self.tool_history(
            ('test_program', save, dict(error='Old missing anchor proof')),
            ('test_program', dict(kind='save', root='save', path='other.sav'), dict(error='Other sample failed')),
            ('inspect_data', dict(root='save', path='current.sav', source='broken inspection'), dict(error='Inspection failed')),
            ('test_program', dict(kind='script', root='save', path='other.sav'), dict(count=1)),
            ('test_program', save, dict(coverage='node', matching_anchors=[1])))
        # Deduped successes still resolve older failures even when their full
        # observations are supplied by the structural ledger instead.
        context = host.CONTEXT['research_context'](self.agent.work, audit, exclude=call_ids[-2:])
        errors = context['known_errors']
        self.assertEqual({row['error'] for row in errors}, {'Other sample failed', 'Inspection failed'})
        self.assertTrue(all(row['historical'] and row['resolved'] is False for row in errors))
        self.assertEqual(context['resolved_errors_omitted'], 1)
        for row in errors:
            record = json.loads((self.agent.work / row['full_record']).read_text('utf-8'))
            self.assertEqual(record['result']['error'], row['error'])

    def test_failure_after_an_earlier_success_remains_visible(self):
        first = dict(kind='script', root='work', path='unpacked\\scene.bin')
        repaired = dict(kind='script', root='work', path='unpacked/scene.bin')
        audit, _ = self.tool_history(
            ('test_program', first, dict(error='Initial failed attempt')),
            ('test_program', repaired, dict(count=169)),
            ('test_program', repaired, dict(error='New failure after changed input')),
            ('read_json', dict(root='work', path='sample.json', offset=20), dict(error='Page failed')),
            ('read_json', dict(root='work', path='sample.json', offset=0), dict(items=[1])))
        context = host.CONTEXT['research_context'](self.agent.work, audit)
        self.assertEqual({row['error'] for row in context['known_errors']},
                         {'New failure after changed input', 'Page failed'})
        self.assertEqual(context['resolved_errors_omitted'], 1)

    def test_speaker_chain_summary_keeps_verified_access_roles_and_sample(self):
        global_resource = dict(key='names', records=90, helper_file='metadata-helpers/names.py',
            input_access='metadata["speaker_resources"]["names"]', host_verified=True,
            sample_entry=dict(entry='native-token', id='native-token', text='Localized name'),
            all_names={str(index): 'UNNEEDED_FULL_NAME_TABLE_' + str(index) for index in range(90)})
        speaker_state = dict(host_verified=True, global_resources=[global_resource], scoped_resources=[],
            empty_resources=[dict(key='probe', records=0, empty=True,
                helper_file='metadata-helpers/probe.py', input_access='metadata["speaker_resources"]["probe"]',
                target=dict(root='work', path='unpacked/scene.bin'))],
            sample=dict(root='work', path='unpacked/scene.bin', rows=169, named_rows=0), next_step='bind_messages')
        contract = dict(input_schema='metadata["speaker_resources"][key][entry]={id,text}',
                        proof='Use verified resource/entry references. ' * 80)
        facts = dict(progress=dict(dialogue_count=169, speaker_review_required=True, speaker_metadata_progress=speaker_state),
            current_request=dict(phase='prepare', speaker_metadata_progress=speaker_state),
            speaker_metadata_progress=speaker_state, speaker_contract=contract,
            parser_files={'script': 'script_extractor.py'}, next_action='Bind native messages to the verified global names on the sample.')
        messages = host.CONTEXT['reset_messages'](self.agent.work, 'system', '{}', facts, [])
        state = json.loads(messages[2]['content'])['state']
        actual = state['speaker_metadata_progress']
        self.assertEqual(actual['sample'], speaker_state['sample'])
        self.assertEqual(actual['next_step'], 'bind_messages')
        self.assertTrue(actual['host_verified'])
        row = actual['global_resources'][0]
        self.assertEqual(row['sample_entry'], global_resource['sample_entry'])
        self.assertEqual(row['input_access'], global_resource['input_access'])
        self.assertEqual(row['records'], 90)
        self.assertTrue(row['host_verified'])
        self.assertTrue(actual['empty_resources'][0]['empty'])
        self.assertEqual(actual['empty_resources'][0]['records'], 0)
        self.assertIn('does not prove', actual['empty_resource_meaning'])
        for key in ('progress', 'current_request'):
            nested = state[key]['speaker_metadata_progress']
            self.assertEqual(nested['state_ref'], 'state.speaker_metadata_progress')
            self.assertEqual(nested['sample'], speaker_state['sample'])
            self.assertEqual(nested['next_step'], 'bind_messages')
            self.assertNotIn('global_resources', nested)
        self.assertNotIn('UNNEEDED_FULL_NAME_TABLE_', messages[2]['content'])
        self.assertEqual(state['speaker_contract'], contract)
        self.assertEqual(state['next_action'], facts['next_action'])
        archive = state['section_archives']['speaker_metadata_progress.global_resources']
        self.assertEqual(json.loads((self.agent.work / archive).read_text('utf-8')), [global_resource])

    def test_large_speaker_scope_inventory_keeps_representative_binding_and_helper_aliases(self):
        sample = dict(root='work', path='unpacked/chapter-079.bin', rows=169, named_rows=0)
        scopes, helpers = [], {}
        for index in range(80):
            key = f'messages:chapter-{index:03d}'
            target = dict(root='work', path=f'unpacked/chapter-{index:03d}.bin')
            scopes.append(dict(key=key, records=169, helper_file='metadata-helpers/' + 'a' * 64 + '.py',
                input_access='metadata["speaker_resources"][' + json.dumps(key) + ']', target=target,
                sample_entry=dict(entry='0', id=0, text='Localized name')))
            helpers[key] = dict(path='metadata-helpers/' + 'a' * 64 + '.py', sha256='a' * 64)
        speaker_state = dict(global_resources=[], scoped_resources=scopes, empty_resources=[],
                             sample=sample, next_step='retest_sample')
        facts = dict(progress=dict(dialogue_count=169, speaker_metadata_progress=speaker_state),
                     current_request=dict(speaker_metadata_progress=speaker_state),
                     speaker_metadata_progress=speaker_state, metadata_helpers=helpers,
                     next_action='Retest the representative sample with its verified scoped message bindings.')
        state = host.CONTEXT['checkpoint_state'](self.agent.work, facts)
        actual = state['speaker_metadata_progress']
        self.assertEqual(actual['sample'], sample)
        self.assertEqual(actual['next_step'], 'retest_sample')
        self.assertEqual(actual['resource_counts']['scoped_resources'], 80)
        shown = actual['scoped_resources']
        self.assertTrue(shown['truncated'])
        self.assertEqual(shown['entries'][0]['target'], {'root': sample['root'], 'path': sample['path']})
        self.assertEqual(shown['entries'][0]['sample_entry'], scopes[-1]['sample_entry'])
        self.assertEqual(shown['entries'][0]['helper_file'], scopes[-1]['helper_file'])
        archive = state['section_archives']['speaker_metadata_progress.scoped_resources']
        self.assertEqual(json.loads((self.agent.work / archive).read_text('utf-8')), scopes)
        grouped = state['metadata_helpers']
        self.assertEqual(grouped['total_helpers'], 80)
        self.assertEqual(len(grouped['groups']), 1)
        self.assertEqual(grouped['groups'][0]['helper_keys'], list(helpers))
        self.assertEqual(state['progress']['dialogue_count'], 169)
        self.assertEqual(state['next_action'], facts['next_action'])
        self.assertLess(len(host.CONTEXT['encode'](state)), 18000)

    def test_legacy_companion_preview_keeps_one_name_without_replaying_the_table(self):
        names = {f'native-{index}': dict(id=f'native-{index}', text=f'Localized name {index}') for index in range(90)}
        companions = dict(names=dict(entries=90, target_script=None, preview=names),
                          probe=dict(entries=0, target_script='unpacked/scene.bin', preview={}))
        state = host.CONTEXT['checkpoint_state'](self.agent.work, dict(companion_metadata=companions))
        row = state['companion_metadata']['names']
        self.assertEqual(row['entries'], 90)
        self.assertEqual(row['preview'], {'native-0': names['native-0']})
        self.assertEqual(state['companion_metadata']['probe']['entries'], 0)
        archive = state['section_archives']['companion_metadata']
        self.assertEqual(json.loads((self.agent.work / archive).read_text('utf-8')), companions)

    def test_nested_speaker_state_has_independent_budget_without_a_canonical_copy(self):
        speaker_state = dict(global_resources=[dict(key='names', records=90, helper_file=None,
            input_access='metadata["speaker_resources"]["names"]', sample_entry=dict(entry='A', id='A', text='Name'))],
            scoped_resources=[], empty_resources=[], sample=None, next_step='discover_names')
        state = host.CONTEXT['checkpoint_state'](self.agent.work,
            dict(progress=dict(speaker_metadata_progress=speaker_state, ready_for_finish=False)))
        actual = state['progress']['speaker_metadata_progress']
        self.assertEqual(actual['global_resources'][0]['sample_entry'], speaker_state['global_resources'][0]['sample_entry'])
        self.assertEqual(actual['next_step'], 'discover_names')
        self.assertNotIn('state_ref', actual)
        self.assertFalse(state['progress']['ready_for_finish'])

    def test_current_program_and_helper_sources_survive_large_inventories_exactly(self):
        script = 'def extract(data, metadata):\r\n    marker = "名字 \\\\ quoted"\r\n    return []\r\n' + '# script \\ "keep"\r\n' * 190
        save = 'def extract(data, metadata):\n    return {}\n' + '# save exact source\n' * 170
        helper = 'def extract(data, metadata):\n    return {"values": {}, "speakers": {}}\n' + '# helper source\n' * 210
        helper_hash = host.hashlib.sha256(helper.encode()).hexdigest()
        failure = dict(kind='script', category='speaker_metadata', root='work', path='unpacked/scene.bin',
                       error='Current sample has an unresolved native speaker binding', source_sha256='b' * 64)
        speaker = dict(global_resources=[], scoped_resources=[], empty_resources=[],
                       sample=dict(root='work', path='unpacked/scene.bin', rows=169, named_rows=0), next_step='bind_messages')
        facts = dict(resource_selections={str(index): dict(selected='data.arc', candidates=['archive-' + 'x' * 100] * 20)
                                         for index in range(45)},
                     program_sources=dict(script=dict(path='script_extractor.py', sha256='b' * 64, source=script),
                                          save=dict(path='save_extractor.py', sha256='c' * 64, source=save)),
                     helper_sources={helper_hash: dict(path='metadata-helpers/' + helper_hash + '.py',
                         helper_keys=['names'] + ['messages:' + str(index) for index in range(300)], source=helper)},
                     last_script_failure=failure, speaker_metadata_progress=speaker,
                     progress=dict(dialogue_count=169, speaker_metadata_progress=speaker), next_action='Repair this exact parser and retest the sample.')
        messages = host.CONTEXT['reset_messages'](self.agent.work, 'system', '{}', facts, [])
        state = json.loads(messages[2]['content'])['state']
        self.assertEqual(host.CONTEXT['VERSION'], 17)
        self.assertEqual(state['program_sources']['script']['source'], script)
        self.assertEqual(state['program_sources']['save']['source'], save)
        self.assertEqual(state['helper_sources'][helper_hash]['source'], helper)
        self.assertNotIn('preview', state['program_sources']['script'])
        self.assertNotIn('source_archived', state['program_sources']['script'])
        self.assertEqual(state['last_script_failure'], failure)
        self.assertEqual(state['speaker_metadata_progress']['sample'], speaker['sample'])
        self.assertEqual(state['progress']['speaker_metadata_progress']['state_ref'], 'state.speaker_metadata_progress')
        aliases = state['helper_sources'][helper_hash]['helper_keys']
        self.assertTrue(aliases['truncated'])
        self.assertEqual(aliases['total_entries'], 301)
        self.assertEqual(state['next_action'], facts['next_action'])
        self.assertLess(len(host.CONTEXT['encode'](messages)), host.CONTEXT['MAX_CONTEXT_BYTES'])

    def test_oversized_source_is_archived_whole_without_a_partial_function(self):
        script = 'def extract(data, metadata): return []\n#' + 's' * host.CONTEXT['PROGRAM_SOURCE_BYTES']
        helper = 'def extract(data, metadata): return {}\n#' + 'h' * host.CONTEXT['HELPER_SOURCE_BYTES']
        save = 'def extract(data, metadata): return {}'
        state = host.CONTEXT['checkpoint_state'](self.agent.work, dict(
            program_sources=dict(script=dict(path='script_extractor.py', sha256='s', source=script),
                                 save=dict(path='save_extractor.py', sha256='v', source=save)),
            helper_sources=dict(large=dict(path='metadata-helpers/large.py', helper_keys=['names'], source=helper))))
        for row, source in ((state['program_sources']['script'], script), (state['helper_sources']['large'], helper)):
            self.assertTrue(row['source_archived'])
            self.assertNotIn('source', row)
            self.assertNotIn('preview', row)
            self.assertEqual(row['source_pointer'], '/source')
            archived = json.loads((self.agent.work / row['full_source']).read_text('utf-8'))
            self.assertEqual(archived['source'], source)
        self.assertEqual(state['program_sources']['save']['source'], save)

    def test_helper_sources_deduplicate_code_and_preserve_host_priority_under_budget(self):
        def source(tag):
            return 'def extract(data, metadata): return {}\n#' + tag + 'x' * 8900
        names, sample, extra = source('names'), source('sample'), source('extra')
        state = host.CONTEXT['checkpoint_state'](self.agent.work, dict(helper_sources=dict(
            names=dict(path='metadata-helpers/names.py', helper_keys=['names'], source=names),
            names_alias=dict(path='metadata-helpers/names-copy.py', helper_keys=['names-alias'], source=names),
            sample=dict(path='metadata-helpers/sample.py', helper_keys=['messages:sample'], source=sample),
            extra=dict(path='metadata-helpers/extra.py', helper_keys=['messages:other'], source=extra)),
            program_sources=dict(script=dict(path='script_extractor.py', sha256='s', source='def extract(data, metadata): return []'))))
        helpers = state['helper_sources']
        self.assertEqual(list(helpers), ['names', 'sample', 'extra'])
        self.assertEqual(helpers['names']['source'], names)
        self.assertEqual(helpers['names']['helper_keys'], ['names', 'names-alias'])
        self.assertEqual(helpers['sample']['source'], sample)
        self.assertTrue(helpers['extra']['source_archived'])
        self.assertNotIn('source', helpers['extra'])
        self.assertIn('source', state['program_sources']['script'])
        self.assertEqual(sum(len(host.CONTEXT['encode'](row['source'])) for row in helpers.values() if 'source' in row),
                         len(host.CONTEXT['encode'](names)) + len(host.CONTEXT['encode'](sample)))

    def test_current_script_failure_keeps_category_and_source_identity_when_details_are_large(self):
        for category in ('script', 'speaker_metadata', 'resource'):
            with self.subTest(category=category):
                failure = dict(kind='script', category=category, root='work', path='unpacked/scene.bin',
                    error='Current independent validation failed', source_sha256='a' * 64,
                    error_details={'large_evidence': 'data' * 10000})
                state = host.CONTEXT['checkpoint_state'](self.agent.work, dict(last_script_failure=failure))
                for key in ('kind', 'category', 'root', 'path', 'error', 'source_sha256'):
                    self.assertEqual(state['last_script_failure'][key], failure[key])
                archive = state['section_archives']['last_script_failure']
                self.assertEqual(json.loads((self.agent.work / archive).read_text('utf-8')), failure)
        cleared = host.CONTEXT['checkpoint_state'](self.agent.work, dict(last_script_failure=None))
        self.assertIsNone(cleared['last_script_failure'])


if __name__ == '__main__':
    unittest.main()
