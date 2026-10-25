"""One saved scene field must not be promoted to current-caption evidence."""
import pathlib
import runpy
import unittest

evidence = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


class SceneCaptionSemanticsTests(unittest.TestCase):
    def test_one_field_can_only_support_scene_coverage(self):
        data = b'Scene'
        span = dict(offset=0, length=5, encoding='ascii')
        dialogue = [dict(source='script', position=12, text='Scene')]
        facts = dict(source='script', line=12, scene='Scene', anchors=['Scene'],
                     evidence=dict(scene=dict(span), anchor=dict(span)))
        with self.assertRaisesRegex(ValueError, 'One preview field cannot prove both'):
            evidence['save_facts'](data, facts, dialogue)
        limited = dict(source='', line=0, scene='Scene', anchors=[], warnings=['Only a scene field is available.'], evidence=dict(scene=span))
        self.assertEqual(evidence['save_facts'](data, limited, dialogue)['anchors'], [])


if __name__ == '__main__':
    unittest.main()
