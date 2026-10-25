"""Text-only unavailable reports identify the rejected observation and field."""
import copy
import json
import unittest

import test_preparation_unavailable as fixtures


host = fixtures.host


class UnavailableFieldFeedbackTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.PreparationUnavailableTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.agent = self.fixture.agent
        self.path = self.fixture.save()
        self.agent.required = {str(self.path)}
        self.previous_failure = copy.deepcopy(self.agent.last_failure)

    def failure(self, observations, cause, field):
        with self.assertRaises(ValueError) as caught:
            self.fixture.report(observations, reason='missing_progress_save')
        message = str(caught.exception)
        self.assertTrue(message.startswith(cause), message)
        self.assertIn(' at ' + field + '. Expected ', message)
        self.assertNotIn('Native menu state', message)
        self.assertIsNone(self.agent.unavailable)
        self.assertEqual(self.agent.last_failure, self.previous_failure)
        self.assertFalse((self.agent.work / 'unavailable.json').exists())
        return message

    def test_header_scalar_is_rejected_at_its_index_with_text_shape(self):
        header = dict(path=self.path.name, proof=dict(offset=0, width=4, endian='little'), value='header')
        state = self.fixture.observation(self.path)
        for index, observations in ((0, [header, state]), (1, [state, header])):
            with self.subTest(index=index):
                message = self.failure(observations, 'Invalid text byte evidence or encoding',
                                       'observations[' + str(index) + '].proof')
                self.assertIn('TEXT span {offset:', message)
                self.assertIn('integer/header scalar proofs {offset,width,endian} are not text observations', message)

    def test_text_proof_failures_keep_cause_index_and_supported_encoding_shape(self):
        state = self.fixture.observation(self.path)
        for proof, cause in ((None, 'Supply text byte evidence'),
            (dict(offset=0, length=1, encoding='uint32-le'), 'Invalid text byte evidence or encoding'),
            (dict(offset=self.path.stat().st_size, length=1, encoding='ascii'), 'Invalid text byte evidence or encoding'),
            (dict(offset=0, length=1, encoding='ascii', view=[dict(op='invented')]), 'Unsupported byte-view transform')):
            invalid = copy.deepcopy(state)
            invalid['proof'] = proof
            with self.subTest(cause=cause):
                message = self.failure([state, invalid], cause, 'observations[1].proof')
                self.assertIn('"utf-16-le"', message)
                self.assertIn('"gb18030"', message)

    def test_bad_item_value_and_path_have_separate_field_paths(self):
        state = self.fixture.observation(self.path)
        invalid_value = dict(state, value='Invented native value')
        self.failure([state, invalid_value], 'Unavailable observation disagrees', 'observations[1].value')
        self.failure([state, dict(state, path='missing.sav')], 'Unavailable observation must cite', 'observations[1].path')
        self.failure([state, dict(state, path='../outside.sav')], 'Path outside allowed root', 'observations[1].path')
        self.failure([state, dict(state, extra=True)], 'Unavailable observation needs path', 'observations[1]')

    def test_text_only_native_observations_still_report_unavailable(self):
        result = self.fixture.report([self.fixture.observation(self.path)], reason='missing_progress_save')
        self.assertEqual(result['status'], 'unavailable')
        self.assertEqual(result['reason'], 'missing_progress_save')
        persisted = json.loads((self.agent.work / 'unavailable.json').read_text('utf-8'))
        self.assertEqual(persisted, self.agent.unavailable)

    def test_standard_text_view_remains_accepted(self):
        state = self.fixture.observation(self.path)
        state['proof']['view'] = [dict(op='slice', offset=0, length=self.path.stat().st_size)]
        result = self.fixture.report([state], reason='missing_progress_save')
        self.assertEqual(result['status'], 'unavailable')

    def test_schema_exposes_text_shape_and_encoding_enum(self):
        tool = next(entry['function'] for entry in host.TOOLS if entry['function']['name'] == 'report_unavailable')
        observation = tool['parameters']['properties']['observations']['items']
        alternatives = observation['properties']['proof']['oneOf']
        proof = alternatives[0]
        self.assertEqual(proof['required'], ['offset', 'length', 'encoding'])
        self.assertEqual(proof['properties']['encoding']['enum'], sorted(host.EVIDENCE['ENCODINGS']))
        self.assertEqual(alternatives[1]['properties']['type']['enum'], ['string'])
        self.assertEqual(observation['properties']['value']['type'], 'string')
        self.assertIn('TEXT ONLY', tool['description'])
        self.assertIn('Integer/header scalar proofs', tool['description'])


if __name__ == '__main__':
    unittest.main()
