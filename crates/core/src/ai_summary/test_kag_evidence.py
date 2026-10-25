"""Multiple synthetic standard KAG games and altered bookmark rejection."""
import copy
import pathlib
import runpy
import unittest

import kag_evidence as kag


EVIDENCE = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


def bookmark(storage='scenario/chapter.ks', label='*entry', record='mainConductor', history=False):
    text = '%["core"=>%["'+record+'"=>%["storageName"=>"'+storage+'",'
    text += '"storageShortName"=>"'+storage.rsplit('/', 1)[-1]+'","curLabel"=>"'+label+'",'
    # These deliberately contradictory fields cannot select the resume node.
    text += '"curLine"=>999,"curPos"=>812,"lineBuffer"=>"unrelated preview"]]'
    if history:
        text += ',"history"=>[%["core"=>%["mainConductor"=>%["storageName"=>"scenario/chapter.ks",'
        text += '"storageShortName"=>"chapter.ks","curLabel"=>"*old"]]]]'
    return (text+']').encode('utf-16-le')


def link(record=None):
    return dict(kind='kag_label_resume', document=dict(syntax='tjs_const', encoding='utf-16-le'),
                record=['core', 'mainConductor'] if record is None else record)


class KagResumeTests(unittest.TestCase):
    def canonical(self, raw, encoding='utf-8', source='unpacked/effective/scenario/chapter.ks'):
        rows = []
        for entry in kag.label_index(raw, dict(encoding=encoding))['labels']:
            proof = dict(entry['evidence'], native=dict(kind='kag_label_resume', encoding=encoding, label=entry['alias']))
            rows.append(dict(position=entry['position'], text=entry['text'], speaker=None, evidence=proof))
        EVIDENCE['script_rows'](raw, dict(rows=rows))
        records = EVIDENCE['ADAPTERS']['attest_native_nodes'](raw, rows, span=EVIDENCE['span'],
            integer=EVIDENCE['integer'], view=EVIDENCE['view'])
        return [dict(source=source, position=row['position'], text=row['text'], speaker=None,
                     _native=records[row['position']]) for row in rows]

    def test_public_label_rules_tabs_newlines_omitted_names_and_aliases(self):
        raw = b';comment\r\n\t*entry|Page one\rFirst text\n*|Page two\r\nSecond text\n*entry|Page three\n'
        labels = kag.label_index(raw, dict(encoding='ascii'))['labels']
        self.assertEqual([entry['alias'] for entry in labels], ['*entry', '*entry:2', '*entry:3'])
        self.assertEqual([entry['heading'] for entry in labels], ['Page one', 'Page two', 'Page three'])
        self.assertEqual([entry['line'] for entry in labels], [1, 3, 5])
        for entry in labels:
            self.assertEqual(raw[entry['position']:entry['position']+entry['evidence']['length']].decode(), entry['text'])

    def test_full_encoded_sources_keep_original_byte_coordinates(self):
        text = '\ufeff;前缀😀\r\n\t*入口|页面\nNative words'
        for encoding in ('utf-8', 'utf-16-le', 'utf-16-be'):
            with self.subTest(encoding=encoding):
                raw = text.encode(encoding)
                entry = kag.label_index(raw, dict(encoding=encoding))['labels'][0]
                self.assertEqual(entry['position'], len(text[:text.index('*')].encode(encoding)))
                self.assertEqual(entry['alias'], '*入口')
        cp932 = ';日本語\n*label|ページ\n台詞'.encode('cp932')
        self.assertEqual(kag.label_index(cp932, dict(encoding='cp932'))['labels'][0]['alias'], '*label')

    def test_two_games_different_native_labels_restore_without_any_saved_caption(self):
        fixtures = [('chapter.ks', '*entry', ';Alpha\n*entry|First scene\nMessage after the label.'),
                    ('another.ks', '*branch:2', '*branch|Earlier\nText\n*|Chosen entrance\nLater words')]
        for name, saved_label, text in fixtures:
            with self.subTest(name=name):
                canonical = self.canonical(text.encode(), source='unpacked/effective/scenario/'+name)
                data = bookmark('scenario/'+name, saved_label)
                result = EVIDENCE['save_facts'](data, dict(anchors=[], evidence=dict(node_link=link())), canonical)
                self.assertEqual(result['anchors'], [])
                self.assertEqual(result['history'], [])
                self.assertEqual(result['resume'], dict(profile=kag.PROFILE, state_role='resume_node',
                    source=result['source'], label=saved_label, position=result['line']))
                self.assertEqual(result['line'], next(row['position'] for row in canonical
                                                     if row['_native'][0]['scope'] == saved_label))
                self.assertNotEqual(result['line'], 999)

    def test_historical_snapshot_cannot_replace_standard_authoritative_bookmark(self):
        canonical = self.canonical(b'*old|Past\nEarlier\n*entry|Restore\nPresent')
        data = bookmark(history=True)
        result = kag.resolve_resume(data, link(), canonical)
        self.assertEqual(result['resume']['label'], '*entry')
        with self.assertRaisesRegex(ValueError, 'arbitrary snapshots'):
            kag.resolve_resume(data, link(['history', 0, 'core', 'mainConductor']), canonical)
        with self.assertRaises(ValueError):
            kag.resolve_resume(bookmark(record='extraConductor'), link(), canonical)

    def test_fake_role_profile_and_model_resume_do_not_qualify(self):
        canonical = self.canonical(b'*entry|Resume\nNarrative')
        fake = dict(profile=kag.PROFILE, state_role='resume_node', source=canonical[0]['source'],
                    position=canonical[0]['position'], label='*entry')
        returned = EVIDENCE['save_facts'](bookmark(), dict(anchors=[], resume=fake), canonical)
        self.assertNotIn('resume', returned)
        for change in (dict(profile=kag.PROFILE), dict(state_role='current'), dict(current=True)):
            with self.subTest(fields=list(change)), self.assertRaises(ValueError):
                kag.resolve_resume(bookmark(), dict(link(), **change), canonical)

    def test_missing_different_and_ambiguous_source_or_labels_fail(self):
        canonical = self.canonical(b'*entry|Resume\nText')
        for data in (bookmark(label='*missing'), bookmark('different/chapter.ks'), bookmark(label='')):
            with self.subTest(bytes=len(data)), self.assertRaises(ValueError):
                kag.resolve_resume(data, link(), canonical)
        with self.assertRaisesRegex(ValueError, 'matches=2'):
            kag.resolve_resume(bookmark(), link(), canonical+copy.deepcopy(canonical))
        altered = bookmark().replace('chapter.ks'.encode('utf-16-le'), 'altered.ks'.encode('utf-16-le'), 1)
        with self.assertRaisesRegex(ValueError, 'storageName and storageShortName disagree'):
            kag.resolve_resume(altered, link(), canonical)

    def test_changed_label_bytes_or_forged_row_coordinates_cannot_be_attested(self):
        raw = b'*entry|Original\nText\n*other|Next'
        entry = kag.label_index(raw, dict(encoding='ascii'))['labels'][0]
        declaration = dict(kind='kag_label_resume', encoding='ascii', label='*entry')
        row = dict(position=entry['position'], text=entry['text'], speaker=None, evidence=entry['evidence'])
        kag.attest_label(raw, row, declaration)
        for change in (dict(position=2), dict(text='*entry|Invented'), dict(speaker='Name')):
            with self.subTest(fields=list(change)), self.assertRaises(ValueError):
                kag.attest_label(raw, dict(row, **change), declaration)
        with self.assertRaises(ValueError):
            kag.attest_label(raw.replace(b'Original', b'Modified'), row, declaration)

    def test_caption_and_resume_source_mismatch_are_rejected(self):
        canonical = self.canonical(b'*entry|Resume\nActual message')
        for fields in (dict(anchors=['Actual message']), dict(anchors=[], line=999),
                       dict(anchors=[], source='invented.ks')):
            with self.subTest(fields=fields), self.assertRaises(ValueError):
                EVIDENCE['save_facts'](bookmark(), dict(fields, evidence=dict(node_link=link())), canonical)

    def test_nxsc_profile_cannot_be_bypassed(self):
        canonical = self.canonical(b'*entry|Resume\nText')
        raw = b'NXSC'+b'\0'*16+(1).to_bytes(4, 'little')+'x'.encode('utf-16-le')
        with self.assertRaises(ValueError):
            EVIDENCE['save_facts'](raw, dict(anchors=[], evidence=dict(node_link=link())), canonical)

    def test_unknown_scanning_variants_and_mutated_cached_results_fail_closed(self):
        for raw in (b'*|No previous label', b'*x\n*x\n*x:2', b'*entry\0trailing'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                kag.label_index(raw, dict(encoding='ascii'))
        raw = b' *not-a-label\n*entry|Native\nText'
        result = kag.label_index(raw, dict(encoding='ascii'))
        self.assertEqual(len(result['labels']), 1)
        result['labels'][0]['alias'] = '*forged'
        self.assertEqual(kag.label_index(raw, dict(encoding='ascii'))['labels'][0]['alias'], '*entry')


if __name__ == '__main__':
    unittest.main()
