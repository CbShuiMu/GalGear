"""控制脚本线索不会因批量预览位置或正文过滤而消失。"""
from pathlib import Path
import runpy
import unittest

BASE=Path(__file__).parent
HINTS=runpy.run_path(str(BASE/'analysis_control_hints.py'))
ARCHIVES=runpy.run_path(str(BASE/'agent_archive_tools.py'))


class ControlHintTests(unittest.TestCase):
    def test_later_file_exposes_callback_and_visibility_condition(self):
        records=[dict(name=f'ordinary{i}.func',output=f'{i}.json') for i in range(10)]
        records.append(dict(name='menu.func',output='menu.json',control_hints=HINTS['text_hints'](
            '// fake unlock condition\nfunc,final,exp,state.entry="grand"\neval,final,this.visible=CanOpenGrand()\n')))
        values=HINTS['batch_hints'](records)
        self.assertEqual(len(values),1)
        self.assertEqual([h['line'] for h in values[0]['hints']],[2,3])
        self.assertEqual(values[0]['pointer'],'/text')
        self.assertIn('CanOpenGrand()',values[0]['hints'][1]['text'])

    def test_companions_include_other_script_types_but_not_media(self):
        names=['chapter.ks.scn','start.ks','system/ui.func','system/menu.tjs','bg/title.png','voice/end.ogg']
        result=ARCHIVES['companion_scripts'](names,['chapter.ks.scn'])
        self.assertEqual(set(result),{'start.ks','system/ui.func','system/menu.tjs'})
        self.assertEqual(len(ARCHIVES['companion_scripts'](names,names)),0)

    def test_preview_budget_does_not_modify_full_projection(self):
        hints=HINTS['text_hints']('\n'.join(f'unlock{i}=flag{i}' for i in range(10)))
        records=[dict(name='boot.lua',output='boot.json',control_hints=hints)]
        preview=HINTS['batch_hints'](records,limit=200)
        self.assertLess(len(preview[0]['hints']),len(hints))
        self.assertEqual(len(hints),10)

    def test_small_companions_are_automatic_but_media_and_framework_scans_are_not(self):
        names=['chapter.ks.scn','boot.ks','extension.tjs','title.png']
        self.assertEqual(set(ARCHIVES['automatic_control_scripts'](names,['chapter.ks.scn'])),
                         {'boot.ks','extension.tjs'})
        self.assertEqual(ARCHIVES['automatic_control_scripts'](names,['title.png']),[])
        names.extend(f'framework{i}.tjs' for i in range(20))
        self.assertEqual(ARCHIVES['automatic_control_scripts'](names,['chapter.ks.scn']),[])


if __name__=='__main__':
    unittest.main()
