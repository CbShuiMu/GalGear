"""Synthetic tests for the bounded bundled Entis adapter; no model/game inputs."""
import copy
import json
from pathlib import Path
import runpy
import struct
import unittest


HERE = Path(__file__).resolve().parent
HOST_PATH = HERE / 'agent_host.py'
if not HOST_PATH.exists():
    HOST_PATH = HERE.parents[2] / 'crates/core/src/ai_summary/agent_host.py'
HOST = runpy.run_path(str(HOST_PATH))
ADAPTER = runpy.run_path(str(HERE / 'entis_adapter.py'))
TITLE = {'class': 'SaveDataTitle', 'version': 20140121, 'caption_field': 3}
SAVE_PROFILE = dict(message_callees=['synthetic_handler'], title_profile=TITLE,
                    text_argument=2, vm_profile='cotopha-frame-vector-v1')
SCRIPT_PROFILE = dict(message_callees=['synthetic_handler'], scalar_arguments=1,
    call_arity=2, speaker_argument=False,
    scope_transform=dict(kind='last_component', separator='@'))


def u32(value):
    return struct.pack('<I', value)


def text(value):
    raw = value.encode('utf-16-le')
    return u32(len(raw) // 2) + raw


def header(description, body_size=None):
    data = bytearray(64)
    data[:8] = b'Entis\x1a\0\0'
    raw = description.encode('ascii')
    data[16:16+len(raw)] = raw
    if body_size is not None:
        struct.pack_into('<Q', data, 56, body_size)
    return bytes(data)


def section(tag, body):
    return tag.encode('ascii').ljust(8, b' ') + struct.pack('<Q', len(body)) + body


def identity_decoder(codec, data, expected_size):
    if codec != 'erisa_nemesis' or len(data) != expected_size:
        raise ValueError('invalid synthetic codec request')
    return data


def number(value):
    return b'\x04' + struct.pack('<Q', value) + b'\xff' * 8


def string(value):
    return b'\x06' + text(value) + b'\0' * 4 + b'\xff' * 4


def obj(name, fields):
    return b'\0' + text(name) + u32(len(fields)) + b''.join(fields) + b'\xff' + b'\0' * 8


def frame(name, base, names):
    return u32(0) + text(name) + u32(base) + u32(len(names)) + u32(0) + u32(len(names)) + b''.join(text(item) for item in names)


def fixture(*, version=20140121, argument='control:caption', caller='synthetic_scope', bridge_base=0,
            extra_callee=False, return_trailer=b'\xff' * 8, return_tag=4, returned=77,
            title_class='SaveDataTitle'):
    title = obj(title_class, [number(version), obj('Time', [number(0)] * 7),
        string('chapter'), string('caption'), number(0), number(0)])
    returned_value = bytes([return_tag]) + struct.pack('<Q', returned) + return_trailer
    reference = b'\x01' + b'\0' * 24 + b'\xff'
    values = [returned_value, reference, string('speaker'), string(argument)]
    frames = [frame(caller, 0, []), frame('', bridge_base, []),
              frame('synthetic_handler', 1, ['this', 'speaker', 'message'])]
    if extra_callee:
        frames.append(frame('synthetic_handler', 1, ['this', 'speaker', 'message']))
    stack = u32(len(values)) + b''.join(values) + u32(len(frames)) + b''.join(frames)
    body = section('global', b'') + section('stack', stack)
    inner = header('Cotopha Script Context', len(body)) + body
    data = header('cotomi context file') + section('title', title) + section('context', u32(len(inner)) + inner)
    row = dict(source='synthetic_source', position=5, text='caption', speaker='speaker',
        _native=[dict(kind='call_return', scope='synthetic_scope', value=returned)])
    return data, row


def script_fixture():
    push = b'\x02\x00\x06\0\0\0\x80'
    sequence = push + u32(0) + b'\x02\x00\x02' + b'\x02\x00\x04' + u32(3)
    sequence += b'\x11' + u32(2) + b'\0\x04' + u32(128)
    pool = u32(1) + text('caption') + u32(1) + u32(len(push))
    funcs = u32(2)
    for name,address in [('module@synthetic_scope',0),('synthetic_handler',128)]:
        funcs += struct.pack('<IIII',0,address,0xffffffff,0) + text(name)
    return (header('Cotopha Image file') + section('image',sequence.ljust(256,b'\0'))
        + section('conststr',pool) + section('funcinfo',funcs)), len(sequence)


class EntisAdapterTests(unittest.TestCase):
    def extract(self, data, rows):
        return ADAPTER['save_frame_adapter'](data, {'dialogue': rows}, decode=identity_decoder,
            title_profile=TITLE, message_callees=SAVE_PROFILE['message_callees'], text_argument=2)

    def test_native_script_projection_is_independently_attested(self):
        raw, returned = script_fixture()
        output = ADAPTER['extract_script'](raw, {}, SCRIPT_PROFILE)
        rows = HOST['EVIDENCE']['script_rows'](raw, output, source='synthetic_source')
        nodes = HOST['EVIDENCE']['ADAPTERS']['attest_native_nodes'](raw, rows,
            span=HOST['EVIDENCE']['span'], integer=HOST['EVIDENCE']['integer'],
            view=HOST['EVIDENCE']['view'], profiles=output['native_profiles'])
        self.assertEqual(nodes[0][0]['scope'], 'synthetic_scope')
        self.assertEqual(nodes[0][0]['value'], returned)

    def test_title_and_frame_proofs_need_no_global_string_scan(self):
        data, row = fixture()
        output = self.extract(data, [row])
        self.assertEqual(output['line'], 5)
        self.assertEqual(HOST['EVIDENCE']['span'](data, output['evidence']['anchor']), 'caption')
        proof = output['evidence']['node_link']['scope']
        state = ADAPTER['saved_context'](data, identity_decoder)
        self.assertEqual(state['data'][proof['offset']:proof['offset']+proof['length']].decode('utf-16-le'),
                         'synthetic_scope')

    def test_caption_cannot_disambiguate_duplicate_native_coordinates(self):
        data, row = fixture()
        duplicate = dict(copy.deepcopy(row), position=6, text='different caption')
        with self.assertRaisesRegex(ValueError, 'matches=2'):
            self.extract(data, [row, duplicate])

    def test_unknown_version_wrong_argument_and_ambiguous_callee_reject(self):
        for settings in (dict(version=7), dict(argument='other message'), dict(extra_callee=True)):
            with self.subTest(settings=settings):
                data, row = fixture(**settings)
                with self.assertRaises(ValueError):
                    self.extract(data, [row])

    def test_return_slot_requires_integer_type_and_valid_trailer(self):
        for settings in (dict(return_trailer=b'\0' * 8), dict(return_tag=5)):
            with self.subTest(settings=settings):
                data, row = fixture(**settings)
                with self.assertRaises(ValueError):
                    self.extract(data, [row])

    def test_save_scope_is_never_normalised_to_fit_metadata(self):
        data, row = fixture(caller='prefix@synthetic_scope')
        with self.assertRaisesRegex(ValueError, 'matches=0'):
            self.extract(data, [row])

    def test_bridge_cannot_move_to_a_different_value_base(self):
        data, row = fixture(bridge_base=1)
        with self.assertRaisesRegex(ValueError, 'caller-frame transition'):
            self.extract(data, [row])

    def test_unknown_array_or_reference_prefix_fails_closed(self):
        for raw in (u32(1)+b'\x02'+b'\0'*24,u32(1)+b'\x01'+b'\0'*25):
            with self.assertRaises(ValueError):
                ADAPTER['stack_prefix'](raw, dict(offset=0), len(raw), 1)

    def test_program_is_self_contained_and_rejects_sample_answer_fields(self):
        for kind,profile in (('script',SCRIPT_PROFILE),('save',SAVE_PROFILE)):
            source = ADAPTER['program'](kind,profile)
            HOST['validate_program'](source)
            self.assertLess(len(source.encode()),128000)
            for forbidden in ('adapter_library','counted_string_candidates','stack_integer_candidates','pathlib','runpy'):
                self.assertNotIn(forbidden,source)
        with self.assertRaises(ValueError):
            ADAPTER['program']('save',dict(SAVE_PROFILE,source='sample source'))

    def test_inspection_is_compact_filterable_and_uses_real_call_bytes(self):
        raw,returned = script_fixture()
        output = ADAPTER['inspect_script'](raw,{},dict(message_callees=['synthetic_handler']))
        self.assertLess(len(json.dumps(output).encode()),40000)
        self.assertEqual(len(output['functions']),1)
        sample = output['call_abi_candidates'][0]['samples'][0]
        self.assertEqual(sample['scalar_arguments'],1)
        self.assertEqual(sample['call_arity'],2)
        self.assertEqual(sample['return_position'],returned)
        output = ADAPTER['inspect_script'](raw,{},dict(inspection=dict(contains='module',limit=1)))
        self.assertEqual(output['functions'][0]['name'],'module@synthetic_scope')
        with self.assertRaises(ValueError):
            ADAPTER['inspect_script'](raw,{},dict(inspection=dict(limit=100)))

    def test_save_inspection_emits_preflight_proofs_without_dialogue(self):
        data,unused = fixture()
        globals_map = ADAPTER['inspect_save'].__globals__
        old_decoder = globals_map['codec_decode']
        globals_map['codec_decode'] = identity_decoder
        try:
            output=ADAPTER['inspect_save'](data,{},SAVE_PROFILE)
        finally:
            globals_map['codec_decode']=old_decoder
        self.assertTrue(output['analysis_only'])
        self.assertEqual(output['current_call']['caller'],'synthetic_scope')
        self.assertEqual(output['current_call']['return_value'],77)
        self.assertEqual([item['role'] for item in output['observations']],
                         ['current_caption','native_caller','native_return'])
        self.assertLess(len(json.dumps(output).encode()),40000)

    def inspect_save(self, data, profile):
        globals_map = ADAPTER['inspect_save'].__globals__
        old_decoder = globals_map['codec_decode']
        globals_map['codec_decode'] = identity_decoder
        try:
            return ADAPTER['inspect_save'](data, {}, profile)
        finally:
            globals_map['codec_decode'] = old_decoder

    def test_known_title_inspection_does_not_require_callee_discovery(self):
        data,unused = fixture()
        default = self.inspect_save(data, {})
        explicit = self.inspect_save(data, dict(title_profile=TITLE))
        self.assertEqual(default, explicit)
        self.assertEqual(len(default['frames']), 3)
        self.assertEqual(default['title']['caption'], 'caption')
        self.assertEqual([item['role'] for item in default['observations']], ['current_caption'])
        self.assertNotIn('current_call', default)

    def test_inspection_rejects_matching_unknown_class_or_version_profiles(self):
        variants = [(dict(version=7), dict(TITLE,version=7)),
                    (dict(title_class='UnknownTitle'),dict(TITLE,**{'class':'UnknownTitle'}))]
        for fixture_profile,title_profile in variants:
            with self.subTest(title_profile=title_profile):
                data,unused = fixture(**fixture_profile)
                with self.assertRaisesRegex(ValueError, 'unsupported current-title class/version/API profile'):
                    self.inspect_save(data, dict(title_profile=title_profile))
                with self.assertRaisesRegex(ValueError, 'unsupported current-title class/version/API profile'):
                    ADAPTER['program']('save',dict(SAVE_PROFILE,title_profile=title_profile))

    def test_inspection_rejects_unknown_native_title_under_default_profile(self):
        for fixture_profile in (dict(version=7),dict(title_class='UnknownTitle')):
            with self.subTest(fixture_profile=fixture_profile):
                data,unused=fixture(**fixture_profile)
                with self.assertRaises(ValueError):
                    self.inspect_save(data,{})

    def test_inspection_and_generation_share_strict_title_profile_types(self):
        for title_profile in (dict(TITLE,version=20140121.0),dict(TITLE,caption_field=3.0),
                              dict(TITLE,caption_field=True),dict(TITLE,extra='unknown'),None):
            with self.subTest(title_profile=title_profile):
                data,unused=fixture()
                with self.assertRaisesRegex(ValueError,'unsupported current-title class/version/API profile'):
                    self.inspect_save(data,dict(title_profile=title_profile))
                with self.assertRaisesRegex(ValueError,'unsupported current-title class/version/API profile'):
                    ADAPTER['program']('save',dict(SAVE_PROFILE,title_profile=title_profile))


if __name__=='__main__':
    unittest.main()
