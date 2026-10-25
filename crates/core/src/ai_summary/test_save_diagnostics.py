"""Save schema errors identify the field and safe structure without story text."""
import pathlib
import runpy
import unittest


checks = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


class SaveDiagnosticsTests(unittest.TestCase):
    def test_null_scene_reports_string_and_the_supported_absent_forms(self):
        facts = dict(anchors=[], scene=None)
        with self.assertRaises(ValueError) as caught:
            checks['save_facts'](b'', facts, [])
        message = str(caught.exception)
        self.assertIn('facts.scene must be a string', message)
        self.assertIn('omit facts.scene or use an empty string', message)
        self.assertIn('do not return null', message)
        self.assertIn('Observed type=null', message)
        self.assertIsNone(facts['scene'])  # The verifier does not silently repair a failed parser.

    def test_invalid_scene_types_report_only_the_type(self):
        for scene in (False, 12, ['Private scene label'], {'private': 'Private scene label'}):
            with self.subTest(kind=type(scene).__name__):
                with self.assertRaises(ValueError) as caught:
                    checks['save_facts'](b'', dict(anchors=[], scene=scene), [])
                message = str(caught.exception)
                self.assertIn('Observed type=' + type(scene).__name__, message)
                self.assertNotIn('Private scene label', message)

    def test_scene_length_error_counts_utf8_bytes_without_echoing_the_scene(self):
        scene = '\u79d8' * 342
        with self.assertRaises(ValueError) as caught:
            checks['save_facts'](scene.encode(), dict(anchors=[], scene=scene), [])
        message = str(caught.exception)
        self.assertIn('facts.scene', message)
        self.assertIn('at most 1024 UTF-8 bytes', message)
        self.assertIn('Observed type=str, utf8_bytes=1026', message)
        self.assertNotIn('\u79d8', message)

    def test_exact_limit_scene_still_requires_matching_save_bytes(self):
        scene = '\u00e9' * 512
        data = scene.encode()
        facts = dict(anchors=[], scene=scene,
                     evidence=dict(scene=dict(offset=0, length=1024, encoding='utf-8')))
        self.assertEqual(checks['save_facts'](data, facts, [])['scene'], scene)
        wrong = dict(anchors=[], scene=scene,
                     evidence=dict(scene=dict(offset=0, length=1024, encoding='utf-8')))
        with self.assertRaisesRegex(ValueError, 'matching save byte evidence'):
            checks['save_facts'](b'x' * 1024, wrong, [])

    def test_omitted_and_empty_scene_remain_supported_without_scene_evidence(self):
        for facts in (dict(anchors=[]), dict(anchors=[], scene='')):
            with self.subTest(has_scene='scene' in facts):
                result = checks['save_facts'](b'', facts, [])
                self.assertEqual(result['anchors'], [])
                self.assertFalse(result.get('scene'))


if __name__ == '__main__':
    unittest.main()
