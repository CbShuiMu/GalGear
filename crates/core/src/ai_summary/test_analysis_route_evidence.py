"""路线显示文本来源、图标降级及 OCR 分词规范化的行为回归。"""
import json
from pathlib import Path
import runpy
import tempfile
from types import SimpleNamespace
import unittest

BASE=Path(__file__).parent
MODULE=runpy.run_path(str(BASE/'analysis_route_evidence.py'))
IMAGES=runpy.run_path(str(BASE/'analysis_images.py'))
READS=runpy.run_path(str(BASE/'analysis_reads.py'))
QUALITY=runpy.run_path(str(BASE/'analysis_text_quality.py'))
API=runpy.run_path(str(BASE/'agent_host.py'))


class RouteEvidenceTests(unittest.TestCase):
    def setUp(self):
        temp=tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        self.work=Path(temp.name).resolve()
        self.write('read.json',dict(value=dict(speaker='星川 咲',selects=[dict(text='一緒に行く',target='next')])) )
        self.write('index.json',dict(input_root='game',records=[dict(input='scene.bin',name='scene.bin',output=str(self.work/'read.json'))]))
        self.agent=SimpleNamespace(work=self.work,batches=['index.json'],image_receipts={'image.json'},
            host=SimpleNamespace(path=lambda root,path:self.work/path),api=API)
        self.graph=dict(characters=[dict(id='saki',name='星川 咲',source='scene.bin',pointer='/value/speaker')],
            nodes=[dict(label='{char:saki}との再会')],edges=[dict(kind='choice',label='一緒に行く',
                text_basis=dict(source='scene.bin',pointer='/value/selects/0/text',kind='script',binding='menu[0] -> next'))])

    def write(self,name,value): (self.work/name).write_text(json.dumps(value,ensure_ascii=False),'utf-8')

    def test_names_expand_from_source_and_choices_remain_verbatim(self):
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(self.graph['nodes'][0]['label'],'星川 咲との再会')
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(self.graph['edges'][0]['label'],'一緒に行く')

    def test_phonetic_name_guess_and_choice_paraphrase_rejected(self):
        self.graph['characters'][0]['name']='星川 早希'
        with self.assertRaisesRegex(ValueError,'verbatim'): MODULE['bind_graph'](self.agent,self.graph,[])
        self.graph['characters'][0]['name']='星川 咲'
        self.graph['edges'][0]['label']='跟她走'
        with self.assertRaisesRegex(ValueError,'exactly'): MODULE['bind_graph'](self.agent,self.graph,[])

    def test_large_script_field_accepts_exact_quote_but_not_rewritten_words(self):
        self.write('read.json',dict(value=dict(speaker='星川 咲',menu='button1="一緒に行く"\nbutton2="戻る"')))
        self.graph['edges'][0]['text_basis'].update(pointer='/value/menu')
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(self.graph['edges'][0]['text_basis']['quote'],'一緒に行く')
        self.graph['edges'][0]['text_basis']['quote']='一緒に帰る'
        with self.assertRaisesRegex(ValueError,'verbatim'): MODULE['bind_graph'](self.agent,self.graph,[])

    def test_query_json_is_not_original_text_evidence(self):
        self.write('query.json',dict(name='捏造'))
        self.graph['characters'][0].update(source='query.json',pointer='/name',name='捏造')
        with self.assertRaisesRegex(ValueError,'registered'): MODULE['bind_graph'](self.agent,self.graph,[])

    def test_short_character_placeholder_expands_without_retry(self):
        self.graph['nodes'][0]['label']='{saki} END'
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(self.graph['nodes'][0]['label'],'星川 咲 END')

    def test_comment_substring_cannot_masquerade_as_button_text(self):
        self.agent.observations={'O7':dict(tool='read_text',arguments=dict(root='game',path='flow.txt'),
            result=dict(text='// 一緒に行く is the internal flag\nif (flag) jump next;'),error=None)}
        self.graph['edges'][0]['text_basis']=dict(source='O7',pointer='/text',kind='script',binding='dispatch -> next')
        with self.assertRaisesRegex(ValueError,'only in source comments'):
            MODULE['bind_graph'](self.agent,self.graph,[])
        self.agent.observations['O7']['result']['text']='button("一緒に行く"); // comment'
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(MODULE['without_code_comments']('button("a // b"); // hidden'),'button("a // b"); ')

    def test_character_spacing_uses_actual_source_without_changing_kanji(self):
        self.graph['characters'][0]['name']='星川咲'
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(self.graph['characters'][0]['name'],'星川 咲')

    def test_character_reference_typo_is_repaired_from_original_not_query(self):
        self.graph['characters'][0].update(source='query-names.json',pointer='speaker table',name='星川咲')
        MODULE['bind_graph'](self.agent,self.graph,[])
        item=self.graph['characters'][0]
        self.assertEqual(item['name'],'星川 咲')
        self.assertEqual(item['pointer'],'/value/speaker')
        self.assertEqual(item['source_binding'],'located_in_original_read')
        self.assertNotEqual(item['source'],'query-names.json')

    def test_referenced_extracted_image_is_read_once_and_unrelated_art_is_ignored(self):
        from unittest.mock import Mock,patch
        import hashlib
        self.agent.image_receipts=set()
        (self.work/'out').mkdir()
        members=[]
        for name in ('story_buttons.hg3','background.hg3'):
            data=name.encode();(self.work/'out'/name).write_bytes(data)
            members.append(dict(path=name,sha256=hashlib.sha256(data).hexdigest()))
        self.write('manifest.json',dict(members=members))
        self.agent.observations={'O1':dict(tool='run_format_program',error=None,
            result=dict(manifest='manifest.json',output='out'))}
        read=Mock(return_value=dict(output='ui-text.json',frames=[dict(native_frame_id=7,ordinal=0,text='進む')]))
        with patch.dict(IMAGES['prepare_route_images'].__globals__,read_images=read):
            result=IMAGES['prepare_route_images'](self.agent,dict(evidence='story_buttons frame 7'))
            self.assertEqual(result[0]['frames'][0]['native_frame_id'],7)
            self.assertEqual(IMAGES['prepare_route_images'](self.agent,dict(evidence='story_buttons frame 7')),[])
        self.assertEqual(read.call_count,1)
        self.assertEqual(Path(read.call_args.args[1]['path']).name,'story_buttons.hg3')

    def test_ambiguous_image_versions_are_not_automatically_chosen(self):
        from unittest.mock import Mock,patch
        self.agent.image_receipts=set()
        self.write('manifest.json',dict(members=[dict(path='buttons.hg3',sha256='digest')]))
        self.agent.observations={str(i):dict(tool='run_format_program',error=None,
            result=dict(manifest='manifest.json',output=folder)) for i,folder in enumerate(('base','patch'))}
        read=Mock()
        with patch.dict(IMAGES['prepare_route_images'].__globals__,read_images=read):
            self.assertEqual(IMAGES['prepare_route_images'](self.agent,dict(evidence='buttons.hg3')),[])
        read.assert_not_called()

    def test_image_receipts_do_not_collide_and_later_ocr_keeps_earlier_frames(self):
        from unittest.mock import Mock,patch
        import hashlib,time
        self.agent.observations={};self.agent.image_receipts=set();self.agent.image_cache={}
        self.agent.stamp=lambda p:None;self.agent.check_inputs=lambda:None
        self.agent.emit=self.write;self.agent.deadline=time.monotonic()+60
        for name in ('a.png','b.png'):
            data=name.encode();(self.work/name).write_bytes(data)
            self.agent.image_cache[hashlib.sha256(data).hexdigest()]=dict(source=str(self.work/name),
                source_sha256=hashlib.sha256(data).hexdigest(),errors=[],frames=[dict(ordinal=i,path=name,
                sha256='x',width=10,height=10,origin={}) for i in range(2)])
        def ocr(*args,**kwargs):
            request=json.loads(kwargs['input'])
            return SimpleNamespace(returncode=0,stdout=json.dumps(dict(images=[dict(id=i['id'],text='text'+str(i['id'])) for i in request['images']])).encode())
        with patch.object(IMAGES['subprocess'],'run',side_effect=ocr):
            first=IMAGES['read_images'](self.agent,dict(root='work',path='a.png',frames=[0]))
            other=IMAGES['read_images'](self.agent,dict(root='work',path='b.png',frames=[0]))
            final=IMAGES['read_images'](self.agent,dict(root='work',path='a.png',frames=[1]))
        self.assertNotEqual(first['output'],other['output'])
        self.assertEqual([f['text'] for f in final['frames']],['text0','text1'])

    def test_walkthrough_navigation_cannot_be_declared_original_button_text(self):
        self.agent.observations={'O7':dict(tool='read_text',arguments=dict(root='game',path='guide.txt'),
            result=dict(text='星川 咲\n一緒に行く'),error=None)}
        self.graph['edges'][0]['text_basis']=dict(source='game/guide.txt',pointer='/text',kind='script',
            quote='一緒に行く',binding='guide route line')
        with self.assertRaisesRegex(ValueError,'walkthrough'):
            MODULE['bind_graph'](self.agent,self.graph,[],['O7'])
        MODULE['bind_graph'](self.agent,self.graph,[('scene.bin',dict(variants=['一緒に行く']))],['O7'])

    def test_source_aliases_accept_windows_paths_and_direct_text_observations(self):
        for source in ('game/scene.bin',str(self.work/'read.json').replace('\\','/'),'read.json'):
            self.graph['characters'][0]['source']=source
            MODULE['bind_graph'](self.agent,self.graph,[])
        self.agent.observations={'O7':dict(tool='read_text',arguments=dict(root='game',path='guide.txt'),
            result=dict(text='星川 咲\n一緒に行く'),error=None)}
        self.graph['characters'][0].update(source='O7',pointer='人物介绍段')
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(self.graph['characters'][0]['pointer'],'/text')
        self.graph['characters'][0].update(source='game/guide.txt',pointer='/text')
        MODULE['bind_graph'](self.agent,self.graph,[])

    def test_same_basename_in_distinct_archives_stays_ambiguous(self):
        self.write('index.json',dict(input_root='work',records=[
            dict(input='base/scene.bin',name='base/scene.bin',output='read.json'),
            dict(input='patch/scene.bin',name='patch/scene.bin',output='other.json')]))
        self.write('other.json',dict(value=dict(speaker='別の名前')))
        with self.assertRaisesRegex(ValueError,'unique original'):
            MODULE['source_value'](self.agent,dict(source='scene.bin',pointer='/value/speaker'))

    def test_ocr_correction_requires_corroborating_original_text_and_keeps_raw(self):
        self.write('image.json',dict(frames=[dict(text='一緒に衍く')]))
        basis=dict(source='image.json',pointer='/frames/0/text',kind='ocr',binding='button -> next')
        self.graph['edges'][0]['text_basis']=basis
        with self.assertRaisesRegex(ValueError,'exactly'):MODULE['bind_graph'](self.agent,self.graph,[])
        basis['correction_basis']=dict(source='scene.bin',pointer='/value/selects/0/text')
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(basis['raw_ocr'],'一緒に衍く')
        self.assertEqual(self.graph['edges'][0]['text_status'],'ocr_corrected_from_source')

    def test_hg3_native_ids_use_headers_not_converter_ordinals(self):
        import struct
        data=b'HG-3'+struct.pack('<II',12,0x300)+struct.pack('<II',16,201)+bytes(8)+struct.pack('<II',0,100)
        self.assertEqual(IMAGES['native_frame_ids'](data),{20:201,36:100})
        self.assertEqual(IMAGES['native_frame_ids'](data[:27]),{})
        self.assertEqual(IMAGES['native_frame_ids'](b'not this format'),{})

    def test_icon_keeps_canonical_name_without_inventing_wording(self):
        self.write('image.json',dict(frames=[dict(sha256='a'*64,ocr=dict(text=''),text='')]))
        self.graph['edges'][0]['text_basis']=dict(source='image.json',pointer='/frames/0',kind='icon',character_id='saki',binding='button id -> script target')
        MODULE['bind_graph'](self.agent,self.graph,[])
        self.assertEqual(self.graph['edges'][0]['label'],'星川 咲（人物图标）')
        self.write('image.json',dict(frames=[dict(sha256='a'*64,ocr=dict(text='行く'),text='行く')]))
        with self.assertRaisesRegex(ValueError,'no detected text'): MODULE['bind_graph'](self.agent,self.graph,[])

    def test_cjk_ocr_spacing_preserves_latin_word_boundaries(self):
        self.assertEqual(IMAGES['normalized_text']('今 は や め て お く'),'今はやめておく')
        self.assertEqual(IMAGES['normalized_text']('New Game'),'New Game')

    def test_custom_reader_uses_existing_choice_projection(self):
        path=self.work/'script.bin'; path.write_bytes(b'custom fixture')
        api=dict(parser_worker=lambda *a,**k:dict(selects=[dict(text='戻る',target='back')]))
        value=READS['read_one'](self.work,path,'custom',dict(source='def extract(data, metadata): return {}'),api)
        self.assertEqual(READS['known_options'](value['choice_texts']),[dict(pointer='/value/selects/0',variants=['戻る'])])

    def test_text_diagnostics_return_samples_once_without_model_request(self):
        self.write('decoded.json',dict(value=['一緒に駅まで帰りましょう。']))
        from unittest.mock import Mock
        self.agent.complete=Mock(side_effect=AssertionError('No review request'))
        self.agent.emit=self.write
        records=[dict(output=str(self.work/'decoded.json'))]
        result=QUALITY['review_reader'](self.agent,'decoder',records)
        self.assertEqual(result['samples'],['一緒に駅まで帰りましょう。'])
        self.assertIsNone(QUALITY['review_reader'](self.agent,'decoder',records))
        self.agent.complete.assert_not_called()


if __name__=='__main__': unittest.main()
