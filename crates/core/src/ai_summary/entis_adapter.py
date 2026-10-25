"""Reusable Entis/Cotopha byte adapter for explicitly audited variant profiles.

The supported title and frame serialization variant is deliberately bounded;
unknown versions, object-prefix layouts and ambiguous native nodes fail closed.
Application display callees and calling ABI require an explicit audited profile.
All returned evidence is replayed by a separate verifier. Generated parser
snapshots retain their own source identity and never import this library.
See docs/ai-adapters.md for integration and the supported evidence boundary.
"""
import bisect
import hashlib
import json
import struct
MAX_BYTES = 32 * 1024 * 1024
MAX_RECORDS = 500000
PUSH_STRING = b'\x02\x00\x06\x00\x00\x00\x80'
NEW_ARRAY = b'\x02\x00\x02'
ADD = b'\x0c\x00'
PUSH_INTEGER = b'\x02\x00\x04'

class UnsupportedVariant(ValueError):
    pass

def uint(data, offset, width=4):
    if type(offset) is not int or offset < 0 or width > len(data) - offset:
        raise ValueError('native integer exceeds its input boundary')
    return int.from_bytes(data[offset:offset + width], 'little')

def wide(data, offset, end):
    units = uint(data, offset)
    if units > MAX_RECORDS or 2 * units > end - offset - 4:
        raise ValueError('native string exceeds its complete record boundary')
    begin, after = (offset + 4, offset + 4 + units * 2)
    return (data[begin:after].decode('utf-16-le'), after, {'offset': begin, 'length': units * 2, 'encoding': 'utf-16-le'})

def chunks(data, base=0):
    if len(data) > MAX_BYTES or data[base:base + 8] != b'Entis\x1a\x00\x00':
        raise UnsupportedVariant('unsupported container size or magic')
    if base + 64 > len(data):
        raise ValueError('truncated native container header')
    cursor, sections = (base + 64, {})
    while cursor < len(data):
        if cursor + 16 > len(data):
            raise ValueError('truncated section header')
        tag = data[cursor:cursor + 8]
        if not all((32 <= value < 127 for value in tag)):
            raise UnsupportedVariant('unsupported section tag encoding')
        name, size = (tag.decode('ascii').strip(), uint(data, cursor + 8, 8))
        body = cursor + 16
        if name in sections or not name or size > len(data) - body:
            raise ValueError('duplicate section or invalid section length')
        sections[name] = dict(header=cursor, offset=body, length=size, tag_hex=tag.hex())
        cursor = body + size
    return sections

def saved_context(data, decode):
    """Decode the audited length-prefixed nested-container variant only."""
    base = 0
    if data[:2] == b'BM':
        if len(data) < 54:
            raise ValueError('truncated BMP wrapper')
        base = uint(data, 2)
        if not 54 <= base <= len(data) - 64:
            raise UnsupportedVariant('BMP size does not bound an appended native container')
    sections = chunks(data, base)
    if not data[base + 16:base + 56].startswith(b'cotomi context file'):
        raise UnsupportedVariant('unsupported outer container description')
    encoded = sections.get('context')
    if encoded is None or encoded['length'] <= 4:
        raise UnsupportedVariant('no length-prefixed encoded context section')
    body, size = (encoded['offset'], encoded['length'])
    expected = uint(data, body)
    if not 64 <= expected <= MAX_BYTES:
        raise ValueError('decoded context size exceeds its bound')
    clear = decode('erisa_nemesis', data[body + 4:body + size], expected_size=expected)
    if len(clear) != expected:
        raise ValueError('decoder output disagrees with native size')
    inner = chunks(clear)
    if not clear[16:56].startswith(b'Cotopha Script Context') or uint(clear, 56, 8) + 64 != len(clear) or (not {'global', 'stack'} <= inner.keys()):
        raise UnsupportedVariant('unsupported nested VM-context layout')
    view = [dict(op='erisa_nemesis', offset=body + 4, length=size - 4, size_from=dict(offset=body, width=4, endian='little'))]
    return dict(data=clear, sections=inner, raw_sections=sections, view=view)

def parse_csx(data):
    sections = chunks(data)
    if not data[16:56].startswith(b'Cotopha Image file'):
        raise UnsupportedVariant('unsupported script description')
    if not {'image', 'conststr', 'funcinfo'} <= sections.keys():
        raise UnsupportedVariant('required complete script tables are absent')
    pool = sections['conststr']
    cursor, end = (pool['offset'] + 4, pool['offset'] + pool['length'])
    count = uint(data, pool['offset'])
    if count > MAX_RECORDS:
        raise ValueError('oversized native text table')
    strings = []
    for index in range(count):
        text, cursor, proof = wide(data, cursor, end)
        reference_count = uint(data, cursor)
        cursor += 4
        if reference_count > (end - cursor) // 4:
            raise ValueError('native reference vector exceeds text-table boundary')
        refs = [uint(data, cursor + 4 * item) for item in range(reference_count)]
        cursor += reference_count * 4
        strings.append(dict(index=index, text=text, proof=proof, refs=refs))
    if cursor != end:
        raise ValueError('complete text table does not end at its section boundary')
    table = sections['funcinfo']
    cursor, end = (table['offset'] + 4, table['offset'] + table['length'])
    count = uint(data, table['offset'])
    if not 0 < count <= MAX_RECORDS:
        raise ValueError('invalid native function count')
    functions = []
    for index in range(count):
        if cursor + 20 > end:
            raise ValueError('truncated native function header')
        flags, address, length, reserved = struct.unpack_from('<IIII', data, cursor)
        name, cursor, proof = wide(data, cursor + 16, end)
        if reserved:
            raise UnsupportedVariant('function records with reserved payload are not supported')
        functions.append(dict(index=index, name=name, address=address, declared_length=length, proof=proof, flags=flags))
    if cursor != end:
        raise ValueError('complete function table does not end at its section boundary')
    image = sections['image']
    if any((function['address'] >= image['length'] for function in functions)):
        raise UnsupportedVariant('function address is outside the native code region')
    addresses = [function['address'] for function in functions]
    if len(set(addresses)) != len(addresses):
        raise UnsupportedVariant('aliased function starts need an audited scope policy')
    return dict(data=data, sections=sections, strings=strings, functions=functions, code=data[image['offset']:image['offset'] + image['length']])

def _call_tail(code, start, strings, *, scalar_arguments, call_arity):
    cursor = start
    if code[cursor:cursor + len(NEW_ARRAY)] != NEW_ARRAY:
        return None
    fields = [dict(kind='bytes', hex=NEW_ARRAY.hex())]
    cursor += len(NEW_ARRAY)
    voices = 0
    while code[cursor:cursor + len(PUSH_STRING)] == PUSH_STRING:
        operand = cursor + len(PUSH_STRING)
        if uint(code, operand) >= len(strings) or code[operand + 4:operand + 6] != ADD or voices >= 16:
            return None
        fields.extend([dict(kind='bytes', hex=PUSH_STRING.hex()), dict(kind='table_ref', name='array_item_' + str(voices), width=4, endian='little', table='conststr'), dict(kind='bytes', hex=ADD.hex())])
        cursor = operand + 6
        voices += 1
    for index in range(scalar_arguments):
        if code[cursor:cursor + len(PUSH_INTEGER)] != PUSH_INTEGER:
            return None
        fields.extend([dict(kind='bytes', hex=PUSH_INTEGER.hex()), dict(kind='uint', name='argument_' + str(index), width=4, endian='little')])
        cursor += len(PUSH_INTEGER) + 4
    prefix = b'\x11' + call_arity.to_bytes(4, 'little') + b'\x00\x04'
    if code[cursor:cursor + len(prefix)] != prefix or cursor + len(prefix) + 4 > len(code):
        return None
    target = uint(code, cursor + len(prefix))
    after = cursor + len(prefix) + 4
    fields.append(dict(kind='bytes', hex=code[cursor:after].hex()))
    return dict(fields=fields, target=target, end=after)

def script_candidate_adapter(data, *, message_callees, scalar_arguments, call_arity, speaker_argument=False, scope_transform=None):
    """Emit current host schemas after the caller audits the application ABI.

    Callee names/argument meanings are explicit input, never discovered from
    readable dialogue or hardcoded application-specific symbols in this module.
    This is a restricted instruction profile, not a complete disassembler.
    """
    if not isinstance(message_callees, (set, frozenset)) or not message_callees or any((not isinstance(name, str) or not name for name in message_callees)) or (type(scalar_arguments) is not int) or (not 0 <= scalar_arguments <= 16) or (type(call_arity) is not int) or (not 1 <= call_arity <= 32):
        raise ValueError('an audited bounded calling-ABI profile is required')
    module = parse_csx(data)
    code, strings, functions = (module['code'], module['strings'], module['functions'])
    image, funcs = (module['sections']['image'], module['sections']['funcinfo'])
    targets = {function['address']: function['name'] for function in functions}
    ordered = sorted(functions, key=lambda function: function['address'])
    starts = [function['address'] for function in ordered]
    function_table = dict(offset=funcs['offset'], length=funcs['length'], count_width=4, endian='little', fields=[dict(kind='uint', name='flags', width=4), dict(kind='uint', name='address', width=4), dict(kind='uint', name='declared_length', width=4), dict(kind='uint', name='reserved_length', width=4), dict(kind='text', name='scope', length_width=4, unit=2, encoding='utf-16-le')])
    profiles, rows, seen = ({}, [], set())
    for string in strings:
        if not string['text']:
            continue
        for operand in string['refs']:
            instruction = operand - len(PUSH_STRING)
            if instruction < 0 or operand + 4 > len(code) or code[instruction:operand] != PUSH_STRING or (uint(code, operand) != string['index']):
                continue
            tail = _call_tail(code, operand + 4, strings, scalar_arguments=scalar_arguments, call_arity=call_arity)
            if tail is None or targets.get(tail['target']) not in message_callees:
                continue
            owner_index = bisect.bisect_right(starts, instruction) - 1
            if owner_index < 0:
                raise ValueError('message instruction precedes its native function table')
            owner = ordered[owner_index]
            end = starts[owner_index + 1] if owner_index + 1 < len(starts) else len(code)
            if tail['end'] > end:
                raise ValueError('message sequence crosses its native function boundary')
            fields = [dict(kind='bytes', hex=PUSH_STRING.hex()), dict(kind='table_ref', name='text_index', width=4, endian='little', table='conststr')] + tail['fields']
            profile = dict(kind='call_return', scope_range=dict(table=function_table, start_field='address', scope_field='scope', base=image['offset']), sequence=dict(fields=fields), text_operand='text_index', code_region=dict(offset=image['header'], tag_hex=image['tag_hex'], length_width=8, endian='little'))
            if scope_transform is not None:
                profile['scope_range']['scope_transform'] = dict(scope_transform)
            key = hashlib.sha256(json.dumps(profile, sort_keys=True).encode()).hexdigest()
            profiles[key] = profile
            if len(profiles) > 64:
                raise UnsupportedVariant('too many distinct calling-sequence profiles')
            if instruction in seen:
                raise ValueError('duplicate message instruction references')
            seen.add(instruction)
            proof = dict(table='conststr', index=string['index'], reference=dict(base=image['offset'], operand_offset=len(PUSH_STRING), width=4, endian='little'), native=dict(profile=key, scope_index=owner['index']))
            speaker = None
            if speaker_argument:
                previous = instruction - len(PUSH_STRING) - 4
                if previous < 0 or code[previous:previous + len(PUSH_STRING)] != PUSH_STRING:
                    raise UnsupportedVariant('audited speaker argument is not a direct string push')
                speaker_index = uint(code, previous + len(PUSH_STRING))
                if speaker_index >= len(strings):
                    raise ValueError('native speaker reference exceeds complete text table')
                if strings[speaker_index]['text']:
                    speaker = strings[speaker_index]['text']
                    proof['speaker'] = strings[speaker_index]['proof']
            rows.append(dict(position=instruction, text=string['text'], speaker=speaker, evidence=proof))
    rows.sort(key=lambda row: row['position'])
    pool = module['sections']['conststr']
    return dict(rows=rows, native_profiles=profiles, tables={'conststr': dict(offset=pool['offset'], length=pool['length'], count_width=4, endian='little', fields=[dict(kind='text', length_width=4, unit=2, encoding='utf-16-le'), dict(kind='skip', count_width=4, unit=4)])})

def title_value(data, cursor, end, depth=0):
    """Exact recursive title-only grammar for the explicitly versioned profile."""
    if depth > 8 or cursor >= end:
        raise ValueError('invalid title recursion or truncated type tag')
    tag = data[cursor]
    cursor += 1
    if tag == 4:
        if cursor + 16 > end or data[cursor + 8:cursor + 16] != b'\xff' * 8:
            raise ValueError('invalid title Integer trailer')
        return (dict(kind='integer', value=uint(data, cursor, 8)), cursor + 16)
    if tag == 6:
        text, cursor, proof = wide(data, cursor, end)
        if cursor + 8 > end or data[cursor:cursor + 8] != b'\x00' * 4 + b'\xff' * 4:
            raise ValueError('invalid title String trailer')
        return (dict(kind='string', value=text, proof=proof), cursor + 8)
    if tag == 0:
        name, cursor, proof = wide(data, cursor, end)
        count = uint(data, cursor)
        cursor += 4
        if count > 32:
            raise ValueError('oversized title object')
        fields = []
        for unused in range(count):
            value, cursor = title_value(data, cursor, end, depth + 1)
            fields.append(value)
        if cursor + 9 > end or data[cursor] != 255:
            raise ValueError('invalid title object terminator')
        return (dict(kind='object', name=name, fields=fields), cursor + 9)
    raise UnsupportedVariant('unsupported title value type')

def current_title(data, section, profile):
    """A class/version/schema profile must have an independently audited API role."""
    start, end = (section['offset'], section['offset'] + section['length'])
    value, after = title_value(data, start, end)
    if after != end or value.get('kind') != 'object' or value.get('name') != profile.get('class'):
        raise UnsupportedVariant('unsupported complete title object')
    fields = value['fields']
    if len(fields) != 6 or [field['kind'] for field in fields] != ['integer', 'object', 'string', 'string', 'integer', 'integer']:
        raise UnsupportedVariant('unsupported title field schema')
    time = fields[1]
    if time['name'] != 'Time' or len(time['fields']) != 7 or any((field['kind'] != 'integer' for field in time['fields'])) or (fields[0]['value'] != profile.get('version')) or (profile.get('caption_field') != 3):
        raise UnsupportedVariant('unsupported title version or caption policy')
    if not fields[3]['value']:
        raise UnsupportedVariant('current caption is empty')
    return fields[3]

def stack_frame_suffixes(data, section):
    """Complete bounded frame-vector suffix, without inspecting integer values."""
    start, end = (section['offset'], section['offset'] + section['length'])
    slots = uint(data, start)
    if not 1 <= slots <= 65536:
        raise UnsupportedVariant('unsupported native value-stack count')
    found = []
    for offset in range(start + 4, end - 4):
        count = uint(data, offset)
        if not 1 <= count <= min(slots + 1, 256):
            continue
        cursor, frames = (offset + 4, [])
        try:
            for index in range(count):
                flags = uint(data, cursor)
                if flags > 3:
                    raise ValueError('unsupported frame flag')
                name, cursor, proof = wide(data, cursor + 4, end)
                if any((ord(value) < 32 for value in name)):
                    raise ValueError('non-text native frame name')
                base, local_count = (uint(data, cursor), uint(data, cursor + 4))
                names_count = uint(data, cursor + 12)
                cursor += 16
                if base > slots or local_count > slots - base or names_count != local_count or (names_count > 256):
                    raise ValueError('invalid complete native frame record')
                names = []
                for item in range(names_count):
                    text, cursor, unused = wide(data, cursor, end)
                    if not text or any((ord(character) < 32 for character in text)):
                        raise ValueError('invalid native frame local name')
                    names.append(text)
                frames.append(dict(name=name, base=base, local_count=local_count, names=names, proof=proof, flags=flags))
            if cursor == end and all((a['base'] <= b['base'] for a, b in zip(frames, frames[1:]))):
                found.append(dict(offset=offset, frames=frames, slots=slots))
        except (ValueError, UnicodeError, IndexError):
            pass
    if len(found) != 1:
        raise UnsupportedVariant('native frame suffix is absent or ambiguous')
    return found[0]

def stack_prefix(data, section, frame_start, count):
    """Strict audited primitive prefix; native Object payloads remain unsupported.

    Reference and empty-array encodings are restricted to the observed native
    serialization profile. Unknown types and layouts fail before a slot is used.
    """
    cursor, result = (section['offset'] + 4, [])
    for index in range(count):
        if cursor >= frame_start:
            raise ValueError('native values overlap frame metadata')
        tag, start = (data[cursor], cursor)
        cursor += 1
        if tag == 4:
            if cursor + 16 > frame_start or data[cursor + 8:cursor + 16] != b'\xff' * 8:
                raise UnsupportedVariant('unsupported stack Integer encoding')
            result.append(dict(kind='integer', value=uint(data, cursor, 8), proof=dict(offset=cursor, width=8, endian='little')))
            cursor += 16
        elif tag == 6:
            text, cursor, proof = wide(data, cursor, frame_start)
            if data[cursor:cursor + 8] != b'\x00' * 4 + b'\xff' * 4:
                raise UnsupportedVariant('unsupported stack String encoding')
            result.append(dict(kind='string', value=text, proof=proof))
            cursor += 8
        elif tag == 1:
            if cursor + 25 > frame_start or data[cursor + 24] != 255:
                raise UnsupportedVariant('unsupported stack Reference encoding')
            result.append(dict(kind='reference'))
            cursor += 25
        elif tag == 2:
            tail = b'\x00' * 4 + b'\x06' + b'\x00' * 8 + b'\xff' * 4 + b'\x00\x00\x00\x80'
            if data[cursor:cursor + len(tail)] != tail:
                raise UnsupportedVariant('unsupported stack Array encoding')
            cursor += len(tail)
            result.append(dict(kind='array'))
        else:
            raise UnsupportedVariant('unsupported value before the requested native slot')
        if cursor > frame_start:
            raise ValueError('native value exceeds its vector boundary')
    return result

def save_frame_adapter(data, metadata, *, decode, title_profile, message_callees, text_argument=2):
    """Variant-specific current call slot; independent host replays every proof.

    The title API and frame base-1 convention require the audited profile. The
    complete global object graph is intentionally not claimed to be decoded.
    """
    state = saved_context(data, decode)
    title_section = state['raw_sections'].get('title')
    if title_section is None:
        raise UnsupportedVariant('native title section is absent')
    caption = current_title(data, title_section, title_profile)
    clear, sections = (state['data'], state['sections'])
    frame_vector = stack_frame_suffixes(clear, sections['stack'])
    selected = [(index, frame) for index, frame in enumerate(frame_vector['frames']) if frame['name'] in message_callees]
    if len(selected) != 1:
        raise UnsupportedVariant('display call frame is absent or ambiguous')
    frame_index, frame = selected[0]
    if frame['base'] < 1:
        raise ValueError('display call has no caller return slot')
    if type(text_argument) is not int or not 0 <= text_argument < frame['local_count'] or frame['local_count'] < 3:
        raise UnsupportedVariant('unsupported display-call argument profile')
    values = stack_prefix(clear, sections['stack'], frame_vector['offset'], frame['base'] + text_argument + 1)
    return_slot = values[frame['base'] - 1]
    if return_slot['kind'] != 'integer':
        raise UnsupportedVariant('display call return slot is not an Integer')
    caller_index = frame_index - 1
    while caller_index >= 0 and (not frame_vector['frames'][caller_index]['name']):
        bridge = frame_vector['frames'][caller_index]
        if bridge['local_count'] or bridge['base'] != frame['base'] - 1:
            raise UnsupportedVariant('unsupported anonymous caller-frame transition')
        caller_index -= 1
    if caller_index < 0:
        raise UnsupportedVariant('display caller frame is unnamed')
    caller = frame_vector['frames'][caller_index]
    if caller['local_count'] or caller['base'] != frame['base'] - 1:
        raise UnsupportedVariant('unsupported named caller-frame transition')
    argument = values[frame['base'] + text_argument]
    if argument['kind'] != 'string' or not argument['value'].endswith(caption['value']):
        raise UnsupportedVariant('current-caption API disagrees with the display-call text argument')
    rows = metadata.get('dialogue', [])
    if isinstance(rows, dict):
        rows = rows.get('rows', [])
    matches = []
    for row in rows:
        for native in row.get('_native', []):
            if native.get('kind') != 'call_return' or native.get('value') != return_slot['value']:
                continue
            if native.get('scope') == caller['name']:
                matches.append(row)
    if len(matches) != 1:
        raise UnsupportedVariant('active native scope+return matches=' + str(len(matches)) + '; inspect the effective resource archive/script version; do not alter save coordinates to fit')
    row = matches[0]
    if row['text'] != caption['value']:
        raise UnsupportedVariant('active native node disagrees with the current-title API field')
    return dict(source=row['source'], line=row['position'], scene='', anchors=[caption['value']], variables={}, warnings=[], evidence=dict(anchor=caption['proof'], node_link=dict(kind='call_return', scope=dict(caller['proof'], view=state['view']), position=dict(return_slot['proof'], view=state['view']))))

def inspect_call_abis(module, names):
    """At most three actual bounded direct-call sequences for each chosen callee."""
    code, strings = (module['code'], module['strings'])
    result = []
    for function in module['functions']:
        if function['name'] not in names:
            continue
        marker = b'\x00\x04' + function['address'].to_bytes(4, 'little')
        cursor, samples, examined = (0, [], 0)
        while len(samples) < 3 and examined < 64:
            cursor = code.find(marker, cursor)
            if cursor < 0:
                break
            examined += 1
            call = cursor - 5
            cursor += len(marker)
            if call < 0 or code[call] != 17:
                continue
            arity = uint(code, call + 1)
            if not 1 <= arity <= 32:
                continue
            before, scalars = (call, 0)
            while scalars < 16 and before >= 7 and (code[before - 7:before - 4] == PUSH_INTEGER):
                before -= 7
                scalars += 1
            array_end, voice_count = (before, 0)
            while voice_count < 16 and before >= 13 and (code[before - 13:before - 6] == PUSH_STRING) and (code[before - 2:before] == ADD):
                before -= 13
                voice_count += 1
            if before < 14 or code[before - 3:before] != NEW_ARRAY:
                continue
            instruction = before - 14
            operand = instruction + len(PUSH_STRING)
            if code[instruction:operand] != PUSH_STRING:
                continue
            index = uint(code, operand)
            if index >= len(strings) or operand not in strings[index]['refs']:
                continue
            tail = _call_tail(code, before - 3, strings, scalar_arguments=scalars, call_arity=arity)
            if tail is None or tail['target'] != function['address'] or tail['end'] != call + 11:
                continue
            speaker = instruction >= 11 and code[instruction - 11:instruction - 4] == PUSH_STRING
            samples.append(dict(scalar_arguments=scalars, call_arity=arity, speaker_argument=speaker, voice_literals=voice_count, instruction_position=instruction, text_operand_position=operand, text_index=index, call_position=call, return_position=tail['end']))
        result.append(dict(callee=function['name'], callee_address=function['address'], callee_name_proof=function['proof'], samples=samples, note='Actual byte sequences; audit display semantics before selecting this ABI.'))
    return result


import ast
import pathlib
import runpy

class SandboxSource(ast.NodeTransformer):

    def visit_Name(self, node):
        if node.id == 'UnsupportedVariant':
            return ast.copy_location(ast.Name(id='ValueError', ctx=node.ctx), node)
        return node

    def visit_Attribute(self, node):
        node = self.generic_visit(node)
        if isinstance(node.value, ast.Name) and node.value.id == 'bisect' and (node.attr == 'bisect_right'):
            return ast.copy_location(ast.Name(id='bisect_right', ctx=ast.Load()), node)
        return node

    def visit_Compare(self, node):
        node = self.generic_visit(node)
        if isinstance(node.left, ast.Call) and isinstance(node.left.func, ast.Name) and (node.left.func.id == 'type') and (len(node.left.args) == 1) and (len(node.ops) == 1) and (len(node.comparators) == 1) and isinstance(node.comparators[0], ast.Name) and (node.comparators[0].id == 'int'):
            value = ast.unparse(node.left.args[0])
            if isinstance(node.ops[0], ast.IsNot):
                expression = 'not isinstance(' + value + ', int) or isinstance(' + value + ', bool)'
            elif isinstance(node.ops[0], ast.Is):
                expression = 'isinstance(' + value + ', int) and not isinstance(' + value + ', bool)'
            else:
                raise ValueError('unexpected type comparison in bundled source')
            return ast.copy_location(ast.parse(expression, mode='eval').body, node)
        return node

def bisect_right(values, value):
    lo, hi = (0, len(values))
    while lo < hi:
        mid = (lo + hi) // 2
        if value < values[mid]:
            hi = mid
        else:
            lo = mid + 1
    return lo

def checked_title_profile(profile):
    """Only the audited title API may assign a current-caption field role."""
    supported = {'class': 'SaveDataTitle', 'version': 20140121, 'caption_field': 3}
    if (not isinstance(profile, dict) or profile != supported
            or type(profile.get('version')) is not int
            or type(profile.get('caption_field')) is not int):
        raise ValueError('unsupported current-title class/version/API profile')
    return dict(supported)


def checked_profile(kind, profile):
    if not isinstance(profile, dict):
        raise ValueError('profile must be ordinary JSON data')
    allowed = {'message_callees', 'scalar_arguments', 'call_arity', 'speaker_argument', 'scope_transform'} if kind == 'script' else {'message_callees', 'title_profile', 'text_argument', 'vm_profile'}
    if set(profile) - allowed:
        raise ValueError('unknown adapter profile field')
    callees = profile.get('message_callees')
    if not isinstance(callees, list) or not 1 <= len(callees) <= 16 or any((not isinstance(item, str) or not item or len(item) > 256 for item in callees)):
        raise ValueError('explicit audited message_callees are required')
    if kind == 'script':
        if profile.get('scope_transform') not in (None, dict(kind='last_component', separator='@')):
            raise ValueError('unsupported native scope projection for this adapter profile')
        if type(profile.get('scalar_arguments')) is not int or not 0 <= profile['scalar_arguments'] <= 16 or type(profile.get('call_arity')) is not int or (not 1 <= profile['call_arity'] <= 32) or (not isinstance(profile.get('speaker_argument', False), bool)):
            raise ValueError('bounded explicit native call ABI is required')
    elif kind == 'save':
        if profile.get('vm_profile') != 'cotopha-frame-vector-v1':
            raise ValueError('the audited native frame serialization profile must be explicit')
        checked_title_profile(profile.get('title_profile'))
        if type(profile.get('text_argument')) is not int or not 0 <= profile['text_argument'] <= 16:
            raise ValueError('explicit native display text argument is required')
    else:
        raise ValueError('adapter kind must be script or save')
    return json.loads(json.dumps(profile, ensure_ascii=False, allow_nan=False))

def assemble(template, kind, profile):
    profile = checked_profile(kind, profile)
    tree = ast.parse(template)
    functions = {node.name: node for node in tree.body if isinstance(node, ast.FunctionDef)}
    functions['bisect_right'] = ast.parse(BISECT_SOURCE).body[0]
    root = 'script_candidate_adapter' if kind == 'script' else 'save_frame_adapter'
    needed, pending = (set(), [root])
    while pending:
        name = pending.pop()
        if name in needed:
            continue
        needed.add(name)
        node = functions[name]
        pending.extend((item.id for item in ast.walk(node) if isinstance(item, ast.Name) and item.id in functions and (item.id not in needed)))
        if any((isinstance(item, ast.Attribute) and item.attr == 'bisect_right' for item in ast.walk(node))):
            pending.append('bisect_right')
    result = [ast.Import(names=[ast.alias(name=name)]) for name in ('hashlib', 'json', 'struct')]
    if kind == 'save':
        result.append(ast.Import(names=[ast.alias(name='galgear_codecs')]))
    constant_names = {'MAX_BYTES', 'MAX_RECORDS', 'PUSH_STRING', 'NEW_ARRAY', 'ADD', 'PUSH_INTEGER'}
    result.extend((node for node in tree.body if isinstance(node, ast.Assign) and any((isinstance(target, ast.Name) and target.id in constant_names for target in node.targets))))
    result.extend((functions[name] for name in functions if name in needed))
    result = SandboxSource().visit(ast.Module(body=result, type_ignores=[]))
    result = ast.fix_missing_locations(result)
    source = ast.unparse(result) + '\n\nPROFILE = json.loads(' + repr(json.dumps(profile, ensure_ascii=False)) + ')\n\n'
    if kind == 'script':
        source += "def extract(data, metadata):\n    return script_candidate_adapter(data, message_callees=set(PROFILE['message_callees']), scalar_arguments=PROFILE['scalar_arguments'], call_arity=PROFILE['call_arity'], speaker_argument=PROFILE.get('speaker_argument', False), scope_transform=PROFILE.get('scope_transform'))\n"
    else:
        source += "def extract(data, metadata):\n    return save_frame_adapter(data, metadata, decode=galgear_codecs.decode, title_profile=PROFILE['title_profile'], message_callees=PROFILE['message_callees'], text_argument=PROFILE['text_argument'])\n"
    return source
BISECT_SOURCE = 'def bisect_right(values, value):\n    lo, hi = 0, len(values)\n    while lo < hi:\n        mid = (lo + hi) // 2\n        if value < values[mid]:\n            hi = mid\n        else:\n            lo = mid + 1\n    return lo\n'

def program(kind, profile):
    return assemble(pathlib.Path(__file__).read_text(encoding='utf-8'), kind, profile)


def codec_decode(codec, data, expected_size):
    fixed = pathlib.Path(__file__).with_name('bytecodecs.py')
    return runpy.run_path(str(fixed))['decode'](codec, data, expected_size=expected_size)


def describe():
    return dict(id='entis_cotopha', version=1,
        scope='One explicitly audited Cotopha title/frame serialization variant; unknown layouts fail closed.',
        profile=dict(script=dict(message_callees=['<audited native callee>'], scalar_arguments='<integer 0..16>',
            call_arity='<integer 1..32>', speaker_argument='<boolean>',
            scope_transform=dict(kind='last_component', separator='@')),
            save=dict(message_callees=['<same audited native callee>'], text_argument='<native local index>',
                vm_profile='cotopha-frame-vector-v1',
                title_profile=dict(**{'class':'SaveDataTitle'},version=20140121,caption_field=3))),
        guarantees=['Complete conststr and funcinfo table walk', 'Actual native call sequence and return coordinate',
            'Complete current-title object and version check', 'Unique complete frame-vector suffix and typed value prefix',
            'Caller short name and return Integer proofs; caption only crosschecks the uniquely selected native node'],
        inspection=dict(script=dict(profile=dict(inspection=dict(function_names=['<exact names>'],
            contains='<optional substring>',offset=0,limit=12),message_callees=['<optional chosen callee names>']),
            note='Default 12 functions; limit 1..20. Exact message_callees also returns up to three real call ABI summaries per callee.'),
            save=dict(note='Raw caption/version profile and proof are returned first. Supply message_callees and text_argument to project caller/return/text proof without a transcript. Outputs are analysis evidence, not verification receipts.')),
        limitations=['Native object payloads and nonempty array prefixes are unsupported',
            'Frame base-1 caller convention and title API role require the explicitly audited variant',
            'Display-call semantic ABI is caller supplied; this is not a universal Entis engine decoder'])


def inspect_script(data, metadata, profile):
    module = parse_csx(data)
    selection = profile.get('inspection',{})
    if not isinstance(selection,dict) or set(selection)-{'function_names','contains','offset','limit'}:
        raise ValueError('inspection must use function_names/contains/offset/limit only')
    names = selection.get('function_names',profile.get('message_callees',[]))
    contains, offset, limit = selection.get('contains',''), selection.get('offset',0), selection.get('limit',12)
    if (not isinstance(names,list) or len(names)>16 or any(not isinstance(item,str) for item in names)
            or not isinstance(contains,str) or len(contains)>256 or type(offset) is not int or offset<0
            or type(limit) is not int or not 1<=limit<=20):
        raise ValueError('invalid bounded inspection filter')
    callees = [item for item in sorted(module['functions'],key=lambda item:item['address'])
        if (not names or item['name'] in names) and (not contains or contains in item['name'])]
    selected = callees[offset:offset+limit]
    functions = [dict(name=item['name'],address=item['address'],index=item['index'],name_proof=item['proof'])
        for item in selected]
    if any(len(item['name'])>512 for item in functions):
        raise ValueError('native function name exceeds compact inspection limit')
    chosen = profile.get('message_callees',[])
    if not isinstance(chosen,list) or len(chosen)>4 or any(not isinstance(item,str) for item in chosen):
        raise ValueError('inspection permits at most four chosen message_callees')
    return dict(sections=module['sections'], string_count=len(module['strings']),
        function_count=len(module['functions']),matched_function_count=len(callees),functions=functions,
        next_offset=offset+len(selected) if offset+len(selected)<len(callees) else None,
        call_abi_candidates=inspect_call_abis(module,chosen),
        requires='Choose a semantic message callee from native frame names and audit the supplied actual call summaries.')


def extract_script(data, metadata, profile):
    profile = checked_profile('script', profile)
    return script_candidate_adapter(data, message_callees=set(profile['message_callees']),
        scalar_arguments=profile['scalar_arguments'], call_arity=profile['call_arity'],
        speaker_argument=profile.get('speaker_argument',False), scope_transform=profile.get('scope_transform'))


def inspect_save(data, metadata, profile):
    # Inspection may precede callee/argument discovery, but it must not let a
    # caller label an unsupported title class/version as the current caption.
    title_profile=checked_title_profile(profile.get('title_profile',
        {'class':'SaveDataTitle','version':20140121,'caption_field':3}))
    state = saved_context(data, codec_decode)
    vector = stack_frame_suffixes(state['data'], state['sections']['stack'])
    caption=current_title(data,state['raw_sections']['title'],title_profile)
    result=dict(analysis_only=True,sections=state['sections'],value_slots=vector['slots'],
        title=dict(caption=caption['value'],proof=caption['proof'],profile=title_profile),
        observations=[dict(role='current_caption',proof=caption['proof'],value=caption['value'])],
        frames=[dict(name=item['name'],base=item['base'],locals=item['local_count'],variables=item['names'])
            for item in vector['frames']],
        requires='An explicit supported title/VM profile and the same audited message callee as the script parser.')
    names=profile.get('message_callees',[])
    if names:
        selected=[(index,item) for index,item in enumerate(vector['frames']) if item['name'] in names]
        if len(selected)!=1:
            raise ValueError('chosen display frame is absent or ambiguous')
        index,callee=selected[0]
        argument=profile.get('text_argument',2)
        if type(argument) is not int or not 0<=argument<callee['local_count'] or callee['base']<1:
            raise ValueError('invalid native text_argument')
        previous=index-1
        while previous>=0 and not vector['frames'][previous]['name']:
            bridge=vector['frames'][previous]
            if bridge['local_count'] or bridge['base']!=callee['base']-1:
                raise ValueError('unsupported anonymous caller-frame transition')
            previous-=1
        if previous<0:
            raise ValueError('no named native caller frame')
        caller=vector['frames'][previous]
        if caller['local_count'] or caller['base']!=callee['base']-1:
            raise ValueError('unsupported named caller-frame transition')
        values=stack_prefix(state['data'],state['sections']['stack'],vector['offset'],callee['base']+argument+1)
        returned=values[callee['base']-1]
        message=values[callee['base']+argument]
        if returned['kind']!='integer' or message['kind']!='string' or not message['value'].endswith(caption['value']):
            raise ValueError('native current-call values disagree with title API')
        scope_proof=dict(caller['proof'],view=state['view'])
        position_proof=dict(returned['proof'],view=state['view'])
        text_proof=dict(message['proof'],view=state['view'])
        result['current_call']=dict(callee=callee['name'],caller=caller['name'],text_argument=argument,
            local_names=callee['names'],return_value=returned['value'],
            node_link=dict(kind='call_return',scope=scope_proof,position=position_proof),
            text_value=message['value'],text_proof=text_proof)
        result['observations'].extend([dict(role='native_caller',proof=scope_proof,value=caller['name']),
            dict(role='native_return',proof=position_proof,value=returned['value'])])
    return result


def extract_save(data, metadata, profile):
    profile = checked_profile('save', profile)
    return save_frame_adapter(data, metadata, decode=codec_decode,
        title_profile=profile['title_profile'], message_callees=profile['message_callees'],
        text_argument=profile['text_argument'])
