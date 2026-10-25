"""The reuse boundary accepts data and emits claims, never verification receipts."""
import importlib.util
import pathlib
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location('adapter_library', pathlib.Path(__file__).with_name('adapter_library.py'))
library = importlib.util.module_from_spec(spec)
spec.loader.exec_module(library)


class AdapterLibraryTests(unittest.TestCase):
    def setUp(self):
        self.calls = []
        def invoke(data, metadata, profile):
            self.calls.append((data, metadata, profile))
            metadata['changed'] = True
            profile['changed'] = True
            return dict(rows=[], evidence=dict(offset=0, length=len(data)), verified_current_node=False)
        fake = dict(describe=lambda: dict(profile=dict(variant='fixture')))
        fake['program'] = lambda kind, profile: 'def extract(data, metadata):\n return ' + repr(profile)
        fake.update({operation + '_' + kind: invoke for operation in ('inspect', 'extract')
                     for kind in ('script', 'save')})
        patch = mock.patch.dict(library._LOADED, entis_cotopha=fake, clear=True)
        patch.start()
        self.addCleanup(patch.stop)

    def test_profile_and_metadata_are_data_copies(self):
        metadata, profile = {'dialogue': []}, {'variant': 'fixture'}
        result = library.extract('entis_cotopha', 'script', b'raw', metadata, profile)
        self.assertEqual(metadata, {'dialogue': []})
        self.assertEqual(profile, {'variant': 'fixture'})
        self.assertFalse(result['verified_current_node'])
        self.assertEqual(self.calls[0][0], b'raw')

    def test_registry_cannot_accept_a_model_supplied_path(self):
        for value in ('../outside.py', 'C:\\outside.py', 'entis_adapter.py', None):
            with self.subTest(adapter=value), self.assertRaisesRegex(ValueError, 'Unknown bundled'):
                library.inspect(value, 'save', b'raw')
        self.assertEqual(self.calls, [])

    def test_profiles_cannot_supply_executable_callbacks(self):
        for value in (dict(callback=lambda data: data), dict(number=float('nan'))):
            with self.subTest(profile=value), self.assertRaisesRegex(ValueError, 'ordinary JSON'):
                library.extract('entis_cotopha', 'script', b'raw', profile=value)
        self.assertEqual(self.calls, [])

    def test_input_profile_and_inspection_output_are_bounded(self):
        for data in (b'', 'path/to/file', b'x' * (library.MAX_INPUT + 1)):
            with self.subTest(type=type(data).__name__), self.assertRaisesRegex(ValueError, 'Adapter input'):
                library.inspect('entis_cotopha', 'save', data)
        with self.assertRaisesRegex(ValueError, 'profile exceeds'):
            library.inspect('entis_cotopha', 'save', b'raw', profile=dict(value='x' * library.MAX_PROFILE))
        library._LOADED['entis_cotopha']['inspect_save'] = lambda *args: dict(value='x' * (1024 * 1024))
        with self.assertRaisesRegex(ValueError, 'output exceeds'):
            library.inspect('entis_cotopha', 'save', b'raw')

    def test_description_and_capabilities_are_copied(self):
        first = library.capabilities()
        first[0]['version'] = -1
        self.assertEqual(library.capabilities()[0]['version'], 1)
        first = library.describe('entis_cotopha')
        first['profile']['variant'] = 'forged'
        self.assertEqual(library.describe('entis_cotopha')['profile']['variant'], 'fixture')

    def test_generated_program_is_a_source_snapshot(self):
        profile = {'variant': 'fixture', 'label': '\"; import os; #'}
        source = library.program('entis_cotopha', 'script', profile)
        namespace = {}
        exec(compile(source, '<fixture>', 'exec'), namespace)
        self.assertEqual(namespace['extract'](b'raw', {}), profile)
        self.assertNotIn('import adapter_library', source)
        self.assertNotIn('import galgear_adapters', source)
        library._LOADED['entis_cotopha']['program'] = lambda *args: 'x' * (library.MAX_PROGRAM + 1)
        with self.assertRaisesRegex(ValueError, 'program must'):
            library.program('entis_cotopha', 'script', profile)


if __name__ == '__main__':
    unittest.main()
