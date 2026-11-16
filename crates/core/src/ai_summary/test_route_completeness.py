"""大图文件提交、重复选项和自动分流的离线回归；不调用模型。"""
import json
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

BASE = Path(__file__).parent
API = runpy.run_path(str(BASE/'agent_host.py'))
ANALYSIS = runpy.run_path(str(BASE/'analysis_agent.py'))


class RouteCompletenessTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        for name in ('game','save','skill','work'):
            (root/name).mkdir()
        (root/'game/engine.exe').write_bytes(b'MZ fixture')
        self.agent = ANALYSIS['AnalysisAgent'](dict(
            game=str(root/'game'),save=str(root/'save'),skill=str(root/'skill'),
            work=str(root/'game/work'),provider='deepseek',model='fixture',token='fixture',
            language='zh-CN',save_candidates=[]), API)
        self.agent.batches=['unavailable-fixture-index.json']
        self.agent.observations={'O1':dict(tool='query_data',error=None)}

    def options(self,texts):
        self.agent.emit(self.agent.batches[0],dict(records=[dict(name='fixture',
            known_options=[dict(variants=[text],pointer=f'/menu/{index}',disabled=False)
                for index,text in enumerate(texts)])]))

    def payload(self):
        return dict(script_language='zh-CN', graph=dict(characters=[],
            nodes=[dict(id='start',label='开始',kind='start',evidence='O1')],
            edges=[],uncertainties=[]), evidence_refs=['O1'],
            scope=dict(flow_basis=dict(kind='script_control_flow',ref='O1',locator='fixture')),
            limitations=[])

    def test_large_file_preserves_repeated_choice_occurrences(self):
        self.options(['同意','拒绝'])
        value=self.payload()
        graph=value['graph']
        previous='start'
        for index in range(70):
            name=f'menu{index}'
            graph['nodes'].append(dict(id=name,label='途中对话',kind='choice',
                decision_kind='player',evidence='O1 '+('剧情来源说明'*25)))
            for label in (['继续'] if index==0 else ['同意','拒绝']):
                graph['edges'].append(dict(**{'from':previous,'to':name},label=label,
                    kind='flow' if index==0 else 'choice',pc=f'scene:{index}:{label}',
                    evidence='O1 '+('保留发生位置'*25),inferred=False))
            previous=name
        self.assertGreater(len(json.dumps(graph,ensure_ascii=False).encode()),24000)
        self.agent.emit('large-route.json',value)
        self.agent.submit_route(dict(route_file='large-route.json'))
        saved=self.agent.route_draft['graph']
        self.assertEqual(len(saved['nodes']),71)
        self.assertEqual(len(saved['edges']),139)
        self.assertEqual(sum(e['label']=='同意' for e in saved['edges']),69)

    def test_automatic_condition_does_not_become_player_option(self):
        self.options(['前往港口','前往车站'])
        value=self.payload()
        graph=value['graph']
        graph['nodes'].extend([
            dict(id='check',label='好感度判定',kind='choice',decision_kind='condition',evidence='O1'),
            dict(id='menu',label='选择去向',kind='choice',decision_kind='player',evidence='O1'),
            dict(id='a',label='港口',kind='scene',evidence='O1'),
            dict(id='b',label='车站',kind='scene',evidence='O1')])
        for source,target,label in [('start','check',''),('check','menu','已满足'),('check','b','未满足'),
                                    ('menu','a','前往港口'),('menu','b','前往车站')]:
            graph['edges'].append(dict(**{'from':source,'to':target},label=label,
                kind='flow' if source=='start' else 'choice',evidence='O1',inferred=False))
        self.agent.submit_route(value)
        self.assertEqual([e['kind'] for e in graph['edges'] if e['from']=='check'],['flow','flow'])
        self.assertEqual([e['kind'] for e in graph['edges'] if e['from']=='menu'],['choice','choice'])

    def test_condition_cannot_hide_a_missing_player_menu(self):
        value=self.payload()
        value['graph']['nodes'].append(dict(id='condition',label='进度判定',
            kind='choice',decision_kind='condition',evidence='O1'))
        self.agent.emit(self.agent.batches[0],dict(records=[dict(name='fixture',
            input='fixture',menu_locations=['/actual-player-menu'])]))
        with self.assertRaisesRegex(ValueError,'distinct menu locations'):
            self.agent.submit_route(value)

    def test_archive_companion_is_read_and_registered_without_another_model_call(self):
        (self.agent.host.roots['game']/'story.arc').write_bytes(b'fixture')
        document=self.agent.host.roots['skill']/'commands/garbro.md'
        document.parent.mkdir()
        document.write_text('Fixture archive documentation',encoding='utf-8')
        (self.agent.work/'entry.lua').write_text(
            'function CanOpenGrand()\n return flags.first and flags.second\nend\n',encoding='utf-16')
        with patch.object(self.agent.host,'archive_backend',return_value='garbro-mod'), \
             patch.object(self.agent.host,'_dispatch',return_value=dict(control_script_outputs=['entry.lua'])):
            result=self.agent.dispatch('run_skill',dict(command='x',archive='story.arc',filter='*.scn'))
        self.assertNotIn('control_script_read_error',result)
        reads=result['control_script_reads']
        self.assertEqual(reads['succeeded'],1)
        self.assertIn(reads['index'],self.agent.batches)
        self.assertIn('flags.first and flags.second',reads['control_sources'][0]['hints'][0]['text'])


if __name__ == '__main__':
    unittest.main()
