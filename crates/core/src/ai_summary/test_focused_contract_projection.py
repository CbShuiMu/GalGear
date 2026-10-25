"""Focused contracts must retain repair rules in the actual bounded API result."""
import contextlib
import importlib.util
import io
import json
import pathlib
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location('focused_contract_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class FocusedContractProjectionTests(unittest.TestCase):
    def test_structured_contract_fits_and_requires_typed_fields(self):
        result = self.call(dict(topic='structured'))
        self.assertEqual(result['structured_literals']['document']['syntax'], 'tjs_const')
        self.assertEqual(result['public_api']['module'], 'galgear_literals')
        self.assertIn('UTF-16 zero high byte', '\n'.join(result['rules']))
        self.assertIn('structured_literals', self.complete())

    def test_adapter_contract_fits_one_result_and_exposes_replayable_schema(self):
        result = self.call(dict(topic='adapter'))
        self.assertIn('code_region', result['adapter_evidence']['shared_spec'])
        self.assertIn('erisa_nemesis', result['byte_codecs']['codecs'])

    def test_codec_discovery_preserves_boundaries_and_full_capabilities_are_retrievable(self):
        result = self.call(dict(topic='script'))
        codecs = result['byte_codecs']
        self.assertIn('galgear_codecs.capabilities()', codecs['full_capabilities'])
        self.assertEqual(codecs['tjs_text_stream']['view'], '{op:"tjs_text_stream"}')
        self.assertIn('never verifies native filter/checksum', codecs['boundary'])
        self.assertIn('current/resume roles', codecs['boundary'])
        self.assertIsInstance(codecs['checksum_hypotheses'], str)
        complete = self.complete()['byte_codecs']
        self.assertEqual(complete, host.codec_capabilities())
        self.assertIn('native mode2', complete['tjs_text_stream']['framing'])
        self.assertIn('Caller-supplied checksum is not native archive evidence', complete['checksum_hypotheses']['scope'])

    def test_optional_codec_details_do_not_displace_focused_proof_schemas(self):
        capabilities = host.CODECS['capabilities']()
        capabilities['tjs_text_stream']['additional_documentation'] = 'Full native framing notes. ' * 1000
        def expanded():
            return json.loads(json.dumps(capabilities))
        with mock.patch.dict(host.CODECS, capabilities=expanded):
            complete = self.complete()
            self.assertEqual(complete['byte_codecs']['tjs_text_stream']['additional_documentation'],
                             capabilities['tjs_text_stream']['additional_documentation'])
            for topic in ('script', 'save', 'adapter'):
                with self.subTest(topic=topic):
                    result = self.call(dict(topic=topic))
                    self.assertNotIn('additional_documentation', result['byte_codecs']['tjs_text_stream'])
                    if topic == 'script':
                        self.assert_scope_repair_rules(result, complete)
                        self.assertIn(complete['rules'][5], result['rules'])
                        self.assertIn(complete['rules'][13], result['rules'])
                        self.assertIn(complete['rules'][14], result['rules'])

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'fixture.exe').write_bytes(b'fixture')
        self.agent = host.Agent(dict(game=str(self.root), save=str(self.root), skill=str(self.root),
            work=str(self.root / 'work'), title='Synthetic contract fixture', language='en-US',
            provider='deepseek', model='offline-fixture', max_turns=30))
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
        self.agent.audit_messages = []
        self.agent.context_system = 'Synthetic contract projection test.'
        self.agent.context_task = '{}'
        self.serial = 0

    def call(self, arguments):
        """Exercise the real tool/ledger/bounding path; only model transport is mocked."""
        self.serial += 1
        reply = dict(role='assistant', content='', tool_calls=[dict(id='contract-' + str(self.serial),
            type='function', function=dict(name='verification_contract', arguments=json.dumps(arguments)))])
        messages = []
        with mock.patch.object(self.agent, 'request_completion', return_value=reply), contextlib.redirect_stderr(io.StringIO()):
            self.agent.run_turn(messages, host.TOOLS, self.serial, 'prepare')
        value = json.loads(messages[-1]['content'])
        self.assertNotIn('error', value)
        self.assertNotIn('truncated', value)
        self.assertLessEqual(len(messages[-1]['content'].encode('utf-8')), host.CONTEXT['MAX_TOOL_BYTES'])
        self.assertTrue(value['observation_ref'].startswith('O'))
        return value

    def complete(self):
        result = self.call(dict(topic='all'))
        return json.loads(self.agent.path('work', result['full_contract']).read_text('utf-8'))

    def assert_scope_repair_rules(self, value, complete):
        # Test what the model actually receives, including the speaker branch's
        # final rule replacement; checking its earlier index tuple misses that bug.
        for index in (8, 9, 10):
            self.assertIn(complete['rules'][index], value['rules'])
        text = '\n'.join(value['rules'])
        self.assertIn('target_script:{root,path}', text)
        self.assertIn('EACH script', text)
        self.assertIn('Missing mapping is an error, not narration', text)
        self.assertIn('Verified auxiliary non-message strings may be omitted as dialogue rows', text)
        self.assertIn('complete text table still retains their ordinals', text)

    def test_script_api_result_keeps_native_scope_narration_and_auxiliary_rules(self):
        complete = self.complete()
        script = self.call(dict(topic='script'))
        self.assertEqual(script['topic'], 'script')
        self.assert_scope_repair_rules(script, complete)
        self.assertIn(complete['rules'][5], script['rules'])
        self.assertIn(complete['rules'][13], script['rules'])
        self.assertIn(complete['rules'][14], script['rules'])

    def test_speaker_api_result_keeps_repair_rules_and_existing_proof_shapes(self):
        complete = self.complete()
        speaker = self.call(dict(topic='speaker'))
        self.assert_scope_repair_rules(speaker, complete)
        self.assertEqual(speaker['proof_shapes'], complete['speaker_proofs'])
        for field in ('global_name', 'scoped_named_message', 'scoped_narration', 'transcript_speaker'):
            self.assertIn(field, speaker['proof_shapes'])
        self.assertEqual(speaker['proof_shapes']['scoped_narration']['text'], None)
        for index in (0, 4, 11):
            self.assertIn(complete['rules'][index], speaker['rules'])

    def test_save_api_contract_keeps_its_existing_projection(self):
        complete = self.complete()
        save = self.call(dict(topic='save'))
        self.assertEqual(save['rules'], [complete['rules'][index] for index in (0, 15, 16, 17, 18)])
        self.assertNotIn('proof_shapes', save)
        self.assertEqual(save['available_topics'], ['script', 'speaker', 'save', 'adapter', 'structured', 'resume'])

    def test_resume_contract_is_complete_and_never_calls_a_label_a_displayed_caption(self):
        result = self.call(dict(topic='resume'))
        self.assertEqual(result['kag_resume']['profile'], 'kag_parser_label_v1')
        self.assertEqual(result['kag_resume']['saved_link']['record'], ['core', 'mainConductor'])
        self.assertEqual(result['public_api']['module'], 'galgear_kag')
        self.assertIn('not the last displayed', result['kag_resume']['role'])

    def test_auto_contract_uses_same_complete_script_and_speaker_repair_rules(self):
        complete = self.complete()
        for stage, expected_topic in (('script_validation', 'script'), ('speaker_review', 'speaker')):
            with self.subTest(stage=stage):
                self.agent.workflow.stage = stage
                value = self.call({})
                self.assertEqual(value['topic'], expected_topic)
                self.assert_scope_repair_rules(value, complete)


if __name__ == '__main__':
    unittest.main()
