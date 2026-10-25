"""Generic save-first workflow receipts; synthetic bytes and no host/model calls."""
import copy
import importlib.util
import os
import pathlib
import struct
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location(
    'save_preflight_checks', pathlib.Path(__file__).with_name('save_preflight.py'))
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


class SavePreflightTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        for name in ('game', 'save', 'work', 'skill'):
            (self.root / name).mkdir()
        self.raw = b'HEAD' + struct.pack('<I', 7) + b'STORY'
        self.save = self.root / 'save' / 'current.bin'
        self.save.write_bytes(self.raw)
        self.helper = self.root / 'game' / 'helper.bin'
        self.helper.write_bytes(b'format dependency')
        self.required = ['current.bin']
        self.span_calls = 0
        self.scalar_calls = 0
        self.view_calls = 0
        self.store = self.new_store()

    def resolve(self, root, relative):
        base = (self.root / root).resolve()
        path = (base / relative).resolve()
        if not path.is_relative_to(base):
            raise ValueError('outside allowed root')
        return path

    def view(self, data, steps):
        self.view_calls += 1
        if not isinstance(steps, list):
            raise ValueError('invalid view')
        for step in steps:
            if step == dict(op='xor', key='ff'):
                data = bytes(value ^ 255 for value in data)
            else:
                raise ValueError('unsupported fixture transform')
        return data

    def span(self, data, proof):
        self.span_calls += 1
        if 'view' in proof:
            data = self.view(data, proof['view'])
        offset, length = proof['offset'], proof['length']
        if not 0 <= offset <= len(data) or not 0 < length <= len(data) - offset:
            raise ValueError('span outside bytes')
        return data[offset:offset + length].decode(proof['encoding'])

    def integer(self, data, offset, width=4, endian='little'):
        self.scalar_calls += 1
        if not 0 <= offset <= len(data) or not 0 < width <= len(data) - offset:
            raise ValueError('integer outside bytes')
        return int.from_bytes(data[offset:offset + width], endian)

    def new_store(self, **overrides):
        options = dict(resolve_path=self.resolve, replay_span=self.span,
                       required_saves=lambda: self.required,
                       integer=self.integer, view=self.view)
        options.update(overrides)
        return preflight.SavePreflight(**options)

    def header(self):
        return dict(role='container_header', value='HEAD',
                    proof=dict(offset=0, length=4, encoding='ascii'))

    def counter(self):
        return dict(role='native_state', value=7,
                    proof=dict(offset=4, width=4, endian='little'))

    def candidate(self):
        return dict(path='current.bin', classification='progress_candidate',
                    observations=[self.header(), self.counter()],
                    explanation='Native fields are available for current-node research.')

    def needs_context(self):
        return dict(path='current.bin', classification='needs_context',
                    observations=[self.header()], explanation='Need one additional state fact.',
                    context_plan=dict(missing_fact='Which native field identifies the current node?',
                        targets=[dict(root='save', path='current.bin'),
                                 dict(root='game', path='helper.bin')]))

    def test_script_work_requires_preflight_before_existing_resource_stages(self):
        for intent in ('script_sample', 'script_batch', 'finish'):
            with self.subTest(intent=intent), self.assertRaises(preflight.PreflightRequired) as raised:
                self.store.check_intent(intent)
            self.assertEqual(raised.exception.pending[0]['reason'], 'not_inspected')
        for intent in ('inspect', 'resource_helper', 'other'):
            self.assertTrue(self.store.check_intent(intent)['allowed'])

    def test_header_alone_cannot_establish_progress_candidate(self):
        args = self.candidate()
        args['observations'] = [self.header()]
        with self.assertRaisesRegex(ValueError, 'container header alone'):
            self.store.record(args)
        self.assertEqual(self.store.records, {})

    def test_receipt_binds_path_hash_and_size_without_verifying_a_node(self):
        receipt = self.store.record(self.candidate())
        self.assertEqual(receipt['input']['path'], str(self.save.resolve()))
        self.assertEqual(receipt['input']['size'], len(self.raw))
        self.assertEqual(len(receipt['input']['sha256']), 64)
        self.assertIs(receipt['verified_current_node'], False)
        result = self.store.check_intent('script_batch')
        self.assertTrue(result['allowed'])
        self.assertIs(result['verified_current_node'], False)
        self.assertEqual((self.span_calls, self.scalar_calls), (1, 1))

    def test_wrong_span_or_scalar_value_never_creates_a_receipt(self):
        for item in (dict(self.header(), value='FAKE'), dict(self.counter(), value=8),
                     dict(self.counter(), value=True)):
            args = self.candidate()
            args['observations'] = [item]
            with self.subTest(item=item), self.assertRaises(ValueError):
                self.store.record(args)
            self.assertEqual(self.store.records, {})

    def test_scalar_views_are_independently_replayed(self):
        self.save.write_bytes(bytes(value ^ 255 for value in self.raw))
        item = self.counter()
        item['proof']['view'] = [dict(op='xor', key='ff')]
        args = self.candidate()
        args['observations'] = [item]
        self.store.record(args)
        self.assertEqual((self.view_calls, self.scalar_calls), (1, 1))
        self.assertTrue(self.store.check_intent('script_batch')['allowed'])

    def test_proof_callback_interface_also_accepts_one_scalar_replayer(self):
        def replay_scalar(data, proof):
            return self.integer(data, proof['offset'], proof['width'], proof['endian'])
        store = self.new_store(integer=None, replay_scalar=replay_scalar)
        store.record(self.candidate())
        self.assertTrue(store.check_intent('script_sample')['allowed'])

    def test_needs_context_allows_focused_research_but_requires_a_new_record_before_batch(self):
        self.store.record(self.needs_context())
        for intent in ('inspect', 'resource_helper', 'script_sample'):
            self.assertTrue(self.store.check_intent(intent)['allowed'])
        for intent in ('script_batch', 'finish'):
            with self.subTest(intent=intent), self.assertRaises(preflight.PreflightRequired) as raised:
                self.store.check_intent(intent)
            self.assertEqual(raised.exception.pending[0]['reason'], 'context_recheck_required')
        self.store.record(self.needs_context())
        with self.assertRaises(preflight.PreflightRequired):
            self.store.check_intent('script_batch')
        self.store.record(self.candidate())
        self.assertTrue(self.store.check_intent('script_batch')['allowed'])

    def test_context_plan_must_identify_one_fact_and_real_files_including_its_save(self):
        mutations = [None, dict(missing_fact='', targets=[]),
                     dict(missing_fact='First fact\nSecond fact', targets=[]),
                     dict(missing_fact='One fact', targets=[dict(root='game', path='helper.bin')]),
                     dict(missing_fact='One fact', targets=[dict(root='save', path='missing.bin')]),
                     dict(missing_fact='One fact', targets=[dict(root='save', path='.')])]
        for plan in mutations:
            args = self.needs_context()
            args['context_plan'] = plan
            with self.subTest(plan=plan), self.assertRaises(ValueError):
                self.store.record(args)
        self.assertEqual(self.store.records, {})

    def test_changed_save_or_dependency_invalidates_preflight(self):
        self.store.record(self.needs_context())
        self.helper.write_bytes(b'different format dependency')
        with self.assertRaises(preflight.PreflightRequired) as changed:
            self.store.check_intent('script_sample')
        self.assertEqual(changed.exception.pending[0]['reason'], 'context_target_changed')
        self.store.record(self.candidate())
        self.save.write_bytes(b'HEAD' + struct.pack('<I', 8) + b'STORY')
        with self.assertRaises(preflight.PreflightRequired) as stale:
            self.store.check_intent('script_batch')
        self.assertEqual(stale.exception.pending[0]['reason'], 'save_bytes_changed')

    def test_dynamic_required_targets_cannot_be_replaced_by_an_older_receipt(self):
        self.store.record(self.candidate())
        (self.root / 'save' / 'newest.bin').write_bytes(self.raw)
        self.required[:] = ['newest.bin']
        with self.assertRaises(preflight.PreflightRequired) as raised:
            self.store.check_intent('script_batch')
        self.assertEqual(raised.exception.pending, [dict(path='newest.bin', reason='not_inspected')])

    def test_all_required_saves_must_be_inspected_and_unknown_targets_do_not_pass(self):
        self.store.record(self.candidate())
        self.required.append('second.bin')
        (self.root / 'save' / 'second.bin').write_bytes(self.raw)
        with self.assertRaises(preflight.PreflightRequired):
            self.store.check_intent('script_batch')
        args = dict(self.candidate(), path='second.bin')
        self.store.record(args)
        self.assertEqual(self.store.check_intent('script_batch')['checked_targets'], 2)
        self.required.clear()
        with self.assertRaises(preflight.PreflightRequired) as raised:
            self.store.check_intent('script_sample')
        self.assertEqual(raised.exception.code, 'save_targets_unknown')

    def test_paths_cannot_escape_named_roots(self):
        for relative in ('../game/helper.bin', '..\\game\\helper.bin',
                         str(self.save.resolve()), 'C:\\outside\\save.bin'):
            with self.subTest(path=relative), self.assertRaises(ValueError):
                self.store.record(dict(self.candidate(), path=relative))

    def test_status_exposes_pending_and_context_without_claiming_a_node(self):
        status = self.store.status()
        self.assertFalse(status['ready_for_script_sample'])
        self.assertEqual(status['pending'][0]['reason'], 'not_inspected')
        self.store.record(self.needs_context())
        status = self.store.status()
        self.assertTrue(status['ready_for_script_sample'])
        self.assertFalse(status['ready_for_script_batch'])
        self.assertEqual(status['needs_context'], ['current.bin'])
        self.assertIs(status['verified_current_node'], False)

    def test_serialised_records_replay_original_proofs_on_restore(self):
        self.store.record(self.candidate())
        saved = self.store.serialise()
        restored = self.new_store()
        with mock.patch.object(restored, 'replay_span', wraps=restored.replay_span) as replay:
            result = restored.restore(saved)
        replay.assert_called_once()
        self.assertEqual(result, dict(restored=1, discarded=0, compatible=True))
        self.assertTrue(restored.check_intent('script_batch')['allowed'])

    def test_old_empty_and_stale_checkpoints_never_satisfy_preflight(self):
        for state in (None, {}, dict(version=0, records=[]), dict(version=1, records=[])):
            restored = self.new_store()
            restored.restore(state)
            with self.subTest(state=state), self.assertRaises(preflight.PreflightRequired):
                restored.check_intent('script_batch')
        self.store.record(self.candidate())
        saved = self.store.serialise()
        self.save.write_bytes(self.raw + b'changed')
        restored = self.new_store()
        self.assertEqual(restored.restore(saved)['discarded'], 1)
        with self.assertRaises(preflight.PreflightRequired):
            restored.check_intent('script_batch')

    def test_restore_drops_changed_dependencies_and_forged_node_claims(self):
        self.store.record(self.needs_context())
        saved = self.store.serialise()
        self.helper.write_bytes(b'changed')
        restored = self.new_store()
        self.assertEqual(restored.restore(saved)['discarded'], 1)
        self.assertEqual(restored.records, {})
        saved['records'][0]['verified_current_node'] = True
        self.assertEqual(restored.restore(saved)['discarded'], 1)

    def test_receipt_and_serialisation_are_detached_from_callers_mutations(self):
        args = self.candidate()
        receipt = self.store.record(args)
        args['observations'][1]['value'] = 99
        receipt['input']['sha256'] = 'invented'
        exported = self.store.serialise()
        exported['records'][0]['observations'][1]['value'] = 99
        self.assertTrue(self.store.check_intent('script_batch')['allowed'])
        self.assertEqual(self.store.serialise()['records'][0]['observations'][1]['value'], 7)

    def test_tool_intent_mapping_is_supplied_by_the_host(self):
        classifier = mock.Mock(return_value='inspect')
        result = self.store.check('arbitrary_read', {'path': 'current.bin'}, intent_for_tool=classifier)
        classifier.assert_called_once_with('arbitrary_read', {'path': 'current.bin'})
        self.assertTrue(result['allowed'])

    def test_large_context_archives_use_streamed_hashes_and_cached_scheduling_checks(self):
        large = self.root / 'game' / 'large.archive'
        with large.open('wb') as handle:
            handle.seek(preflight.MAX_BYTES + 17)
            handle.write(b'archive-tail')
        args = self.needs_context()
        args['context_plan']['targets'][1]['path'] = large.name
        original_open = pathlib.Path.open
        archive_reads = []
        def tracked_open(path, *positional, **keywords):
            if path == large and positional and positional[0] == 'rb':
                archive_reads.append(path)
            return original_open(path, *positional, **keywords)
        with mock.patch.object(pathlib.Path, 'open', tracked_open):
            receipt = self.store.record(args)
            self.assertEqual(len(archive_reads), 1)
            for _ in range(3):
                self.assertTrue(self.store.status()['ready_for_script_sample'])
                self.assertTrue(self.store.check_intent('script_sample')['allowed'])
            self.assertEqual(len(archive_reads), 1)
            restored = self.new_store()
            self.assertEqual(restored.restore(self.store.serialise())['restored'], 1)
            self.assertEqual(len(archive_reads), 2)
        self.assertGreater(receipt['context_plan']['targets'][1]['input']['size'], preflight.MAX_BYTES)

    def test_context_metadata_change_rehashes_and_detects_changed_archive_bytes(self):
        self.store.record(self.needs_context())
        old = self.helper.stat()
        self.helper.write_bytes(b'changed dependency')
        os.utime(self.helper, ns=(old.st_atime_ns, old.st_mtime_ns + 1_000_000_000))
        with self.assertRaises(preflight.PreflightRequired) as raised:
            self.store.check_intent('script_sample')
        self.assertEqual(raised.exception.pending[0]['reason'], 'context_target_changed')

    def test_restore_rehashes_even_if_dependency_size_and_mtime_were_preserved(self):
        self.store.record(self.needs_context())
        saved = self.store.serialise()
        before = self.helper.stat()
        self.helper.write_bytes(b'X' * before.st_size)
        os.utime(self.helper, ns=(before.st_atime_ns, before.st_mtime_ns))
        # Scheduling deliberately trusts an unchanged metadata fingerprint.
        self.assertTrue(self.store.check_intent('script_sample')['allowed'])
        # A restored receipt is checked from its bytes, never from that cache.
        restored = self.new_store()
        self.assertEqual(restored.restore(saved)['discarded'], 1)
        self.assertEqual(restored.records, {})

    def test_save_input_limit_does_not_expand_with_archive_support(self):
        with self.save.open('wb') as handle:
            handle.seek(preflight.MAX_BYTES)
            handle.write(b'x')
        with self.assertRaisesRegex(ValueError, 'bounded file'):
            self.store.record(self.candidate())


if __name__ == '__main__':
    unittest.main()
