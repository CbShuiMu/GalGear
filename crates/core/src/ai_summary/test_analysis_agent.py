"""静态交付契约、批量查询及冷启动隔离的行为回归。"""
import json
from pathlib import Path
import runpy
import struct
import tempfile
import unittest
from unittest.mock import patch

BASE = Path(__file__).parent
API = runpy.run_path(str(BASE / 'agent_host.py'))
ANALYSIS = runpy.run_path(str(BASE / 'analysis_agent.py'))
READS = runpy.run_path(str(BASE / 'analysis_reads.py'))


class AnalysisTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.game, self.skill, self.save = [self.root / p for p in ('game','skill','save')]
        for path in (self.game,self.skill,self.save): path.mkdir()
        (self.game/'engine.exe').write_bytes(b'MZ')
        (self.skill/'SKILL.md').write_text('Fixture instructions',encoding='utf-8')
        (self.save/'slot.dat').write_bytes(b'fixture save')
        self.config=dict(game=str(self.game),skill=str(self.skill),save=str(self.save),
            work=str(self.game/'.galgear-ai-summary'/'new'), provider='deepseek',model='test',token='test',
            title='fixture',language='zh-CN',cold_start=True,save_candidates=[])
        self.agent=ANALYSIS['AnalysisAgent'](self.config,API)
        self.recap_requests=[]
        def summary(messages,available,**kwargs):
            self.recap_requests.append(messages)
            return dict(message=dict(tool_calls=[dict(function=dict(arguments=json.dumps(dict(
                recap='根据存档之前的脚本片段，伙伴们在港口商量行程，并确认出发前的准备。当前重建只依据这一小段剧情。',
                current_progress='目前仍处于港口交谈阶段，存档文本与最后一条消息相符；这是静态候选，未验证实际加载。'),ensure_ascii=False)))]))
        self.agent.complete=summary

    def result(self):
        self.agent.stamp(self.save/'slot.dat')
        self.agent.batches=['fixture-index.json']
        self.agent.observations={'O1':dict(tool='query_data',error=None)}
        self.agent.recap_context=dict(save_input=str(self.save/'slot.dat'),records=['港口准备'],anchor='港口准备')
        return dict(recap='伙伴们在港口商量出发前的准备，交流了目的地与各自的顾虑。'*4,
            current_progress='当前场景仍在港口。根据存档文本推测，他们正在确认旅程安排，还没有开始下一阶段。',
            script_language='zh-CN',graph=dict(nodes=[dict(id='s',label='开始',kind='start',evidence='O1')],edges=[],uncertainties=['候选位置']),
            evidence_refs=['O1'],save_position=dict(root='save',path='slot.dat',level='candidate',locator='caption',explanation='候选'),
            scope=dict(dialogue_count=5,flow_basis=dict(kind='script_control_flow',ref='O1',locator='fixture calls')),limitations=['未加载游戏'])

    def test_static_result_retains_level_and_rejects_runtime_claim(self):
        value=self.result()
        value['save_position']['level']='runtime_verified'
        with self.assertRaisesRegex(ValueError,'precise locator'): self.agent.finish(value)
        value['save_position']['level']='candidate'
        self.agent.finish(value)
        self.assertFalse(self.agent.done['save_position']['runtime_verified'])
        self.assertEqual(self.agent.done['save_position']['level'],'candidate')

    def test_changed_save_rejects_publication(self):
        value=self.result()
        (self.save/'slot.dat').write_bytes(b'changed bytes')
        with self.assertRaisesRegex(ValueError,'Input changed'): self.agent.finish(value)
        self.assertFalse((self.agent.work/'analysis-result.json').exists())

    def test_last_filename_cannot_be_submitted_as_an_ending(self):
        value=self.result()
        value['graph']['nodes'][0]['kind']='ending'
        with self.assertRaisesRegex(ValueError,'terminal_basis'): self.agent.finish(value)
        value['graph']['nodes'][0]['terminal_basis']=dict(kind='local_guide',ref='O1',locator='guide section 4')
        self.agent.finish(value)

    def test_model_cannot_promote_its_own_static_coordinate_claim(self):
        value=self.result()
        value['save_position']['level']='static_consistent'
        with self.assertRaisesRegex(ValueError,'precise locator'): self.agent.finish(value)

    def test_long_linear_catalog_requires_grouping_but_shared_options_survive(self):
        value=self.result()
        value['graph']['nodes'] += [dict(id='n'+str(i),label='场景',kind='scene',evidence='O1') for i in range(10)]
        value['graph']['edges']=[dict(**{'from':a,'to':b},label='',inferred=True,evidence='O1')
            for a,b in zip(['s']+['n'+str(i) for i in range(9)],['n'+str(i) for i in range(10)])]
        with self.assertRaisesRegex(ValueError,'consecutive linear'):self.agent.finish(value)
        value['graph']['nodes']=value['graph']['nodes'][:2]
        value['graph']['edges']=[dict(**{'from':'s','to':'n0'},kind='choice',label=label,inferred=False,evidence='O1') for label in ['答应','拒绝']]
        self.agent.finish(value)
        self.assertEqual(len(self.agent.done['graph']['edges']),2)

    def test_unlock_requires_a_readable_prerequisite(self):
        value=self.result()
        value['graph']['nodes'].append(dict(id='extra',label='后日谈',kind='extra',evidence='O1'))
        value['graph']['edges']=[dict(**{'from':'s','to':'extra'},kind='unlock',label='进入',inferred=True,evidence='O1')]
        with self.assertRaisesRegex(ValueError,'readable prerequisite'):self.agent.finish(value)
        value['graph']['edges'][0]['condition']='通关后'
        with self.assertRaisesRegex(ValueError,'Unlock must cite'):self.agent.finish(value)

    def test_repeated_menu_text_at_different_locations_cannot_be_merged(self):
        value=self.result()
        self.agent.emit('fixture-index.json',dict(records=[dict(input='scene.ast',menu_locations=['/block1','/block2'])]))
        value['graph']['nodes'].append(dict(id='c',label='选择',kind='choice',evidence='O1'))
        with self.assertRaisesRegex(ValueError,'2 distinct menu locations'):self.agent.finish(value)

    def test_finish_tool_can_express_supported_grand_unlock_contract(self):
        schema=next(t['function']['parameters'] for t in ANALYSIS['tools'](API) if t['function']['name']=='finish_analysis')
        graph=schema['properties']['graph']['properties']
        for kind in ('extra','grand'):
            self.assertIn(kind,graph['nodes']['items']['properties']['kind']['enum'])
        edge=graph['edges']['items']['properties']
        self.assertIn('unlock',edge['kind']['enum'])
        self.assertEqual(set(edge['unlock_basis']['required']),{'kind','ref','locator'})
        self.assertIn('condition',edge)
        self.assertIn('excluded_menus',schema['properties']['scope']['properties'])
        value=self.result()
        value['graph']['nodes'].append(dict(id='grand',label='真结局',kind='grand',evidence='O1',
            terminal_basis=dict(kind='ending_flag',ref='O1',locator='clear flag')))
        value['graph']['edges'].append(dict(**{'from':'s','to':'grand'},kind='unlock',label='进入真结局',condition='全路线通关',
            inferred=False,evidence='O1',unlock_basis=dict(kind='script_condition',ref='O1',locator='persistent flags')))
        self.agent.finish(value)
        self.assertEqual(self.agent.done['graph']['nodes'][-1]['kind'],'grand')

    def test_scene_title_must_not_be_a_script_catalog(self):
        value=self.result()
        value['graph']['nodes'].append(dict(id='road',label='scene_04_1–05_4',kind='scene',evidence='O1'))
        with self.assertRaisesRegex(ValueError,'player-facing story title'):self.agent.finish(value)
        value['graph']['nodes'][-1]['label']='The Long Road'
        value['graph']['nodes'][-1]['pc']='scene_04_1.dat–scene_05_4.dat'
        self.agent.finish(value)
        self.assertEqual(self.agent.done['graph']['nodes'][-1]['label'],'The Long Road')

    def test_grouped_narration_cannot_create_a_fake_self_loop(self):
        value=self.result()
        value['graph']['edges']=[dict(**{'from':'s','to':'s'},kind='flow',label='叙事继续',inferred=True,evidence='O1')]
        with self.assertRaisesRegex(ValueError,'remove internal edges'):self.agent.finish(value)
        value['graph']['edges'][0].update(kind='reference',inferred=False,evidence='O1 explicit jump to same label')
        self.agent.finish(value)
        self.assertEqual(len(self.agent.done['graph']['edges']),1)

    def test_conditional_fork_cannot_be_hidden_in_a_story_segment(self):
        value=self.result()
        value['graph']['nodes'] += [dict(id=i,kind='scene',label=label,evidence='O1') for i,label in [('gate','后续剧情和结局'),('a','继续旅程'),('b','告别')]]
        value['graph']['edges']=[dict(**{'from':'gate','to':target},kind='flow',label=label,condition=label,inferred=False,evidence='O1') for target,label in [('a','条件满足'),('b','条件未满足')]]
        with self.assertRaisesRegex(ValueError,'separate decision'):self.agent.finish(value)
        value['graph']['nodes'][1].update(kind='choice',label='是否满足旅程继续的条件')
        self.agent.finish(value)
        self.assertEqual(self.agent.done['graph']['nodes'][1]['kind'],'choice')

    def test_save_marker_cannot_point_back_from_a_disconnected_root(self):
        value=self.result()
        value['graph']['nodes'].append(dict(id='sv',kind='save',label='当前存档',evidence='O1'))
        value['graph']['edges']=[dict(**{'from':'sv','to':'s'},label='字幕位置',inferred=True,evidence='O1')]
        with self.assertRaisesRegex(ValueError,'must be reachable'):self.agent.finish(value)
        value['graph']['edges'][0].update({'from':'s','to':'sv'})
        self.agent.finish(value)

    def test_unresolved_file_order_cannot_be_declared_known_execution(self):
        value=self.result()
        value['scope']['flow_basis']['kind']='unresolved'
        value['graph']['nodes'].append(dict(id='b',kind='scene',label='后续片段',evidence='O1'))
        value['graph']['edges']=[dict(**{'from':'s','to':'b'},label='列表相邻',inferred=False,evidence='O1')]
        with self.assertRaisesRegex(ValueError,'Unresolved execution order'):self.agent.finish(value)
        value['graph']['edges'][0]['inferred']=True
        self.agent.finish(value)

    def test_structured_choice_projection_retains_languages_and_disabled_state(self):
        value={'story/part':{'selects':[
            dict(text='Go',language=[None,dict(text='出发')],target='next',exp='flag=1'),
            dict(text='Go',language=[dict(text='出发')],disabled='true')]}}
        before=json.dumps(value)
        choices=READS['structured_choices'](value)
        self.assertEqual(choices[0]['option_pointer'],'/value/story~1part/selects/0')
        self.assertEqual(choices[0]['exp'],'flag=1')
        self.assertEqual(choices[1]['disabled'],'true')
        self.assertEqual(READS['known_options'](choices),[dict(pointer='/value/story~1part/selects/0',variants=['Go','出发'])])
        self.assertEqual(json.dumps(value),before)

    def test_readable_option_cannot_be_replaced_by_unknown_placeholder(self):
        value=self.result()
        self.agent.emit('fixture-index.json',dict(records=[dict(name='scene.scn',known_options=[dict(pointer='/value/scenes/1/selects/0',variants=['Go','出发'])])]))
        value['graph']['nodes'].append(dict(id='c',kind='scene',label='前往车站',evidence='O1'))
        value['graph']['edges']=[dict(**{'from':'s','to':'c'},label='选项一（未解码）',evidence='O1',inferred=False,kind='choice')]
        with self.assertRaisesRegex(ValueError,'already decoded'):self.agent.finish(value)
        value['graph']['edges'][0]['label']='出发'
        self.agent.finish(value)
        self.assertEqual(self.agent.done['graph']['edges'][0]['label'],'出发')

    def test_choice_after_first_six_files_is_still_in_batch_response(self):
        from types import SimpleNamespace
        names=[str(i)+'.txt' for i in range(7)]
        for name in names:(self.game/name).write_text('fixture',encoding='utf-8')
        (self.agent.work/'reader-results'/'1').mkdir(parents=True)
        records=[dict(name=name,output=str(self.agent.work/(name+'.json')),read_key=name,choice_count=0) for name in names]
        records[-1].update(choice_count=1,choice_preview=[dict(text='出发',target='next')],menu_locations=['/value/selects'])
        response=SimpleNamespace(returncode=0,stdout=json.dumps(dict(records=records,succeeded=7,failed=0)).encode(),stderr=b'')
        with patch.object(ANALYSIS['subprocess'],'run',return_value=response):
            result=self.agent.batch(dict(root='game',paths=names,reader='text'))
        self.assertEqual(len(result['records']),6)
        self.assertEqual(result['choice_sources'][0]['name'],'6.txt')
        self.assertEqual(result['choice_sources'][0]['choice_preview'][0]['text'],'出发')

    def test_failed_reference_and_unknown_endpoint_rejected(self):
        value=self.result()
        value['evidence_refs']=['O9']
        with self.assertRaisesRegex(ValueError,'Invalid refs'): self.agent.finish(value)
        value['evidence_refs']=['O1']
        value['graph']['edges']=[dict(**{'from':'s','to':'absent'},inferred=True,evidence='O1')]
        with self.assertRaisesRegex(ValueError,'endpoints'): self.agent.finish(value)

    def test_json_query_reads_all_index_records_in_one_call(self):
        self.agent.emit('one.json',dict(value='甲'))
        self.agent.emit('two.json',dict(value='乙'))
        self.agent.emit('index.json',dict(records=[dict(name='a/x',output='one.json'),dict(name='b/x',output='two.json')]))
        self.agent.batches=['index.json']
        value=self.agent.query(dict(indexes=['index.json'],source='def extract(data, metadata):\n return {k:v["value"] for k,v in metadata["documents"].items()}'))
        self.assertEqual(json.loads((self.agent.work/value['output']).read_text('utf-8')),{'a/x':'甲','b/x':'乙'})

    def test_batch_retains_partial_failures_and_native_input_hashes(self):
        (self.game/'good.txt').write_text('消息',encoding='utf-8')
        (self.game/'bad.txt').write_bytes(b'\xff\x01')
        result=READS['worker'](dict(skill=str(self.skill),output=str(self.agent.work/'batch'),reader='text',
            items=[dict(input=str(self.game/p),name=p) for p in ('good.txt','bad.txt')]))
        self.assertEqual((result['succeeded'],result['failed']),(1,1))
        self.assertEqual(len(result['records'][0]['sha256']),64)

    def test_query_projection_skips_heavy_unneeded_native_tables(self):
        self.agent.emit('data.json',dict(dialogue=[dict(text='one')],instructions=['unused']*10000))
        value=self.agent.query(dict(files=['data.json'],fields=['dialogue'],source='def extract(data, metadata):\n return metadata["documents"]'))
        documents=json.loads((self.agent.work/value['output']).read_text('utf-8'))
        self.assertEqual(documents,{'data.json':{'dialogue':[{'text':'one'}]}})

    def test_single_argument_query_contract(self):
        self.agent.emit('data.json',dict(value=42))
        value=self.agent.query(dict(files=['data.json'],source='def analyze(documents):\n return documents["data.json"]["value"]'))
        self.assertEqual(json.loads((self.agent.work/value['output']).read_text('utf-8')),42)

    def test_recap_request_cannot_see_route_exploration(self):
        value=self.result()
        self.agent.messages=[dict(role='user',content='FUTURE ENDING SPOILER')]
        self.agent.finish(value)
        self.assertNotIn('FUTURE ENDING SPOILER',json.dumps(self.recap_requests))
        self.assertIn('港口准备',json.dumps(self.recap_requests,ensure_ascii=False))

    def test_caption_context_stops_inside_record_before_future_text(self):
        self.agent.emit('save.json',dict(caption='当前句子'))
        self.agent.emit('script.json',dict(rows=[dict(text='先前句子'),dict(parts=['当前句子','FUTURE IN SAME BLOCK']),dict(text='FUTURE NEXT BLOCK')]))
        for number,root,file in [(1,'save','save.json'),(2,'work','script.json')]:
            index=str(number)+'.json'
            self.agent.emit(index,dict(input_root=root,records=[dict(output=file,input=str(self.save/'slot.dat') if root=='save' else str(self.game/'scene'),sha256='fixture')]))
            self.agent.batches.append(index)
        result=self.agent.dispatch('select_recap_context',dict(save_file='save.json',save_pointer='/caption',script_file='script.json',script_pointer='/rows',explanation='caption candidate'))
        text=(self.agent.work/result['context']).read_text('utf-8')
        self.assertNotIn('FUTURE',text)
        self.assertIn('先前句子',text)

    def test_coordinate_window_never_includes_later_rows(self):
        self.agent.emit('save.json',dict(line=10))
        self.agent.emit('script.json',dict(rows=[dict(source_line=9,text='before'),dict(source_line=10,text='at'),dict(source_line=11,text='future')]))
        self.agent.emit('s.json',dict(input_root='save',records=[dict(output='save.json',input=str(self.save/'slot.dat'),sha256='save')]))
        self.agent.emit('t.json',dict(input_root='work',records=[dict(output='script.json',input='scene',sha256='script')]))
        self.agent.batches=['s.json','t.json']
        self.agent.dispatch('select_recap_context',dict(save_file='save.json',save_pointer='/line',script_file='script.json',script_pointer='/rows',coordinate_field='source_line',explanation='source line'))
        self.assertEqual([r['value']['text'] for r in self.agent.recap_context['records']],['before','at'])

    def test_small_query_result_preserves_all_entries(self):
        self.agent.emit('many.json',dict(values=list(range(40))))
        result=self.agent.query(dict(files=['many.json'],source='def analyze(documents): return documents["many.json"]'))
        self.assertEqual(result['result']['values'],list(range(40)))

    def test_batch_reuses_only_same_input_and_canonical_reader(self):
        path=self.game/'sample.ksd'
        path.write_bytes(bytes.fromhex('fffe')+'(const) %["caption" => "当前句子"]'.encode('utf-16-le'))
        first=self.agent.batch(dict(root='game',paths=['sample.ksd']))
        second=self.agent.batch(dict(root='game',paths=['sample.ksd'],reader='structured'))
        self.assertEqual(first['reused'],0)
        self.assertEqual(second['reused'],1)
        self.assertEqual(first['records'][0]['output'],second['records'][0]['output'])
        path.write_bytes(bytes.fromhex('fffe')+'(const) %["caption" => "后来的句子"]'.encode('utf-16-le'))
        third=self.agent.batch(dict(root='game',paths=['sample.ksd']))
        self.assertEqual(third['reused'],0)
        self.assertNotEqual(first['records'][0]['sha256'],third['records'][0]['sha256'])

    def test_historical_bookmark_cannot_replace_decoded_latest_slot(self):
        self.agent.emit('save.json',dict(caption='当前句子'))
        self.agent.emit('script.json',dict(rows=['当前句子']))
        self.agent.emit('s.json',dict(input_root='save',records=[dict(output='save.json',input=str(self.save/'old.dat'),sha256='old')]))
        self.agent.emit('t.json',dict(input_root='work',records=[dict(output='script.json',input='scene',sha256='script')]))
        self.agent.batches=['s.json','t.json']
        self.agent.preferred_save=str(self.save/'slot.dat')
        with self.assertRaisesRegex(ValueError,'latest progress save'):
            self.agent.dispatch('select_recap_context',dict(save_file='save.json',save_pointer='/caption',script_file='script.json',script_pointer='/rows',explanation='old bookmark'))

    def test_recap_source_names_resolve_without_opaque_json_paths(self):
        self.agent.emit('save.json',dict(caption='当前句子'))
        self.agent.emit('script.json',dict(rows=['先前消息','当前句子','未来消息']))
        self.agent.emit('s.json',dict(input_root='save',records=[dict(output='save.json',input=str(self.save/'slot.dat'),name='slot.dat',sha256='save')]))
        self.agent.emit('t.json',dict(input_root='work',records=[dict(output='script.json',input=str(self.game/'scene.ast'),name='archive/scene.ast',sha256='script')]))
        self.agent.batches=['s.json','t.json']
        self.agent.preferred_save=str(self.save/'slot.dat')
        self.agent.dispatch('select_recap_context',dict(save_pointer='/caption',script_file='scene.ast',script_pointer='/rows',explanation='caption'))
        self.assertEqual([r['value'] for r in self.agent.recap_context['records']],['先前消息','当前句子'])
        self.agent.emit('other.json',dict(rows=['当前句子']))
        self.agent.emit('other-index.json',dict(input_root='work',records=[dict(output='other.json',input=str(self.game/'other'/'scene.ast'),name='other/scene.ast',sha256='other')]))
        self.agent.batches.append('other-index.json')
        with self.assertRaisesRegex(ValueError,'unique original source'):
            self.agent.dispatch('select_recap_context',dict(save_pointer='/caption',script_file='scene.ast',script_pointer='/rows',explanation='ambiguous'))

    def test_unknown_native_slot_precedes_newer_unknown_config(self):
        (self.save/'preferences.sav').write_bytes(b'configuration')
        (self.save/'native.sav').write_bytes(struct.pack('<10i',1,0,2025,6,2,1,12,30,0,0))
        self.agent.save_candidates=[dict(root='save',path='preferences.sav',kind='unknown',modified=20),
                                    dict(root='save',path='native.sav',kind='unknown',modified=10)]
        self.assertEqual(self.agent.initial_candidates()[0]['path'],'native.sav')
        self.assertEqual(self.agent.initial_candidates()[0]['native_reader'],'siglus_save')

    def test_transient_completion_retries_once_and_records_failed_time(self):
        transport=self.agent.api['TRANSPORT']
        error=transport['CompletionError']('timeout',True,elapsed_ms=120000)
        from unittest.mock import Mock
        call=Mock(side_effect=[error,dict(message=dict(content='ok'),elapsed_ms=20)])
        with patch.dict(transport,complete=call):
            result=ANALYSIS['AnalysisAgent'].complete(self.agent,[],[])
        self.assertEqual(result['message']['content'],'ok')
        self.assertEqual(call.call_count,2)
        logged=[json.loads(line) for line in (self.agent.work/'requests.jsonl').read_text().splitlines()]
        self.assertEqual(logged[0]['elapsed_ms'],120000)
        self.assertEqual(self.agent.requests,2)

    def test_payment_error_is_not_retried(self):
        transport=self.agent.api['TRANSPORT']
        from unittest.mock import Mock
        call=Mock(side_effect=transport['CompletionError']('http',False,http_status=402))
        with patch.dict(transport,complete=call), self.assertRaises(transport['CompletionError']):
            ANALYSIS['AnalysisAgent'].complete(self.agent,[],[])
        self.assertEqual(call.call_count,1)

    def test_cold_tool_cannot_read_previous_work_or_escape_root(self):
        with self.assertRaisesRegex(ValueError,'another preparation cache'):
            self.agent.host.path('game','.galgear-ai-summary/old/result.json')
        with self.assertRaisesRegex(ValueError,'outside allowed root'):
            self.agent.query(dict(files=['../secret.json'],source='def extract(data, metadata): return {}'))

    def test_custom_readers_bind_caption_to_native_ordinal_without_future_text(self):
        (self.save/'slot.dat').write_bytes('当前句子'.encode('utf-16-le'))
        (self.game/'script.bin').write_bytes('先前消息\n当前句子\n未来消息'.encode('utf-16-le'))
        source='def extract(data, metadata):\n return {"rows": [{"id": i, "text": text} for i, text in enumerate(data.decode("utf-16-le").split("\\n"))]}'
        saved=self.agent.batch(dict(root='save',paths=['slot.dat'],reader='custom',options=dict(source=source)))
        script=self.agent.batch(dict(root='game',paths=['script.bin'],reader='custom',options=dict(source=source)))
        self.assertEqual((saved['succeeded'],script['succeeded']),(1,1))
        self.agent.dispatch('select_recap_context',dict(save_file='slot.dat',save_pointer='/value/rows/0/text',
            script_file='script.bin',script_pointer='/value/rows',explanation='Exact decoded caption candidate'))
        context=self.agent.recap_context
        self.assertTrue(context['custom_decoder'])
        self.assertEqual([r['value']['id'] for r in context['records']],[0,1])
        self.assertNotIn('未来消息',json.dumps(context,ensure_ascii=False))
        self.assertEqual(context['save_sha256'],__import__('hashlib').sha256((self.save/'slot.dat').read_bytes()).hexdigest())
        repeated=self.agent.batch(dict(root='game',paths=['script.bin'],reader='custom',options=dict(source=source)))
        self.assertEqual(repeated['reused'],1)

    def test_custom_reader_can_reuse_inspection_but_not_different_input(self):
        args=dict(root='save',path='slot.dat',source='def extract(data, metadata): return {"caption":data.decode("ascii")}')
        self.agent.observations['O1']=dict(tool='inspect_data',arguments=args,error=None)
        result=self.agent.batch(dict(root='save',paths=['slot.dat'],reader='custom',options=dict(inspection_ref='O1')))
        self.assertEqual(result['succeeded'],1)
        (self.save/'other.dat').write_bytes(b'other')
        with self.assertRaisesRegex(ValueError,'exactly this input'):
            self.agent.batch(dict(root='save',paths=['other.dat'],reader='custom',options=dict(inspection_ref='O1')))
        with self.assertRaisesRegex(ValueError,'Unavailable name'):
            self.agent.batch(dict(root='save',paths=['slot.dat'],reader='custom',options=dict(source='def extract(data, metadata): return open("secret")')))

    def test_custom_context_cannot_be_promoted_to_active_engine_state(self):
        value=self.result()
        self.agent.recap_context['custom_decoder']=True
        value['save_position']['level']='decoded_active'
        with self.assertRaisesRegex(ValueError,'only support candidate'):self.agent.finish(value)
        value['save_position']['level']='candidate'
        self.agent.finish(value)

    def test_route_checkpoint_survives_missing_recap_and_can_finish_without_resending_graph(self):
        value=self.result()
        context=self.agent.recap_context
        self.agent.recap_context=None
        with self.assertRaisesRegex(ValueError,'Route saved'):self.agent.finish(value)
        draft=json.loads((self.agent.work/'route-draft.json').read_text('utf-8'))
        self.assertEqual(draft['status'],'partial')
        self.assertFalse((self.agent.work/'analysis-result.json').exists())
        self.agent.recap_context=context
        self.agent.finish(dict(save_position=value['save_position']))
        self.assertEqual(self.agent.done['graph'],value['graph'])

    def test_no_tool_replies_stop_after_one_recovery_and_preserve_conversation(self):
        from unittest.mock import Mock
        self.agent.complete=Mock(return_value=dict(message=dict(role='assistant',content='Cannot bind the save caption.')))
        with self.assertRaisesRegex(RuntimeError,'no progress'):self.agent.run()
        self.assertEqual(self.agent.complete.call_count,2)
        self.assertIn('Cannot bind the save caption',(self.agent.work/'conversation.json').read_text('utf-8'))
        self.assertEqual(json.loads((self.agent.work/'session.json').read_text())['status'],'failed')

    def test_identical_failed_calls_stop_instead_of_consuming_entire_budget(self):
        from unittest.mock import Mock
        message=dict(role='assistant',tool_calls=[dict(id='call',type='function',function=dict(name='read_text',
            arguments=json.dumps(dict(root='game',path='missing.txt'))))])
        self.agent.complete=Mock(return_value=dict(message=message))
        with self.assertRaisesRegex(RuntimeError,'no progress'):self.agent.run()
        self.assertEqual(self.agent.complete.call_count,3)

    def test_resource_path_accepts_file_and_unambiguous_directory_relative_names(self):
        (self.game/'scripts').mkdir()
        (self.game/'scripts'/'scene.txt').write_text('scene',encoding='utf-8')
        direct=self.agent.batch(dict(root='game',path='scripts/scene.txt'))
        listed=self.agent.batch(dict(root='game',path='scripts',paths=['scene.txt']))
        self.assertEqual((direct['succeeded'],listed['succeeded'],listed['reused']),(1,1,1))
        (self.game/'scene.txt').write_text('other scene',encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'Ambiguous'):
            self.agent.batch(dict(root='game',path='scripts',paths=['scene.txt']))
        with self.assertRaisesRegex(ValueError,'outside allowed root'):
            self.agent.batch(dict(root='game',path='scripts',paths=['../../secret.txt']))

    def test_custom_reader_cannot_replace_known_native_parser(self):
        (self.save/'slot.dat').write_bytes(struct.pack('<10i',1,0,2025,6,2,1,12,30,0,0))
        result=self.agent.batch(dict(root='save',path='slot.dat',reader='custom',
            options=dict(source='def extract(data, metadata): return {"caption":"invented"}')))
        self.assertEqual(result['failed'],1)
        self.assertIn('Use the installed siglus_save reader',result['errors'][0]['error'])

    def test_truncated_response_retries_with_larger_output_budget_once(self):
        transport=self.agent.api['TRANSPORT']
        from unittest.mock import Mock
        error=transport['CompletionError']('length',True,max_tokens=16000,finish_reason='length')
        call=Mock(side_effect=[error,dict(message=dict(content='ok'),elapsed_ms=20)])
        with patch.dict(transport,complete=call):
            ANALYSIS['AnalysisAgent'].complete(self.agent,[],[])
        self.assertEqual([c.kwargs['max_tokens'] for c in call.call_args_list],[16000,32000])

    def test_recap_ignores_only_outer_whitespace_and_still_cuts_at_anchor(self):
        self.agent.emit('save.json',dict(caption='当前句子\r\n\r\n'))
        self.agent.emit('script.json',dict(rows=['先前消息',dict(parts=['当前句子','未来消息'])]))
        self.agent.emit('s.json',dict(input_root='save',records=[dict(output='save.json',input=str(self.save/'slot.dat'),name='slot.dat',sha256='save')]))
        self.agent.emit('t.json',dict(input_root='work',records=[dict(output='script.json',input=str(self.game/'scene'),name='scene',sha256='script')]))
        self.agent.batches=['s.json','t.json']
        args=dict(save_file='slot.dat',save_pointer='/caption',script_file='scene',script_pointer='/rows',explanation='caption')
        self.agent.dispatch('select_recap_context',args)
        self.assertNotIn('未来消息',json.dumps(self.agent.recap_context,ensure_ascii=False))
        self.assertEqual(self.agent.recap_context['anchor'],'当前句子\r\n\r\n')
        self.agent.emit('script.json',dict(rows=['当前句子',' 当前句子\n']))
        with self.assertRaisesRegex(ValueError,'matches=2'):self.agent.dispatch('select_recap_context',args)
        self.agent.emit('script.json',dict(rows=['当前 句子']))
        with self.assertRaisesRegex(ValueError,'matches=0'):self.agent.dispatch('select_recap_context',args)

    def test_multilingual_projection_preserves_native_indexes_and_full_source(self):
        value=dict(scenes=[dict(label='start',texts=[None,[None,None,[[None,'original'],['名字','译文','另一显示形式']]]])])
        before=json.dumps(value,ensure_ascii=False)
        projected=READS['structured_dialogue'](value)
        row=projected[0]['records'][0]
        self.assertEqual(row['record_index'],1)
        self.assertEqual(row['variants'][1]['text'],'译文')
        self.assertEqual(row['variants'][1]['text_pointer'],'/value/scenes/0/texts/1/2/1/1')
        self.assertEqual(json.dumps(value,ensure_ascii=False),before)
        self.assertEqual(READS['anchor_hints'](dict(core=dict(bookMarkInfo=dict(lastText='当前句子')))),
            [dict(pointer='/value/core/bookMarkInfo/lastText',value='当前句子',level='candidate')])

    def test_effective_localized_batch_does_not_double_count_original_menus(self):
        value=self.result()
        for index,name,text in [('original.json','original/script.ast','Go'),('localized.json','localized/script.ast','出发')]:
            self.agent.emit(index,dict(input_root='work',records=[dict(input=name,name=name,output=name+'.json',
                menu_locations=['/choice'],known_options=[dict(pointer='/choice/0',variants=[text])])]))
        self.agent.batches=['original.json','localized.json']
        value['graph']['nodes'].append(dict(id='c',label='是否出发',kind='choice',evidence='O1'))
        value['graph']['edges']=[dict(**{'from':'s','to':'c'},label='出发',kind='choice',inferred=False,evidence='O1')]
        with self.assertRaisesRegex(ValueError,'2 distinct menu'):self.agent.submit_route(value)
        value['scope'].update(script_indexes=['localized.json'],resource_selection='Localized overlay supplies the displayed text; original was inspected only for structure.')
        self.agent.submit_route(value)
        self.assertEqual(self.agent.route_draft['scope']['script_indexes'],['localized.json'])
        value['scope']['script_indexes']=['unknown.json']
        with self.assertRaisesRegex(ValueError,'existing read_resources'):self.agent.submit_route(value)
        value['scope']['script_indexes']=['original.json']
        with self.assertRaisesRegex(ValueError,'already decoded this enabled option'):self.agent.submit_route(value)


if __name__ == '__main__': unittest.main()
