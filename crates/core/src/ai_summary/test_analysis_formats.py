"""未知格式工具的权限、真实字节验证与源码研究回归，使用隔离夹具。"""
import hashlib
import io
import json
from pathlib import Path
import runpy
import tempfile
import sys
import time
import types
import unittest
import zipfile
from unittest.mock import patch

BASE = Path(__file__).parent
API = runpy.run_path(str(BASE/'agent_host.py'))
FORMATS = runpy.run_path(str(BASE/'analysis_formats.py'))


class FormatTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.input = self.root/'archive.bin'
        self.input.write_bytes(b'fixture payload')
        self.config = dict(inputs=[dict(id='archive',root='game',path='archive.bin',absolute=str(self.input))],
            output=str(self.root/'out'), source='def analyze(inputs, metadata):\n return {"files":{"story/scene.txt": inputs["archive"]}, "result":{"count":1}}')

    def test_output_hashes_and_original_preserved(self):
        before = self.input.read_bytes()
        result = FORMATS['worker'](self.config, API)
        self.assertEqual(result['count'],1)
        self.assertEqual(self.input.read_bytes(),before)
        receipt=json.loads((self.root/'out.json').read_text('utf-8'))
        self.assertEqual(receipt['members'][0]['sha256'],hashlib.sha256(before).hexdigest())
        self.assertEqual((self.root/'out/story/scene.txt').read_bytes(),before)

    def test_plain_probe_object_is_not_silently_discarded(self):
        self.config['source']='def analyze(inputs, metadata):\n return {"count":len(inputs["archive"]),"header":inputs["archive"][:7].hex()}'
        result=FORMATS['worker'](self.config,API)
        self.assertEqual(result['result']['count'],15)
        self.assertEqual(result['result']['header'],b'fixture'.hex())
        self.assertEqual(result['count'],0)

    def test_plain_probe_list_is_preserved(self):
        self.config['source']='def analyze(inputs, metadata): return [len(inputs["archive"]), "header"]'
        self.assertEqual(FORMATS['worker'](self.config,API)['result'],[15,'header'])

    def test_large_input_mapping_supports_original_parser_and_releases_handles(self):
        self.config['source']='import struct\ndef analyze(inputs, metadata):\n d=inputs["archive"]\n return {"result":struct.unpack_from("<I",d,0)[0],"files":{"tail.bin":d[-7:]}}'
        with patch.dict(FORMATS['_worker'].__globals__,MAX_INPUT=8):
            result=FORMATS['worker'](self.config,API)
        self.assertEqual((self.root/'out/tail.bin').read_bytes(),b'payload')
        self.assertEqual(result['input_headers'][0]['access'],'mapped_read_only')
        self.assertEqual(result['input_headers'][0]['sha256'],hashlib.sha256(self.input.read_bytes()).hexdigest())
        self.input.rename(self.root/'moved.bin')  # Windows 上仍存活的映射会阻止该操作。

    def test_mapped_input_is_readonly_and_exception_releases_it(self):
        self.config['source']='def analyze(inputs, metadata):\n inputs["archive"][0]=65\n return {}'
        with patch.dict(FORMATS['_worker'].__globals__,MAX_INPUT=8):
            with self.assertRaisesRegex(RuntimeError,'readonly memory map'):FORMATS['worker'](self.config,API)
        self.assertEqual(self.input.read_bytes(),b'fixture payload')
        self.input.rename(self.root/'moved.bin')

    def test_unaligned_large_range_retains_source_coordinates(self):
        self.config['inputs'][0].update(offset=1,length=10)
        self.config['source']='def analyze(inputs, metadata): return {"files":{"part.bin":inputs["archive"][2:5]}}'
        with patch.dict(FORMATS['_worker'].__globals__,MAX_INPUT=8):
            result=FORMATS['worker'](self.config,API)
        self.assertEqual((self.root/'out/part.bin').read_bytes(),b'tur')
        self.assertEqual(result['input_headers'][0]['offset'],1)

    def test_bounded_input_range_preserves_container_coordinates(self):
        with self.input.open('wb') as stream:
            stream.seek(FORMATS['MAX_INPUT']+100)
            stream.write(b'index')
        self.config['inputs'][0].update(offset=FORMATS['MAX_INPUT']+100,length=5)
        self.config['source']='def analyze(inputs, metadata): return {"text":inputs["archive"].decode("ascii"),"range":metadata["input_ranges"]["archive"]}'
        result=FORMATS['worker'](self.config,API)['result']
        self.assertEqual(result['text'],'index')
        self.assertEqual(result['range'],dict(offset=FORMATS['MAX_INPUT']+100,length=5,file_size=FORMATS['MAX_INPUT']+105))
        receipt=json.loads((self.root/'out.json').read_text('utf-8'))['inputs'][0]
        self.assertEqual(receipt['sha256'],hashlib.sha256(b'index').hexdigest())
        self.assertEqual(receipt['size'],5)

    def test_input_range_rejects_invalid_or_out_of_bounds_ranges(self):
        for offset,length in [(-1,1),(0,0),(0,100),(1,True),(True,1),(15,1)]:
            with self.subTest(offset=offset,length=length):
                self.config['inputs'][0].update(offset=offset,length=length)
                with self.assertRaisesRegex(ValueError,'Input range'):FORMATS['worker'](self.config,API)

    def test_unsafe_names_and_collisions_emit_nothing(self):
        for names in [('../escape',),('C:/escape',),('a','A'),('folder','folder/file'),('CON',)]:
            with self.subTest(names=names):
                self.config['source']='def analyze(inputs, metadata):\n return {"files":'+repr({n:b'x' for n in names})+'}'
                with self.assertRaises(ValueError): FORMATS['worker'](self.config,API)
                self.assertFalse((self.root/'out').exists())

    def test_model_cannot_open_files_or_import_process_modules(self):
        for expression in ['open("anything")','__import__("os")','inputs.__class__']:
            self.config['source']='def analyze(inputs, metadata):\n return '+expression
            with self.assertRaises(ValueError): FORMATS['worker'](self.config,API)

    def test_ordered_dictionary_is_available_to_pure_format_programs(self):
        self.config['source']='import collections\ndef analyze(inputs, metadata):\n d=collections.OrderedDict([(1,"one"),(2,"two")])\n d.move_to_end(1)\n return {"result":list(d.values())}'
        self.assertEqual(FORMATS['worker'](self.config,API)['result'],['two','one'])

    def test_dictionary_fromkeys_is_available_for_ordered_deduplication(self):
        self.config['source']='def analyze(inputs, metadata): return {"result":list(dict.fromkeys([3,1,3,2]))}'
        self.assertEqual(FORMATS['worker'](self.config,API)['result'],[3,1,2])

    def test_all_empty_outputs_cannot_count_as_decoded_scripts(self):
        self.config['source']='def analyze(inputs, metadata):\n return {"files":{"a.txt":b"","b.txt":b""}}'
        with self.assertRaisesRegex(ValueError,'Every emitted member is empty'):
            FORMATS['worker'](self.config,API)
        self.assertFalse((self.root/'out').exists())

    def test_roundtrip_compares_bytes_before_any_output(self):
        self.config['verify_roundtrip']=True
        self.config['source']+='\ndef pack(files, metadata):\n files.clear()\n return b"bad"\ndef unpack(data, metadata):\n return {"story/scene.txt": b"bad"}'
        with self.assertRaisesRegex(ValueError,'roundtrip failed'): FORMATS['worker'](self.config,API)
        self.assertFalse((self.root/'out').exists())

    def test_self_roundtrip_is_not_runtime_validation(self):
        self.config['verify_roundtrip']=True
        self.config['source']+='\ndef pack(files, metadata):\n return files["story/scene.txt"]\ndef unpack(data, metadata):\n return {"story/scene.txt":data}'
        value=FORMATS['worker'](self.config,API)
        self.assertEqual(value['roundtrip']['level'],'self_roundtrip')

    def test_standard_blowfish_vector_and_alignment(self):
        self.assertEqual(FORMATS['cipher']('blowfish',bytes(8),bytes(8),encrypt=True).hex(),'4ef997456198dd78')
        with self.assertRaises(ValueError): FORMATS['cipher']('blowfish',b'x',bytes(8))

    def test_reference_primitive_rejects_arbitrary_native_calls(self):
        with self.assertRaises(ValueError):FORMATS['reference_primitive']('System.IO.File',[])
        with self.assertRaises(ValueError):FORMATS['reference_primitive']('garbro_mt19937',[dict(seed=-1)])
        with self.assertRaises(ValueError):FORMATS['reference_primitive']('garbro_blowfish',[dict(key=b'abcd',data=b'x')])
        with self.assertRaises(ValueError):FORMATS['reference_primitive']('garbro_crc32normal_table',[dict(path='arbitrary')])

    def test_valid_native_computations_are_not_stopped_at_arbitrary_call_count(self):
        from unittest.mock import Mock
        primitive=Mock(return_value=[[1]])
        with patch.dict(FORMATS['install_primitives'].__globals__,reference_primitive=primitive):
            FORMATS['install_primitives'](API)
            entry=API['MODULES']['galgear_binary'].reference_primitive
            for i in range(20):self.assertEqual(entry('garbro_mt19937',[dict(seed=i)]),[[1]])
        self.assertEqual(primitive.call_count,20)

    def test_empty_batches_and_empty_cipher_members_are_identity_operations(self):
        self.assertEqual(FORMATS['reference_primitive']('garbro_mt19937',[]),[])
        self.assertEqual(FORMATS['reference_primitive']('garbro_blowfish',[dict(key=b'abcd',data=b'')]),[b''])

    def test_source_search_and_member_read(self):
        archive=self.root/'source.zip'
        with zipfile.ZipFile(archive,'w') as output:
            output.writestr('code/format.cs','public int ParseHeader() { return 7; }')
        agent=types.SimpleNamespace(host=types.SimpleNamespace(roots=dict(skill=self.root),path=lambda root,path:self.root/path))
        value=FORMATS['read_reference'](agent,dict(path='source.zip',search='ParseHeader'))
        self.assertEqual(value['matches'][0]['member'],'code/format.cs')
        value=FORMATS['read_reference'](agent,dict(path='source.zip',member='code/format.cs',offset=7,length=3))
        self.assertEqual(value['text'],'int')
        self.assertEqual(FORMATS['read_reference'](agent,dict(path='source.zip',member='format.cs',offset=7,length=3))['text'],'int')

    def test_existing_output_is_never_overwritten(self):
        (self.root/'out').mkdir()
        with self.assertRaises(FileExistsError):FORMATS['worker'](self.config,API)

    def test_reference_includes_actual_helper_even_when_filename_differs(self):
        archive=self.root/'source.zip'
        with zipfile.ZipFile(archive,'w') as output:
            output.writestr('code/archive.cs','class Archive { uint Decode() { return OddChecksum.Table[0] + new Generator(1).Next(); } }')
            output.writestr('code/checksums.cs','class OddChecksum { public static uint[] Table = { 17, 29 }; }')
            output.writestr('code/Generator.cs','class Generator { public uint Next() { return 63; } }')
            output.writestr('code/unrelated.cs','class Other { }')
        agent=types.SimpleNamespace(host=types.SimpleNamespace(roots=dict(skill=self.root),path=lambda root,path:self.root/path))
        value=FORMATS['read_reference'](agent,dict(path='source.zip',member='code/archive.cs',length=20))
        self.assertEqual({d['member'] for d in value['dependencies']},{'code/checksums.cs','code/Generator.cs'})
        self.assertTrue(any('17, 29' in d['text'] for d in value['dependencies']))
        repeated=FORMATS['read_reference'](agent,dict(path='source.zip',member='code/archive.cs',offset=20))
        self.assertTrue(all(d['already_supplied'] and not d['text'] for d in repeated['dependencies']))

    def test_nested_helper_preserves_called_signature_when_full_type_exceeds_budget(self):
        archive=self.root/'nested.zip'
        with zipfile.ZipFile(archive,'w') as output:
            output.writestr('code/archive.cs','class Archive { void Read() { var file = new ResourceFile.Accessor(path); file.GetResource("item", "kind"); } }')
            output.writestr('code/ResourceFile.cs','class ResourceFile {\n'+'// padding\n'*4000+'public byte[] GetResource(string name, string type) { return null; }\n}')
        with zipfile.ZipFile(archive) as source:
            result=FORMATS['source_dependencies'](source,'code/archive.cs',source.read('code/archive.cs').decode())
        self.assertIn('GetResource(string name, string type)',result[0]['text'])
        self.assertTrue(result[0]['excerpt'])
        self.assertTrue(result[0]['requires_read'])

    def test_reference_maps_only_known_library_primitives(self):
        source='using GameRes.Cryptography; using GameRes.Utility; class Archive { void Read() { new Blowfish(key).Decipher(bytes); new MersenneTwister(seed).Rand(); var t = Crc32Normal.Table; } }'
        bindings=FORMATS['primitive_bindings'](source)
        self.assertEqual(len(bindings),3)
        self.assertIn('wrapped key/passphrase',bindings['GameRes.Cryptography.Blowfish.Decipher'])
        self.assertEqual(FORMATS['primitive_bindings']('class Blowfish { }'),{})

    def test_remote_source_is_pinned_documented_bounded_and_cached(self):
        from unittest.mock import Mock
        skill,work=self.root/'skill',self.root/'work'
        skill.mkdir();work.mkdir()
        revision='a'*40
        url='https://github.com/example/reader/blob/'+revision+'/source.cs'
        (skill/'SKILL.md').write_text('[Source]('+url+')',encoding='utf-8')
        roots=dict(skill=skill,work=work)
        agent=types.SimpleNamespace(api=API,deadline=time.monotonic()+60,
            host=types.SimpleNamespace(roots=roots,path=lambda root,path:roots[root]/path))
        opener=Mock()
        opener.open.return_value=io.BytesIO(b'class Reader { }')
        with patch.object(FORMATS['urllib'].request,'build_opener',return_value=opener):
            first=FORMATS['read_reference'](agent,dict(url=url,length=5))
            second=FORMATS['read_reference'](agent,dict(url=url,offset=5))
            self.assertEqual(first['text']+second['text'],'class Reader { }')
            self.assertEqual(opener.open.call_count,1)
            request=opener.open.call_args.args[0]
            self.assertEqual(request.full_url,'https://raw.githubusercontent.com/example/reader/'+revision+'/source.cs')
            self.assertIsNone(request.get_header('Authorization'))
            for forbidden in [url.replace(revision,'b'*40),url.replace(revision,'main'),
                              url+'?token=secret',url.replace('github.com','127.0.0.1'),url.replace('/source.cs','/%2e%2e/secret')]:
                with self.subTest(forbidden=forbidden),self.assertRaises(ValueError):
                    FORMATS['read_reference'](agent,dict(url=forbidden))
            opener.open.return_value=io.BytesIO(b'x'*(512*1024+1))
            with self.assertRaisesRegex(ValueError,'512 KiB'):
                FORMATS['read_reference'](agent,dict(url=url.replace('source.cs','large.cs')))

    def test_remote_partial_reader_includes_real_layout_definitions(self):
        from unittest.mock import Mock
        revision='a'*40
        url='https://raw.githubusercontent.com/example/reader/'+revision+'/Extract/Scene.Extract.cs'
        self.root.joinpath('work').mkdir()
        agent=types.SimpleNamespace(api=API,deadline=time.monotonic()+60,reference_revisions={('example','reader',revision)},
            host=types.SimpleNamespace(path=lambda root,path:self.root/root/path))
        tree=dict(tree=[dict(type='blob',path='Extract/Scene.Extract.cs'),
                        dict(type='blob',path='Extract/Structs/Scene.Structs.cs')])
        opener=Mock()
        opener.open.side_effect=[io.BytesIO(b'partial class Scene { void Read() { ReadUnmanaged<HEADER>(); } }'),
                                 FORMATS['urllib'].error.URLError('transient tree connection'),
                                 io.BytesIO(json.dumps(tree).encode()),
                                 io.BytesIO(b'partial class Scene { struct HEADER { int InputGroups; int RecordOffsets; } }')]
        with patch.object(FORMATS['urllib'].request,'build_opener',return_value=opener):
            result=FORMATS['read_reference'](agent,dict(url=url))
            self.assertIn('int InputGroups; int RecordOffsets;',result['dependencies'][0]['text'])
            self.assertEqual(opener.open.call_count,4)
            repeated=FORMATS['read_reference'](agent,dict(url=url))
            self.assertTrue(repeated['dependencies'][0]['already_supplied'])
            self.assertEqual(opener.open.call_count,4)

    def test_remote_plain_class_includes_helper_and_native_binding(self):
        from unittest.mock import Mock
        revision='b'*40
        url='https://raw.githubusercontent.com/example/reader/'+revision+'/Archive.cs'
        self.root.joinpath('work').mkdir()
        agent=types.SimpleNamespace(api=API,deadline=time.monotonic()+60,reference_revisions={('example','reader',revision)},
            host=types.SimpleNamespace(path=lambda root,path:self.root/root/path))
        tree=dict(tree=[dict(type='blob',path='Archive.cs'),dict(type='blob',path='Blowfish.cs')])
        opener=Mock()
        opener.open.side_effect=[io.BytesIO(b'using GameRes.Cryptography; class Archive { void Read() { new Blowfish(key).Decipher(data); } }'),
            io.BytesIO(json.dumps(tree).encode()),io.BytesIO(b'namespace GameRes.Cryptography { class Blowfish {\n'+b'// constants\n'*3000+b'public void Decipher(byte[] bytes) { } } }')]
        with patch.object(FORMATS['urllib'].request,'build_opener',return_value=opener):
            result=FORMATS['read_reference'](agent,dict(path=url))
        self.assertIn('GameRes.Cryptography.Blowfish.Decipher',result['native_bindings'])
        self.assertIn('void Decipher(byte[] bytes)',result['dependencies'][0]['text'])

    def test_remote_source_tree_recovers_missing_definition_and_filters_cached_paths(self):
        from unittest.mock import Mock
        import urllib.error
        skill,work=self.root/'skill',self.root/'work'
        skill.mkdir();work.mkdir()
        revision='a'*40
        url='https://github.com/example/reader/blob/'+revision+'/missing.cs'
        (skill/'SKILL.md').write_text('[Source]('+url+')',encoding='utf-8')
        roots=dict(skill=skill,work=work)
        agent=types.SimpleNamespace(api=API,deadline=time.monotonic()+60,
            host=types.SimpleNamespace(roots=roots,path=lambda root,path:roots[root]/path))
        tree=dict(tree=[dict(type='blob',path='Types/ActualHeader.cs'),dict(type='blob',path='reader.py'),
                        dict(type='blob',path='binary.exe'),dict(type='tree',path='Folder.cs')])
        opener=Mock()
        opener.open.side_effect=[urllib.error.HTTPError(url,404,'Not found',{},None),
                                 io.BytesIO(json.dumps(tree).encode())]
        with patch.object(FORMATS['urllib'].request,'build_opener',return_value=opener):
            first=FORMATS['read_reference'](agent,dict(url=url))
            self.assertEqual([entry['path'] for entry in first['members']],['Types/ActualHeader.cs','reader.py'])
            second=FORMATS['read_reference'](agent,dict(url=url,pattern='*header*'))
            self.assertEqual(second['total'],1)
            self.assertEqual(opener.open.call_count,2)
            self.assertTrue(second['members'][0]['url'].endswith('/'+revision+'/Types/ActualHeader.cs'))
            self.assertIsNone(opener.open.call_args.args[0].get_header('Authorization'))
        with self.assertRaisesRegex(ValueError,'not cited'):
            FORMATS['read_reference'](agent,dict(url=url.replace(revision,'b'*40),pattern='*'))

    def test_saved_program_can_be_repaired_without_rewriting_inputs(self):
        agent=types.SimpleNamespace(work=self.root,api=API,deadline=time.monotonic()+90,stamp=lambda path:None,
            host=types.SimpleNamespace(roots={'game':self.root,'work':self.root},path=lambda root,path:self.root/path,
                reader_command=[sys.executable,'-I','-S',str(BASE/'agent_host.py')]))
        inputs=[dict(id='archive',root='game',path='archive.bin')]
        first=FORMATS['run_program'](agent,dict(inputs=inputs,source=self.config['source']))
        second=FORMATS['run_program'](agent,dict(program=first['program'],
            edits=[dict(old='story/scene.txt',new='story/repaired.txt')]))
        self.assertEqual((self.root/second['output']/'story/repaired.txt').read_bytes(),b'fixture payload')
        self.assertTrue((self.root/first['output']/'story/scene.txt').is_file())
        self.assertEqual(second['inputs'],inputs)
        with self.assertRaisesRegex(ValueError,'exactly once'):
            FORMATS['run_program'](agent,dict(inputs=inputs,program=second['program'],edits=[dict(old='missing',new='x')]))


if __name__=='__main__':unittest.main()
