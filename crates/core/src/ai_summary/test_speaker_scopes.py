"""Same numeric message IDs in different scripts must never share speakers."""
import copy
import importlib.util
import json
import pathlib
import struct
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('scoped_speaker_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)

def companion_bytes(native=0, name='', message='Original fixture message'):
    """A bounded native Name→Message chain; no arbitrary NUL stand-ins."""
    return (bytes([0x15]) + name.encode('cp932') + bytes([0, 0, 0x14])
            + struct.pack('<I', native) + bytes([0]) + message.encode('cp932') + bytes([0, 0]))


HELPER = '''def extract(data, metadata):
    name_end = data.index(bytes([0]), 1)
    message = name_end + 2
    native = int.from_bytes(data[message+1:message+5], 'little')
    name = data[1:name_end].decode('cp932')
    return {'values':{},'speakers':{'message':{'id':native,'text':name if name else None,
        'id_evidence':{'offset':message+1,'width':4,'endian':'little'},
        'evidence':{'offset':1,'length':name_end-1 if name else 1,'encoding':'cp932'},
        'native_link':{'kind':'willplus_ws2_name_message_config1_cp932',
            'name_command_offset':0,'message_command_offset':message}}}}
'''

PARSER = '''def extract(data, metadata):
    count = int.from_bytes(data[:4],'little')
    rows = []
    by_id = {}
    for resource, records in metadata['speaker_resources'].items():
        for entry, record in records.items():
            by_id.setdefault(record['id'], (resource,entry,record['text']))
    pos = 4
    for native in range(count):
        length = int.from_bytes(data[pos:pos+4],'little')
        pos += 4
        text = data[pos:pos+length].decode('ascii')
        pos += length
        hit = by_id.get(native)
        proof = {'table':'lines','index':native,'reference':{'mode':'ordinal'}}
        if hit:
            proof['speaker'] = {'resource':hit[0],'entry':hit[1],'reference':{'mode':'table_index'}}
        rows.append({'position':native,'text':text,'speaker':hit[2] if hit else None,'evidence':proof})
    return {'rows':rows,'tables':{'lines':{'offset':0,'length':len(data),'fields':[{'kind':'text','encoding':'ascii'}]}}}
'''


class ScriptSpeakerScopeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root/'game.exe').write_bytes(b'fixture')
        self.agent = host.Agent(dict(game=str(self.root),save=str(self.root),skill=str(self.root),
                                     work=str(self.root/'work'),language='en-US'))
        self.agent.read_contract = self.agent.read_skill = True
        self.agent.dispatch('write_program',dict(kind='script',source=PARSER))
        for name,speaker in [('first','Alice'),('second','Yuka'),('narration','')]:
            (self.root/(name+'.ws2')).write_bytes(companion_bytes(0, speaker))
            text = ('Message '+name).encode()
            (self.root/(name+'.lng')).write_bytes(struct.pack('<II',1,len(text))+text)

    def load(self,name):
        return self.agent.dispatch('load_script_metadata',dict(root='game',path=name+'.ws2',key=name,source=HELPER,
            target_script=dict(root='game',path=name+'.lng')))

    def test_same_native_id_in_other_script_resolves_its_own_yuka(self):
        self.load('first')
        self.load('second')
        result = self.agent.dispatch('test_program',dict(kind='script',root='game',path='second.lng'))
        self.assertEqual(result['preview'][0]['speaker'],'Yuka')
        self.assertEqual(self.agent.speaker_parts[str(self.root/'second.lng')]['external'],1)

    def test_missing_own_companion_is_rejected_instead_of_copying_another_script(self):
        self.load('first')
        with self.assertRaisesRegex(ValueError,'matching original-script'):
            self.agent.dispatch('test_program',dict(kind='script',root='game',path='second.lng'))

    def test_available_name_cannot_be_silently_dropped_as_narration(self):
        self.load('second')
        data = (self.root/'second.lng').read_bytes()
        value = host.run_parser(PARSER,data,dict(speaker_resources=self.agent.speaker_resources))
        value['rows'][0]['speaker'] = None
        value['rows'][0]['evidence'].pop('speaker')
        with self.assertRaisesRegex(ValueError,'silently replace'):
            host.EVIDENCE['script_rows'](data,value,speaker_resources=self.agent.speaker_resources,source=str(self.root/'second.lng'))

    def test_empty_name_byte_proof_preserves_real_narration(self):
        self.load('narration')
        result = self.agent.dispatch('test_program',dict(kind='script',root='game',path='narration.lng'))
        self.assertIsNone(result['preview'][0]['speaker'])
        status = self.agent.speaker_status()
        self.assertFalse(status['speaker_review_required'])
        self.assertEqual(status['speaker_verification']['narration_rows'],1)
        self.assertEqual(status['speaker_verification']['unresolved_rows'],0)

    def test_batch_pairs_each_original_with_its_own_transcript(self):
        result = self.agent.dispatch('load_script_metadata_batch',dict(root='game',path='.',filter='*.ws2',source=HELPER,
            target_root='game',target_path='.',target_extension='.lng'))
        self.assertEqual(result['loaded'],3)
        batch = self.agent.dispatch('test_program_batch',dict(root='game',path='.',filter='*.lng'))
        self.assertTrue(all('error' not in item for item in batch['files']))
        self.assertEqual({pathlib.Path(path).stem:rows[0]['speaker'] for path,rows in self.agent.parts.items()},
                         dict(first='Alice',second='Yuka',narration=None))

    def test_scope_cannot_be_reused_by_a_different_source_with_same_id(self):
        self.load('first')
        raw = (self.root/'first.lng').read_bytes()
        output = host.run_parser(PARSER,raw,dict(speaker_resources=self.agent.speaker_resources))
        with self.assertRaisesRegex(ValueError,'local to each script'):
            host.EVIDENCE['script_rows'](raw,output,speaker_resources=self.agent.speaker_resources,source=str(self.root/'second.lng'))

    def test_current_save_chapter_does_not_satisfy_complete_archive_coverage(self):
        self.agent.dispatch('load_script_metadata_batch',dict(root='game',path='.',filter='*.ws2',source=HELPER,
            target_root='game',target_path='.',target_extension='.lng'))
        archive = self.root/'dialogue.pack'
        archive.write_bytes(b'archive fixture with three chapter members')
        self.agent.extracted_from[str(self.root)] = str(archive)
        self.agent.extraction_stamps[str(self.root)] = self.agent.file_fingerprint(archive)
        listing = '\n'.join('[{:08X}] {} {}.lng'.format(index * 32,
                            (self.root/(name+'.lng')).stat().st_size,name)
                            for index,name in enumerate(('first','second','narration')))
        original_dispatch = self.agent._dispatch

        def dispatch(name,args):
            if name == 'run_skill':
                self.assertEqual(args,dict(command='l',archive=archive.name))
                return dict(exit_code=0,stdout=listing,stderr='')
            return original_dispatch(name,args)

        with mock.patch.object(self.agent,'_dispatch',side_effect=dispatch):
            self.agent.dispatch('test_program',dict(kind='script',root='game',path='first.lng'))
            coverage = self.agent.transcript_coverage()
            self.assertEqual(coverage['covered_members'],1)
            self.assertEqual({item['member'] for item in coverage['pending_members']},{'second.lng','narration.lng'})
            self.agent.dispatch('test_program_batch',dict(root='game',path='.',filter='*.lng'))
        self.assertEqual(self.agent.transcript_coverage()['pending_members'],[])

    def test_verified_named_message_cannot_be_omitted_from_transcript(self):
        self.load('second')
        records = self.agent.speaker_resources['second']
        records['another'] = dict(id=1,text='Yuka',_source=str(self.root/'second.lng'))
        texts = [b'First',b'Second']
        raw = struct.pack('<I',2)+b''.join(struct.pack('<I',len(text))+text for text in texts)
        output = host.run_parser(PARSER,raw,dict(speaker_resources=dict(second=records)))
        output['rows'].pop()
        with self.assertRaisesRegex(ValueError,'omitted verified native messages'):
            host.EVIDENCE['script_rows'](raw,output,speaker_resources=dict(second=records),source=str(self.root/'second.lng'))

    def test_legacy_unscoped_numeric_metadata_cannot_override_matching_scope(self):
        self.agent.speaker_resources['legacy'] = dict(message=dict(id=0,text='Alice'))
        self.load('second')
        result = self.agent.dispatch('test_program',dict(kind='script',root='game',path='second.lng'))
        self.assertEqual(result['preview'][0]['speaker'],'Yuka')

    def test_duplicate_verified_aliases_for_same_script_and_name_are_equivalent(self):
        self.load('second')
        self.agent.speaker_resources['alias'] = copy.deepcopy(self.agent.speaker_resources['second'])
        self.agent.speaker_resource_targets['alias'] = str(self.root/'second.lng')
        result = self.agent.dispatch('test_program',dict(kind='script',root='game',path='second.lng'))
        self.assertEqual(result['preview'][0]['speaker'],'Yuka')


if __name__ == '__main__':
    unittest.main()
