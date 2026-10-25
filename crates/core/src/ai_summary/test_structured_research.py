"""Offline research scheduling and typed inspection boundaries."""
import json
import pathlib
import re
import tempfile
import unittest

import test_agent_host as fixtures

host = fixtures.host


class StructuredResearchTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.agent = host.Agent(dict(game=str(self.root), save=str(self.root), skill=str(self.root),
                                    work=str(self.root / '.galgear-ai-summary' / 'fresh'), language='en-US'))
        self.target = self.root / 'chapter.dat'
        self.target.write_bytes(b'(const) %["message" => "Hello", "point" => 12]')

    def observe(self, name='read_bytes', path='chapter.dat'):
        args = dict(root='game', path=path, offset=0, length=64)
        description = self.agent.input_description(name, args)
        return self.agent.research.observe('inspection-' + str(len(self.agent.research.records)),
            dict(name=name, arguments=args), dict(success=True), inputs=description)['ref']

    def classify(self, role='story', refs=None, path='chapter.dat'):
        return self.agent.dispatch('record_script_role', dict(root='game', path=path, role=role,
            reason='Observed structural records distinguish research roles.',
            evidence_refs=refs or [self.observe(path=path)]))

    def test_role_requires_actual_nonstale_inspection_of_the_same_file(self):
        inventory = self.observe('list_files')
        with self.assertRaisesRegex(ValueError, 'exact script file'):
            self.classify(refs=[inventory])
        second = self.root / 'other.dat'
        second.write_bytes(b'Other input')
        with self.assertRaisesRegex(ValueError, 'exact script file'):
            self.classify(refs=[self.observe(path='other.dat')])
        good = self.observe()
        self.agent.research.mark_stale()
        with self.assertRaisesRegex(ValueError, 'nonstale'):
            self.classify(refs=[good])

    def test_system_sample_returns_to_resource_discovery_without_waiving_coverage(self):
        self.agent.required_script_sets['fixture'] = dict(archive='fixture.xp3', members=['chapter.dat', 'later.dat'])
        expected = self.agent.transcript_coverage()
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
        self.classify('system')
        self.assertEqual(self.agent.transcript_coverage(), expected)
        progress = self.agent.preparation_status()
        self.assertTrue(progress['script_sample_selection']['needs_story_sample'])
        self.assertEqual(self.agent.work_guidance()['stage'], 'resources')
        with self.assertRaisesRegex(ValueError, 'system/unresolved sample'):
            self.agent.dispatch('review_speakers', dict(reason='No names in this sample', evidence_refs=[self.observe()]))
        self.assertFalse(progress['ready_for_finish'])

    def test_story_role_selects_a_research_sample_without_creating_validation(self):
        result = self.classify()
        self.assertEqual(self.agent.representative_script, dict(root='game', path='chapter.dat'))
        self.assertEqual(self.agent.parts, {})
        self.assertEqual(self.agent.tests, {})
        self.assertFalse(self.agent.preparation_status()['ready_for_finish'])
        self.assertFalse(self.agent.preparation_status()['script_sample_selection']['needs_story_sample'])
        self.assertFalse(result['host_verified'])

    def test_changed_input_drops_the_role_without_reusing_its_old_findings(self):
        self.classify()
        self.target.write_bytes(b'Changed bytes')
        self.assertEqual(self.agent.current_script_roles(), {})
        self.assertTrue(self.agent.preparation_status()['script_sample_selection']['needs_story_sample'])

    def test_typed_inspection_preserves_native_field_identity_and_exact_json_locator(self):
        result = self.agent.dispatch('inspect_structured', dict(root='game', path='chapter.dat',
            document=dict(syntax='tjs_const', encoding='ascii', view=[]), field_path=['message']))
        self.assertEqual(result['node']['type'], 'string')
        self.assertEqual(result['node']['value'], 'Hello')
        reference = result['read_node']
        parsed = json.loads((self.agent.work / reference['path']).read_text('utf-8'))
        self.assertEqual(host.json_pointer_value(parsed, reference['pointer']), result['node'])
        self.assertFalse(result['host_verified_current_node'])
        self.assertEqual(self.agent.parts, {})

    def test_typed_inspection_rejects_partial_documents_and_missing_paths(self):
        self.target.write_bytes(self.target.read_bytes() + b' trailing code()')
        with self.assertRaises(ValueError):
            self.agent.dispatch('inspect_structured', dict(root='game', path='chapter.dat',
                document=dict(syntax='tjs_const', encoding='ascii', view=[])))
        self.target.write_bytes(b'(const) %["message" => "Hello"]')
        with self.assertRaisesRegex(ValueError, 'existing typed'):
            self.agent.dispatch('inspect_structured', dict(root='game', path='chapter.dat',
                document=dict(syntax='tjs_const', encoding='ascii', view=[]), field_path=['missing']))

    def test_preflight_replays_typed_integer_without_promoting_it_to_a_current_node(self):
        arguments = dict(path='chapter.dat', classification='progress_candidate',
            explanation='A native structural observation for preparation only.',
            observations=[dict(role='coordinate', value=12, proof=dict(kind='typed_field',
                document=dict(syntax='tjs_const', encoding='ascii', view=[]), path=['point'], type='integer'))])
        result = self.agent.dispatch('record_save_preflight', arguments)
        self.assertFalse(result['receipt']['verified_current_node'])
        self.assertEqual(self.agent.current_validated_saves(), {})
        arguments['observations'][0]['value'] = 13
        with self.assertRaisesRegex(ValueError, 'disagrees'):
            self.agent.dispatch('record_save_preflight', arguments)

    def speaker_review_observations(self, root='game'):
        self.agent.parts[str(self.target)] = [dict(source='chapter.dat', position=0, text='Hello', speaker=None)]
        records = []
        for name, args in (('list_files', dict(root='game', path='.')),
                           ('inspect_structured', dict(root=root, path='chapter.dat',
                            document=dict(syntax='tjs_const', encoding='ascii', view=[])))):
            result = self.agent.dispatch(name, args)
            records.append(self.agent.research.observe('speaker-check-' + str(len(self.agent.research.records)),
                dict(name=name, arguments=args), result,
                inputs=self.agent.input_description(name, args))['ref'])
        return records

    def speaker_review(self, references):
        return self.agent.dispatch('review_speakers', dict(reason='The inspected resource has no recoverable displayed name association.',
                                                          evidence_refs=references))

    def test_complete_typed_resource_inspection_can_support_an_inventory_backed_review(self):
        references = self.speaker_review_observations()
        with self.assertRaisesRegex(ValueError, 'actual game inventory'):
            self.speaker_review(references[1:])
        result = self.speaker_review(references)
        self.assertFalse(result['speaker_review_required'])
        self.assertEqual(self.agent.speaker_review['evidence_refs'], references)
        self.assertEqual(self.agent.tests, {})

    def test_typed_speaker_review_rejects_changed_or_stale_resource_observations(self):
        references = self.speaker_review_observations()
        self.target.write_bytes(self.target.read_bytes().replace(b'Hello', b'World'))
        with self.assertRaisesRegex(ValueError, 'current host-stamped game/work resource'):
            self.speaker_review(references)
        self.agent.research.records[references[1]]['stale'] = True
        with self.assertRaisesRegex(ValueError, 'successful current host-stamped'):
            self.speaker_review(references)
        self.assertIsNone(self.agent.speaker_review)

    def test_save_observations_and_known_companion_names_cannot_waive_speaker_bindings(self):
        references = self.speaker_review_observations(root='save')
        with self.assertRaisesRegex(ValueError, 'game/work resource'):
            self.speaker_review(references)
        references = self.speaker_review_observations()
        self.agent.speaker_resources['verified_names'] = {'native_name': dict(id=7, text='Name')}
        with self.assertRaisesRegex(ValueError, 'cannot be waived'):
            self.speaker_review(references)
        self.assertIsNone(self.agent.speaker_review)

    def test_preflight_role_schema_matches_the_existing_ascii_identifier_validator(self):
        function = next(item['function'] for item in host.TOOLS if item['function']['name'] == 'record_save_preflight')
        schema = function['parameters']['properties']['observations']['items']['properties']['role']
        self.assertEqual((schema['minLength'], schema['maxLength']), (1, 48))
        for role in ('a', 'observed_field_9', 'a'*48, '', 'A', 'a-b', 'a b', '状态', 'a'*49, 'a\n'):
            schema_accepts = bool(re.search(schema['pattern'], role)) and schema['minLength'] <= len(role) <= schema['maxLength']
            try:
                self.agent.save_preflight._observation(b'Hello', dict(role=role, value='Hello',
                    proof=dict(offset=0, length=5, encoding='ascii')))
                validator_accepts = True
            except ValueError:
                validator_accepts = False
            self.assertEqual(schema_accepts, validator_accepts, repr(role))


if __name__ == '__main__':
    unittest.main()
