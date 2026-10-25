"""Exercise the public companion contract on synthetic bytes, without a game decoder."""
import json
import pathlib
import struct
import tempfile
import unittest

from test_speaker_scopes import host, PARSER, companion_bytes


NAMES = '''def extract(data, metadata):
    end = data.index(b'\\0')
    token = data[:end].decode('ascii')
    text = data[end+1:-1].decode('ascii')
    return {'values':{},'speakers':{'entry':{'id':token,'text':text,
        'id_evidence':{'offset':0,'length':end,'encoding':'ascii'},
        'evidence':{'offset':end+1,'length':len(data)-end-2,'encoding':'ascii'}}}}
'''

MESSAGES = '''def extract(data, metadata):
    name_end = data.index(bytes([0]), 1)
    message = name_end + 2
    native = int.from_bytes(data[message+1:message+5], 'little')
    token = data[1:name_end].decode('ascii')
    for entry, record in metadata['speaker_resources']['names'].items():
        if record['id'] == token:
            return {'values':{},'speakers':{'message':{'id':native,'text':record['text'],
                'id_evidence':{'offset':message+1,'width':4,'endian':'little'},
                'native_link':{'kind':'willplus_ws2_name_message_config1_cp932',
                    'name_command_offset':0,'message_command_offset':message},
                'evidence':{'resource':'names','entry':entry,
                    'reference':{'offset':1,'length':name_end-1,'encoding':'cp932'}}}}}
    raise ValueError('Unresolved native token')
'''


class SpeakerChainContractTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        (self.root / 'names.bin').write_bytes(b'T\0Alice\0')
        (self.root / 'message.bin').write_bytes(companion_bytes(0, 'T'))
        (self.root / 'text.bin').write_bytes(struct.pack('<II', 1, 7) + b'Current')
        self.config = dict(game=str(self.root), save=str(self.root), skill=str(self.root),
                           work=str(self.root / 'work'), language='en-US')
        self.agent = host.Agent(self.config)
        self.agent.read_contract = self.agent.read_skill = self.agent.used_skill = True
        self.sample = dict(root='game', path='text.bin')

    def load_names(self):
        return self.agent.dispatch('load_script_metadata', dict(root='game', path='names.bin', key='names', source=NAMES))

    def test_each_focused_contract_fits_one_unmodified_tool_result(self):
        for topic in ('script', 'speaker', 'save'):
            with self.subTest(topic=topic):
                result = self.agent.dispatch('verification_contract', dict(topic=topic))
                self.assertLessEqual(len(host.CONTEXT['encode'](result)), host.CONTEXT['MAX_TOOL_BYTES'])
                self.assertEqual(host.CONTEXT['bounded_result'](self.agent.work, result), result)
        self.agent.workflow.stage = 'speaker_review'
        self.assertEqual(self.agent.dispatch('verification_contract', {})['topic'], 'speaker')
        complete = self.agent.dispatch('verification_contract', dict(topic='all'))
        self.assertTrue((self.agent.work / complete['full_contract']).is_file())

    def test_real_chain_retains_sample_through_name_invalidation_and_consumes_bindings(self):
        self.agent.dispatch('write_program', dict(kind='script', source=PARSER))
        self.agent.dispatch('test_program', dict(kind='script', **self.sample))
        self.assertEqual(self.agent.speaker_metadata_progress()['sample']['named_rows'], 0)
        names = self.load_names()
        self.assertEqual(names['binding_status'], 'global_names')
        self.assertEqual(self.agent.parts, {})
        progress = self.agent.speaker_metadata_progress()
        self.assertEqual(progress['sample']['path'], 'text.bin')
        self.assertEqual(progress['next_step'], 'bind_messages')
        self.assertEqual(self.agent.work_guidance()['stage'], 'speaker_review')
        self.agent.dispatch('load_script_metadata', dict(root='game', path='message.bin', key='messages',
                                                        source=MESSAGES, target_script=self.sample))
        self.assertEqual(self.agent.speaker_metadata_progress()['next_step'], 'retest_sample')
        result = self.agent.dispatch('test_program', dict(kind='script', **self.sample))
        self.assertEqual(result['preview'][0]['speaker'], 'Alice')
        self.assertEqual(self.agent.speaker_metadata_progress()['next_step'], 'ready_for_batch')
        self.agent.checkpoint([])
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text('utf8'))
        restored = host.Agent(self.config)
        restored.programs['script'] = PARSER
        self.assertTrue(restored.restore_checkpoint(saved))
        self.assertEqual(restored.representative_script, self.sample)

    def test_inspection_and_helper_receive_the_same_verified_global_and_selected_scoped_maps(self):
        self.load_names()
        self.agent.dispatch('load_script_metadata', dict(root='game', path='message.bin', key='messages',
                                                        source=MESSAGES, target_script=self.sample))
        code = "def extract(data, metadata): return metadata['speaker_resources']"
        global_only = self.agent.dispatch('inspect_data', dict(root='game', path='message.bin', source=code))
        scoped = self.agent.dispatch('inspect_data', dict(root='game', path='message.bin', source=code, target_script=self.sample))
        self.assertEqual(set(global_only), {'names'})
        self.assertEqual(set(scoped), {'names', 'messages'})
        self.assertEqual(scoped['messages']['message']['id'], 0)

    def test_empty_probe_is_explicitly_not_a_message_binding(self):
        result = self.agent.dispatch('load_script_metadata', dict(root='game', path='message.bin', key='probe',
            source="def extract(data, metadata): return {'values':{'count':1},'speakers':{}}", target_script=self.sample))
        self.assertEqual(result['binding_status'], 'analysis_only_no_speaker_bindings')
        self.assertEqual(result['verified_speakers'], 0)
        progress = self.agent.speaker_metadata_progress()
        self.assertEqual(progress['scoped_resources'], [])
        self.assertEqual(progress['empty_resources'][0]['key'], 'probe')

    def test_batch_preserves_structured_proof_diagnostics(self):
        self.load_names()
        originals, translated = self.root / 'companions', self.root / 'translated'
        originals.mkdir()
        translated.mkdir()
        (originals / 'chapter.src').write_bytes((self.root / 'message.bin').read_bytes())
        (translated / 'chapter.bin').write_bytes((self.root / 'text.bin').read_bytes())
        wrong = MESSAGES.replace("'text':record['text']", "'text':'Wrong'")
        result = self.agent.dispatch('load_script_metadata_batch', dict(root='game', path='companions',
            filter='*.src', source=wrong, target_root='game', target_path='translated', target_extension='.bin'))
        self.assertEqual(result['loaded'], 0)
        diagnostic = result['files'][0]['error_details']
        self.assertEqual(set(diagnostic), {'path', 'entry_index', 'expected', 'observed'})
        self.assertNotIn('Alice', json.dumps(diagnostic))

    def test_safe_unicode_character_classification_is_available_without_file_access(self):
        result = host.run_parser("def extract(data, metadata): return {'upper':'A'.isupper(),'number':'12'.isdecimal(),'fold':'A'.casefold()}", b'', {})
        self.assertEqual(result, dict(upper=True, number=True, fold='a'))
        with self.assertRaisesRegex(ValueError, 'Unavailable (name|attribute)'):
            host.run_parser("def extract(data, metadata): return open('secret').read()", b'', {})


if __name__ == '__main__':
    unittest.main()
