"""Production host keeps independently proved bookmark roles distinct."""
import json
import pathlib
import tempfile
import unittest

import test_agent_host as fixtures

host = fixtures.host
SCRIPT = '''import galgear_kag
def extract(data, metadata):
    rows=[]
    for entry in galgear_kag.label_index(data, {"encoding":"ascii","view":[]})["labels"]:
        proof=dict(entry["evidence"])
        proof["native"]={"kind":"kag_label_resume","encoding":"ascii","label":entry["alias"]}
        rows.append({"position":entry["position"],"text":entry["text"],"speaker":None,"evidence":proof})
    offset=0
    for line in data.splitlines(True):
        text=line.rstrip(b"\\r\\n")
        if text and not text.startswith(b"*"):
            rows.append({"position":offset,"text":text.decode("ascii"),"speaker":None,
                         "evidence":{"offset":offset,"length":len(text),"encoding":"ascii"}})
        offset+=len(line)
    return {"rows":rows}
'''
SAVE = '''def extract(data, metadata):
    return {"anchors":[],"evidence":{"node_link":{"kind":"kag_label_resume",
            "document":{"syntax":"tjs_const","encoding":"utf-16-le","view":[]},
            "record":["core","mainConductor"]}}}
'''


class HostResumeSemanticsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.script = self.root / 'chapter.ks'
        self.script.write_bytes(b'*entry|Chapter\nA synthetic message.\n')
        self.save = self.root / 'current.sav'
        self.save.write_bytes('%["core"=>%["mainConductor"=>%["storageName"=>"chapter.ks",'
                              '"storageShortName"=>"chapter.ks","curLabel"=>"*entry",'
                              '"curLine"=>999,"curPos"=>812]]]'.encode('utf-16-le'))
        self.agent = host.Agent(dict(game=str(self.root), save=str(self.root), skill=str(self.root),
            work=str(self.root / '.galgear-ai-summary' / 'fresh'), language='en-US',
            title='Synthetic KAG fixture', provider='deepseek', model='offline', required_saves=[str(self.save)]))
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True

    def parse(self):
        self.agent.dispatch('write_program', dict(kind='script', source=SCRIPT))
        self.agent.dispatch('test_program', dict(kind='script', root='game', path='chapter.ks'))
        fixtures.review_narration(self.agent)
        self.agent.dispatch('write_program', dict(kind='save', source=SAVE))
        return self.agent.dispatch('test_program', dict(kind='save', root='save', path='current.sav'))

    def test_verified_resume_satisfies_target_without_inventing_a_current_caption(self):
        result = self.parse()
        self.assertEqual(result['coverage'], 'resume_node')
        self.assertEqual(result['facts']['anchors'], [])
        self.assertEqual(result['facts']['history'], [])
        self.assertEqual(result['facts']['resume']['state_role'], 'resume_node')
        self.assertEqual(result['facts']['resume']['label'], '*entry')
        status = self.agent.preparation_status()
        self.assertEqual(status['dialogue_count'], 1)
        self.assertEqual(status['resume_marker_count'], 1)
        self.assertTrue(status['ready_for_finish'])
        self.assertEqual(status['verified_progress_roles'][str(self.save)], 'resume_node')
        preflight = self.agent.preflight_status()
        self.assertTrue(preflight['satisfied_by_verified_progress'])
        self.assertFalse(preflight['satisfied_by_current_nodes'])
        self.agent.check_model_tool('finish', {})
        prepared = self.agent.dispatch('finish', dict(engine='Standard KAG fixture', script_language='en-US',
            speaker_resolution='unavailable', notes='A resume entry is not last displayed text.'))
        self.assertEqual(prepared['validated_saves'][str(self.save)]['coverage'], 'resume_node')
        self.assertEqual(prepared['dialogue_count'], 1)
        self.assertEqual(prepared['artifact_rows_count'], 2)
        self.assertEqual(prepared['resume_marker_count'], 1)

    def test_label_only_extraction_preserves_markers_but_cannot_complete_story_preparation(self):
        self.script.write_bytes(b'*entry|Chapter\n')
        self.parse()
        status = self.agent.preparation_status()
        self.assertEqual(status['dialogue_count'], 0)
        self.assertEqual(status['resume_marker_count'], 1)
        self.assertFalse(status['ready_for_finish'])
        with self.assertRaisesRegex(ValueError, 'before finishing'):
            self.agent.dispatch('finish', dict(engine='fixture', script_language='en-US',
                speaker_resolution='unavailable', notes='Labels alone are not dialogue.'))

    def test_model_authored_resume_and_unknown_profiles_do_not_satisfy_a_target(self):
        self.parse()
        fake = dict(source='chapter.ks', line=0, anchors=[], resume=dict(profile='kag_parser_label_v1',
            state_role='resume_node', source='chapter.ks', label='*entry', position=0))
        for source in ('def extract(data, metadata): return ' + repr(fake),
                       SAVE.replace('"kag_label_resume"', '"unreviewed_resume"')):
            with self.subTest(source=source[:70]):
                self.agent.dispatch('write_program', dict(kind='save', source=source))
                with self.assertRaises((ValueError, RuntimeError)):
                    self.agent.dispatch('test_program', dict(kind='save', root='save', path='current.sav'))
                self.assertEqual(self.agent.current_validated_saves(), {})


if __name__ == '__main__':
    unittest.main()
