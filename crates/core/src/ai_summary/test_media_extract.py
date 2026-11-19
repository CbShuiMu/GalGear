"""独立媒体任务回归；只使用临时资源、模拟模型及随附离线转换器。"""
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

BASE = Path(__file__).parent
API = runpy.run_path(str(BASE / 'agent_host.py'))
MEDIA = runpy.run_path(str(BASE / 'media_agent.py'))
FILES = MEDIA['FILES']


class MediaTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.game, self.skill = self.root / 'game', self.root / 'skill'
        self.game.mkdir()
        self.skill.mkdir()
        (self.game/'game.exe').write_bytes(b'MZ fixture only')
        (self.skill/'SKILL.md').write_text('Installed extraction skill fixture', encoding='utf-8')
        self.config = dict(game=str(self.game), save=str(self.game), skill=str(self.skill),
            work=str(self.game/'.galgear-ai-summary'/'media-fixture'), kind='audio',
            provider='deepseek', model='fixture', token='never-sent', title='fixture', language='zh-CN')
        self.agent = MEDIA['MediaAgent'](self.config, API)

    def finish(self):
        return self.agent.dispatch('finish_media', dict(scope='Fixture audio selection', warnings=[]))

    def test_audio_preserves_paths_sources_and_repeat_exports(self):
        for folder, content in [('voice', b'OggS voice'), ('music', b'OggS music')]:
            directory = self.game/folder/'nested'
            directory.mkdir(parents=True)
            (directory/'same.ogg').write_bytes(content)
            self.agent.dispatch('export_media', dict(root='game', path=folder, pattern='**/*.ogg'))
        self.agent.dispatch('export_media', dict(root='game', path='voice', pattern='**/*.ogg'))
        self.assertEqual(self.finish()['files'], 2)
        manifest = json.loads((self.agent.delivery/'manifest.json').read_text('utf-8'))
        self.assertEqual({(self.agent.delivery/e['path']).read_bytes() for e in manifest['files']}, {b'OggS voice', b'OggS music'})
        self.assertTrue(all(e['path'].endswith('nested/same.ogg') for e in manifest['files']))
        self.assertEqual((self.game/'voice/nested/same.ogg').read_bytes(), b'OggS voice')

    def test_empty_wrong_kind_and_non_media_do_not_succeed(self):
        with self.assertRaisesRegex(ValueError, 'No media'):
            self.finish()
        (self.game/'cg.png').write_bytes(b'image')
        for path in ['game.exe', 'cg.png']:
            with self.assertRaisesRegex(ValueError, 'unrecognized media'):
                self.agent.dispatch('export_media', dict(root='game', path=path))
        self.assertFalse((self.agent.delivery/'manifest.json').exists())

    def test_changed_source_prevents_completion(self):
        path = self.game/'voice.ogg'
        path.write_bytes(b'OggS original')
        self.agent.dispatch('export_media', dict(root='game', path=path.name))
        path.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'changed'):
            self.finish()

    def test_absolute_traversal_and_other_task_inputs_are_rejected(self):
        for path in ['../secret.ogg', str(self.root/'secret.ogg'), 'C:/secret.ogg', '.galgear-ai-summary/media-fixture/delivery']:
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.agent.dispatch('export_media', dict(root='game', path=path))

    @unittest.skipUnless(os.name == 'nt', 'Windows junction boundary')
    def test_junction_cannot_export_outside_game(self):
        outside = self.root/'outside'
        outside.mkdir()
        (outside/'voice.ogg').write_bytes(b'private')
        junction = self.game/'linked'
        subprocess.run(['cmd', '/c', 'mklink', '/J', str(junction), str(outside)], check=True, capture_output=True)
        self.addCleanup(lambda: junction.rmdir())
        with self.assertRaisesRegex(ValueError, 'reparse'):
            self.agent.dispatch('export_media', dict(root='game', path='linked', pattern='*.ogg'))

    def test_model_loop_uses_installed_skill_and_only_actual_output_can_finish(self):
        for index in range(8):
            (self.game/('voice'+str(index)+'.ogg')).write_bytes(b'OggS'+bytes([index])*32)
        responses = [
            [('test_extraction', dict(mode='files',path='.',pattern='*.ogg'))],
            [('start_batch', dict(tests=['sample-1'],scope='本地音频',warnings=[]))],
        ]
        def complete(config, messages, available, **kwargs):
            self.assertIn('Installed extraction skill fixture', messages[1]['content'])
            self.assertNotIn('never-sent', json.dumps(messages))
            return dict(message=dict(role='assistant', content=None, tool_calls=[dict(id='c'+str(i),type='function',function=dict(name=name,arguments=json.dumps(args))) for i,(name,args) in enumerate(responses.pop(0))]))
        with patch.dict(API['TRANSPORT'], complete=complete):
            result = self.agent.run()
        self.assertEqual(result['tested_samples'], 3)
        self.assertEqual(len(self.agent.files), 3)
        self.assertFalse((self.agent.work/'delivery').exists())
        self.assertEqual(json.loads((self.agent.work/'session.json').read_text())['status'], 'completed')
        self.assertNotIn('never-sent', (self.agent.work/'conversation.json').read_text('utf-8'))
        batch=runpy.run_path(str(BASE/'media_batch.py'))
        config={key:value for key,value in self.config.items() if key not in ('token','provider','model')}
        with patch.dict(API['TRANSPORT'],complete=lambda *args,**kwargs: self.fail('Batch script must never call AI')):
            result=batch['run'](config,API)
        self.assertEqual(result['files'],8)
        progress=json.loads((self.agent.work/'progress.json').read_text())
        self.assertEqual(progress['batch'],dict(completed=1,total=1,files=8))
        self.assertEqual(json.loads((self.agent.work/'ai-session.json').read_text())['requests'],2)

    def test_untested_recipe_cannot_start_batch_and_batch_rejects_credentials(self):
        with self.assertRaisesRegex(ValueError,'successful test'):
            self.agent.dispatch('start_batch',dict(tests=['sample-1'],scope='test',warnings=[]))
        batch=runpy.run_path(str(BASE/'media_batch.py'))
        with self.assertRaisesRegex(ValueError,'credentials'):
            batch['run'](self.config,API)

    def test_dialogue_batch_exports_verified_speaker_table_and_links_native_ids(self):
        config = dict(self.config, kind='dialogue', work=str(self.agent.work.parent/'media-dialogue'))
        agent = MEDIA['MediaAgent'](config, API)
        (self.game/'names.bin').write_bytes(b'\x07Alice')
        for index in range(5):
            (self.game/f'scene{index}.bin').write_bytes(
                b'\x07' + f'第 {index} 句台词'.encode() + b'\n\x00Narration')
        old = self.game/'dialogues/media-old'
        old.mkdir(parents=True)
        (old/'manifest.json').write_text('{}')
        (old/'scene-old.bin').write_bytes(b'not a script')

        table = agent.dispatch('test_extraction', dict(mode='files',path='.',pattern='names.bin',
            dialogue=dict(reader='speakers', program='''def extract(data, metadata):
    return {'values':{},'speakers':{'character':{'id':data[0],'text':data[1:].decode('ascii'),
        'id_evidence':{'offset':0,'width':1},'evidence':{'offset':1,'length':len(data)-1,'encoding':'ascii'}}}}
''')))['test']
        with self.assertRaisesRegex(ValueError, 'at least one verified dialogue script'):
            agent.dispatch('start_batch', dict(tests=[table],scope='Names only',warnings=[]))
        script = '''def extract(data, metadata):
    rows = []
    offset = 0
    for line in data.split(b"\\n"):
        proof = {'offset':offset+1,'length':len(line)-1,'encoding':'utf-8'}
        speaker = None
        if line[0] != 0:
            speaker = metadata['speaker_resources']['sample-1']['character']['text']
            proof['speaker'] = {'resource':'sample-1','entry':'character','reference':{'offset':offset,'width':1}}
        rows.append({'position':offset+1,'speaker':speaker,'text':line[1:].decode('utf-8'),'evidence':proof})
        offset += len(line)+1
    return rows
'''
        sample = agent.dispatch('test_extraction', dict(mode='files',path='.',pattern='**/scene*.bin',
            dialogue=dict(reader='script',program=script,speaker_tests=[table])))
        self.assertEqual((sample['selected'], sample['tested']), (5, 3))
        with self.assertRaisesRegex(ValueError, 'companion table tests'):
            agent.dispatch('start_batch', dict(tests=[sample['test']],scope='Missing table',warnings=[]))
        agent.dispatch('start_batch', dict(tests=[table,sample['test']],scope='Five scenes and native name table',warnings=[]))
        self.assertFalse((agent.work/'delivery').exists())

        batch = runpy.run_path(str(BASE/'media_batch.py'))
        batch_config = {key:value for key,value in config.items() if key not in ('token','provider','model')}
        with patch.dict(API['TRANSPORT'],complete=lambda *args,**kwargs: self.fail('Batch script must never call AI')):
            self.assertEqual(batch['run'](batch_config, API)['files'], 6)
        delivery = agent.work/'delivery'
        manifest = json.loads((delivery/'manifest.json').read_text('utf-8'))
        self.assertEqual(manifest['kind'], 'dialogue')
        for member in manifest['files']:
            path = delivery/member['path']
            self.assertEqual(member['sha256'], hashlib.sha256(path.read_bytes()).hexdigest())
            result = json.loads(path.read_text('utf-8'))
            if path.name == 'speakers.json':
                self.assertEqual(result['entries'], {'character':{'id':7,'text':'Alice'}})
                self.assertEqual(result['source']['member'], 'names.bin')
            else:
                index = result['source']['member'].removeprefix('scene').removesuffix('.bin')
                self.assertEqual([(row['text'],row['speaker']) for row in result['rows']],
                                 [(f'第 {index} 句台词','Alice'),('Narration',None)])
        self.assertEqual((old/'scene-old.bin').read_bytes(), b'not a script')

    def test_dialogue_without_name_table_rejects_fabricated_text_and_allows_narration(self):
        config = dict(self.config, kind='dialogue', work=str(self.agent.work.parent/'media-narration'))
        agent = MEDIA['MediaAgent'](config, API)
        (self.game/'scene.txt').write_bytes(b'Narration')
        script = """def extract(data, metadata):
    return [{'position':0,'speaker':None,'text':data.decode('ascii'),
             'evidence':{'offset':0,'length':len(data),'encoding':'ascii'}}]
"""
        args = dict(mode='files',path='.',pattern='scene.txt')
        with self.assertRaisesRegex(ValueError, 'match|byte'):
            agent.dispatch('test_extraction', dict(args,
                dialogue=dict(reader='script',program=script.replace("data.decode('ascii')", "'Fabricated'"))))
        sample = agent.dispatch('test_extraction', dict(args, dialogue=dict(reader='script',program=script)))
        agent.dispatch('start_batch', dict(tests=[sample['test']],scope='One scene',warnings=[]))
        batch = runpy.run_path(str(BASE/'media_batch.py'))
        batch_config = {key:value for key,value in config.items() if key not in ('token','provider','model')}
        self.assertEqual(batch['run'](batch_config, API)['files'], 1)
        self.assertFalse((agent.work/'delivery/speakers').exists())

    def test_archive_sample_and_batch_share_case_insensitive_mod_selection(self):
        archive = self.game/'voice.arc'
        archive.write_bytes(b'archive fixture')
        tool = self.skill/'tools/GARbro-cli'
        tool.mkdir(parents=True)
        (tool/'GARbro.Console.exe').touch()
        (tool/'provenance.json').write_text('{"distribution":"GARbro-Mod-1.0.2.2"}')
        guide = self.skill/'commands/garbro.md'
        guide.parent.mkdir()
        guide.write_text('Fixture CLI documentation')
        self.agent.dispatch('read_text', dict(root='skill', path='commands/garbro.md'))
        selected = ['a.ogg', 'b.OGG', 'c.Ogg', 'd.ogg']
        listing = ''.join(f' 36 [{i:08x}] {name}\n' for i, name in
                          enumerate([*selected, 'script.txt'])).encode()
        extraction_calls = []

        def run(argv, **kwargs):
            if len(argv) == 2:
                return subprocess.CompletedProcess(argv, 0, listing, b'')
            extraction_calls.append(argv[2:])
            for name in argv[2:]:
                (kwargs['cwd']/name).write_bytes(b'OggS' + bytes(32))
            return subprocess.CompletedProcess(argv, 0, b'', b'')

        batch = runpy.run_path(str(BASE/'media_batch.py'))
        config = {key: value for key, value in self.config.items()
                  if key not in ('token', 'provider', 'model')}
        with patch.object(subprocess, 'run', side_effect=run):
            sample = self.agent.dispatch('test_extraction', dict(mode='archive', path=archive.name, pattern='*.ogg'))
            self.assertEqual(sample['selected'], 4)
            self.assertEqual(sample['tested'], 3)
            self.agent.dispatch('start_batch', dict(tests=[sample['test']], scope='Mixed-case audio', warnings=[]))
            self.assertEqual(batch['run'](config, API)['files'], 4)
        self.assertEqual(extraction_calls, [['a.ogg'], ['c.Ogg'], ['d.ogg'], selected])

    def test_archive_inventory_distinguishes_missing_and_unexpected_members(self):
        verify = MEDIA['PLAN']['verify_archive_members']
        for actual, diagnostic in [
            ([], r'missing 1: scene\.png\)$'),
            (['scene.png', 'extra.PNG'], r'unexpected 1: extra\.PNG\)$'),
            (['extra.PNG'], r'missing 1: scene\.png; unexpected 1: extra\.PNG\)$'),
        ]:
            with self.subTest(actual=actual), self.assertRaisesRegex(ValueError, diagnostic):
                verify('images.arc', ['scene.png'], actual)

    def test_converter_partial_failures_are_retained_even_if_model_omits_them(self):
        self.agent.kind = 'cg'
        (self.game/'cg.native').write_bytes(b'native fixture')
        executable = self.skill/'tools/GARbro-cli/GalImageExport.exe'
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b'fixture')
        guide = self.skill/'commands/garbro-images.md'
        guide.parent.mkdir()
        guide.write_text('fixture guide')
        self.agent.dispatch('read_text', dict(root='skill',path='commands/garbro-images.md'))
        def convert(command, **kwargs):
            job = json.loads(Path(command[1]).read_text())
            output = Path(job['output'])
            output.mkdir()
            native = output/'cg.native'
            native.write_bytes(b'native fixture')
            entries = [dict(kind='raw',path=str(native),sha256=hashlib.sha256(native.read_bytes()).hexdigest()), dict(kind='decode_error',name='cg.native',error='unsupported fixture')]
            (output/'manifest.jsonl').write_text('\n'.join(json.dumps(e) for e in entries))
            return subprocess.CompletedProcess(command, 0, b'', b'')
        with patch.object(FILES['subprocess'], 'run', side_effect=convert):
            result = self.agent.dispatch('convert_images', dict(root='game',path='cg.native',mode='files'))
        self.assertEqual(result['failed'], 1)
        self.finish()
        self.assertEqual(len(json.loads((self.agent.delivery/'manifest.json').read_text())['warnings']), 1)

    @unittest.skipUnless(os.name == 'nt', 'Bundled Windows converter')
    def test_bundled_image_converter_exports_real_png_without_model_or_network(self):
        skill = BASE.parents[3]/'Galgear-GalgameReverse-Skill/GalgameResourceExtract'
        if not (skill/'tools/GARbro-cli/GalImageExport.exe').is_file():
            self.skipTest('Bundled image converter not installed')
        self.agent.kind = 'cg'
        self.agent.host.roots['skill'] = skill.resolve()
        path = self.game/'sample.png'
        # Valid 1x1 RGBA PNG; all files are temporary and unrelated to a real game.
        import struct
        import zlib
        def chunk(name, data):
            return struct.pack('>I',len(data))+name+data+struct.pack('>I',zlib.crc32(name+data))
        png = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0\xff\0\0\xff'))+chunk(b'IEND',b'')
        path.write_bytes(png)
        self.agent.dispatch('read_text',dict(root='skill',path='commands/garbro-images.md'))
        result = self.agent.dispatch('convert_images',dict(root='game',path=path.name,mode='files'))
        self.assertEqual(result['failed'], 0)
        self.assertGreaterEqual(result['files'], 2)
        self.finish()
        self.assertEqual(path.read_bytes(), png)


if __name__ == '__main__':
    unittest.main()
