"""Save byte failures retain their cause and identify the exact proof field."""
import copy
import json
import unittest

import test_adapter_evidence as native_fixtures
import test_save_node_feedback as fixtures


host = fixtures.host
checks = host.EVIDENCE


class _SaveFailureAssertions:
    def failure(self, data, facts, rows, cause, field):
        with self.assertRaises(ValueError) as caught:
            checks['save_facts'](data, copy.deepcopy(facts), rows)
        message = str(caught.exception)
        self.assertTrue(message.startswith(cause), message)
        self.assertIn(' at ' + field + '. Expected ', message)
        self.assertEqual(message.count('. Expected '), 1)
        self.assertNotIn('PRIVATE_SYNTHETIC_CAPTION', message)
        return message


class SaveFieldFeedbackTests(_SaveFailureAssertions, unittest.TestCase):
    def fixture(self):
        data, facts, rows, _ = fixtures.SaveNodeFeedbackTests().fixture()
        return data, facts, rows

    def test_missing_text_proofs_have_distinct_field_paths(self):
        for field in ('source', 'anchor', 'scene'):
            data, facts, rows = self.fixture()
            if field == 'scene':
                facts['scene'] = 'Scene'
            else:
                facts['evidence'].pop(field)
            with self.subTest(field=field):
                message = self.failure(data, facts, rows, 'Supply text byte evidence', 'evidence.' + field)
                self.assertIn('TEXT span {offset:', message)
                self.assertIn('"utf-16-le"', message)

    def test_malformed_text_proofs_retain_the_original_error(self):
        data, original, rows = self.fixture()
        mutations = [(None, 'Supply text byte evidence'), ('proof', 'Supply text byte evidence'),
            (dict(offset=0, width=4, endian='little'), 'Invalid text byte evidence or encoding'),
            (dict(offset=0, length=1, encoding='uint32-le'), 'Invalid text byte evidence or encoding'),
            (dict(offset=len(data), length=1, encoding='ascii'), 'Invalid text byte evidence or encoding'),
            (dict(offset=0, length=-1, encoding='ascii'), 'Invalid text byte evidence or encoding'),
            (dict(offset=0, length=1, encoding='ascii', view=[dict(op='invented')]), 'Unsupported byte-view transform')]
        for proof, cause in mutations:
            facts = copy.deepcopy(original)
            facts['evidence']['anchor'] = proof
            with self.subTest(cause=cause, proof_type=type(proof).__name__):
                self.failure(data, facts, rows, cause, 'evidence.anchor')

    def test_position_shape_view_bounds_and_value_have_field_context(self):
        data, original, rows = self.fixture()
        for proof, cause, field in (
            (None, 'Current caption needs native position byte evidence', 'evidence.position'),
            (dict(offset=0, width=5), 'Integer evidence is outside', 'evidence.position'),
            (dict(offset=len(data), width=4), 'Integer evidence is outside', 'evidence.position'),
            (dict(offset=0, width=4, view=[dict(op='invented')]), 'Unsupported byte-view transform', 'evidence.position.view'),
            (dict(offset=0, width=4), 'Current save position byte evidence disagrees', 'evidence.position')):
            facts = copy.deepcopy(original)
            facts['evidence']['position'] = proof
            with self.subTest(field=field, cause=cause):
                self.failure(data, facts, rows, cause, field)

    def test_anchor_count_error_is_not_misreported_as_missing_text_evidence(self):
        data, facts, rows = self.fixture()
        facts['anchors'] *= 2
        self.failure(data, facts, rows, 'Current anchor needs exact byte evidence', 'facts.anchors')

    def test_history_failure_retains_its_array_index(self):
        data, facts, rows = self.fixture()
        facts['evidence']['history'] = [copy.deepcopy(facts['evidence']['anchor']), None]
        self.failure(data, facts, rows, 'Supply text byte evidence', 'evidence.history[1]')

    def test_same_stem_source_and_absent_scene_still_pass_with_resume_verification_upgrade(self):
        data, facts, rows = self.fixture()
        result = checks['save_facts'](data, copy.deepcopy(facts), rows)
        self.assertEqual(result['source'], facts['source'])
        self.assertEqual(result['anchors'], facts['anchors'])
        self.assertEqual(result['history'], [])
        self.assertEqual(checks['SAVE_VERIFICATION_VERSION'], 3)
        self.assertEqual(checks['SCRIPT_VERIFICATION_VERSION'], 3)

    def test_real_worker_stderr_preserves_the_field_without_an_error_envelope(self):
        data, facts, rows = self.fixture()
        facts['evidence'].pop('source')
        source = 'import json\ndef extract(data, metadata):\n return json.loads(' + repr(json.dumps(facts)) + ')'
        with self.assertRaises(RuntimeError) as caught:
            host.run_parser(source, data, dict(dialogue=rows), verify_save=True)
        message = str(caught.exception)
        self.assertIn('Supply text byte evidence {offset,length,encoding} at evidence.source. Expected ', message)
        self.assertNotIn('PRIVATE_SYNTHETIC_CAPTION', message)


class NativeSaveFieldFeedbackTests(_SaveFailureAssertions, unittest.TestCase):
    def fixture(self):
        fixture = native_fixtures.AdapterEvidenceTests()
        script, raw_rows = fixture.fixture()
        rows = fixture.canonical(script, raw_rows)
        data, link = fixture.saved()
        anchor_offset = len(data)
        data += b'Current'
        facts = dict(source=rows[0]['source'], line=rows[0]['position'], anchors=['Current'], evidence=dict(
            anchor=dict(offset=anchor_offset, length=7, encoding='ascii'), node_link=link))
        return data, facts, rows

    def test_node_link_callbacks_retain_the_deepest_field(self):
        mutations = [
            ('scope', None, 'Supply text byte evidence', 'evidence.node_link.scope'),
            ('scope', dict(offset=0, length=1, encoding='uint32-le'), 'Invalid text byte evidence', 'evidence.node_link.scope'),
            ('scope', dict(offset=0, length=1, encoding='ascii', view=[dict(op='invented')]), 'Unsupported byte-view transform', 'evidence.node_link.scope'),
            ('position', None, 'Saved native coordinate needs scalar', 'evidence.node_link.position'),
            ('position', dict(offset=0, length=1, encoding='ascii'), 'Saved native coordinate needs scalar', 'evidence.node_link.position'),
            ('position', dict(offset=10000, width=4), 'Integer evidence is outside', 'evidence.node_link.position'),
            ('position', dict(offset=0, width=4, view=[dict(op='invented')]), 'Unsupported byte-view transform', 'evidence.node_link.position.view')]
        for field, proof, cause, expected in mutations:
            data, facts, rows = self.fixture()
            facts['evidence']['node_link'][field] = proof
            with self.subTest(field=expected, cause=cause):
                self.failure(data, facts, rows, cause, expected)

    def test_missing_scope_and_position_identify_the_required_field(self):
        for field in ('scope', 'position'):
            data, facts, rows = self.fixture()
            facts['evidence']['node_link'].pop(field)
            with self.subTest(field=field):
                self.failure(data, facts, rows, 'Saved native node link', 'evidence.node_link.' + field)

    def test_no_attested_match_is_still_rejected_with_its_original_context(self):
        data, facts, rows = self.fixture()
        rows[0]['_native'] = []
        message = self.failure(data, facts, rows, 'Saved native scope and return address', 'evidence.node_link')
        self.assertIn('matches=0', message)
        self.assertNotIn('at evidence.node_link.scope.', message)

    def test_ambiguous_attested_matches_remain_rejected(self):
        data, facts, rows = self.fixture()
        rows.append(copy.deepcopy(rows[0]))
        message = self.failure(data, facts, rows, 'Saved native scope and return address', 'evidence.node_link')
        self.assertIn('matches=2', message)

    def test_invalid_outer_kind_is_not_misreported_as_a_missing_scope(self):
        for link, cause in (({'position': {}}, 'Saved native node link'),
                ({'kind': 'unknown', 'position': {}}, 'Saved native node link'),
                ({'kind': 'unknown', 'scope': {}, 'position': {}}, 'Unsupported saved native node link kind')):
            data, facts, rows = self.fixture()
            facts['evidence']['node_link'] = link
            with self.subTest(has_kind='kind' in link):
                message = self.failure(data, facts, rows, cause, 'evidence.node_link')
                self.assertNotIn('at evidence.node_link.scope.', message)

    def test_strict_scope_decoding_failure_keeps_its_field(self):
        data, facts, rows = self.fixture()
        facts['evidence']['node_link']['scope'] = dict(offset=len(data), length=1, encoding='ascii')
        self.failure(data + b'\xff', facts, rows, "'ascii' codec can't decode byte", 'evidence.node_link.scope')

    def test_verified_native_link_still_returns_the_same_node(self):
        data, facts, rows = self.fixture()
        result = checks['save_facts'](data, copy.deepcopy(facts), rows)
        self.assertEqual((result['source'], result['line'], result['anchors']),
                         (rows[0]['source'], rows[0]['position'], ['Current']))


if __name__ == '__main__':
    unittest.main()
