"""Replay declarative adapter proofs without executing adapter verification code.

Only the host calls attestation after its normal script-row verification. An
adapter declares a complete native scope table and a bounded byte sequence;
the host derives return addresses and emits canonical ``_native`` records.
No engine names, opcode meanings, game offsets or model-signed receipts live
here. These checks establish the declared byte relationships, not a universal
proof of an unknown virtual machine's execution semantics.
"""
import bisect
import hashlib
import json
import re
import pathlib
import runpy


TYPED = runpy.run_path(str(pathlib.Path(__file__).with_name('typed_evidence.py')))
KAG = runpy.run_path(str(pathlib.Path(__file__).with_name('kag_evidence.py')))


VERSION = 1
MAX_NODES = 500_000
MAX_FIELDS = 256
MAX_SEQUENCE_BYTES = 1024 * 1024
ENCODINGS = {'utf-8', 'utf-16-le', 'utf-16-be', 'cp932', 'gb18030', 'ascii'}
WIDTHS = (1, 2, 3, 4, 8)
CONTRACT = dict(
    version=VERSION,
    script='After normal script verification, evidence.native declares call_return links. '
           'The host derives canonical _native records; adapter-supplied _native is ignored.',
    shared_profiles='Preferred: output.native_profiles={id:shared_spec}; each row uses '
                    'evidence.native={profile:id,scope_index:<native function record index>}. '
                    'shared_spec omits scope, scope_range.index and sequence.offset; the host derives them.',
    shared_spec=dict(kind='call_return', scope_range=dict(
        table=dict(offset='<complete native record-table byte offset>', length='<exact full table bytes>',
                   count_width=4, fields=[dict(kind='uint', name='address', width=4),
                                          dict(kind='text', name='scope', length_width=4, unit=2, encoding='utf-16-le')]),
        start_field='address', scope_field='scope', base='<same base as the verified script reference>'),
        sequence=dict(fields=[dict(kind='bytes', hex='<observed opcode bytes>'),
                              dict(kind='table_ref', name='text_index', width=4, endian='little'),
                              dict(kind='bytes', hex='<remaining observed instruction/call bytes>')]),
        text_operand='text_index',
        code_region=dict(offset='<native section header>', tag_hex='<exact native section tag bytes>',
                         length_width='<native payload length width>', endian='little')),
    full_declaration='The same shared_spec with scope=<exact native scope-name span>, '
                     'scope_range.index=<native function index>, sequence.offset=<base+row.position>.',
    code_region='Required for every native node: derive the instruction base and payload size from '
                'an exact native section tag and its native counted length. The entire input file '
                'or a model-supplied return-address constant cannot replace these code bounds.',
    scope_rules='Walk the complete record table exactly, preserving all entries. Scope name and address '
                'belong to the same record. Its end is the next strictly larger native address, '
                'or the independently bounded code_region payload end for the final scope. '
                'Optional scope_range.scope_transform={kind:"last_component",separator:"::"} projects '
                'the verified full name to its native leaf identifier when saved frames omit namespaces. '
                'No text fallback: projected scope plus return address must still identify exactly one row.',
    sequence_rules='No arbitrary return_address/value constant is accepted. Return address is sequence_end-base. '
                   'The text operand must be at the exact already verified instruction operand address, '
                   'with the same width/endian/table/index; an equal integer elsewhere is insufficient.',
    saved_link=dict(kind='call_return', scope=dict(offset='<saved native scope offset>', length='<bytes>', encoding='<codec>', view=[]),
                    position=dict(offset='<saved native return-address offset>', width=4, endian='little', view=[])),
    limits='Only exact bytes and uint/table_ref sequence fields; bounded complete tables and sequences. '
           'Script declarations use the same verified byte view. Saved link fields may use supported views. '
           'The saved scope and return address must identify exactly one host-attested dialogue row.')
CONTRACT['other_topics'] = 'Typed PSB/TJS record schemas: verification_contract(topic="structured"). Audited KAG restore entries: topic="resume". A recorded tuple alone is not current-state proof.'


def _hash(value):
    try:
        raw = json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()
    except (TypeError, ValueError) as error:
        raise ValueError('Native adapter schema must be JSON data') from error
    return hashlib.sha256(raw).hexdigest()


def _keys(value, allowed, required, label):
    if not isinstance(value, dict) or set(value) - set(allowed) or not set(required) <= value.keys():
        raise ValueError(label + ' has missing or unsupported fields')


def _hex(value, maximum, label):
    if (not isinstance(value, str) or not value or len(value) > maximum * 2
            or len(value) % 2 or not re.fullmatch('[0-9a-fA-F]+', value)):
        raise ValueError(label + ' needs bounded, nonempty exact hex bytes')
    return bytes.fromhex(value)


def _name(value, label):
    if not isinstance(value, str) or not value or len(value.encode()) > 4096 or '\0' in value:
        raise ValueError(label + ' needs bounded nonempty text')
    return value


def _scope_name(value, transform):
    """Replay an explicit identifier projection, never choose a scope by text."""
    _name(value, 'Native scope name')
    if transform is None:
        return value
    _keys(transform, ('kind', 'separator'), ('kind', 'separator'), 'Native scope transform')
    separator = transform['separator']
    if (transform['kind'] != 'last_component' or not isinstance(separator, str)
            or not 1 <= len(separator) <= 8
            or any(not 33 <= ord(char) <= 126 or char.isalnum() for char in separator)):
        raise ValueError('Native scope transform needs last_component and a bounded punctuation separator')
    components = value.split(separator)
    if len(components) > 64 or any(not component for component in components):
        raise ValueError('Native scope transform requires nonempty bounded name components')
    return _name(components[-1], 'Projected native scope name')


def _width(value, label):
    if type(value) is not int or value not in WIDTHS:
        raise ValueError(label + ' needs a supported native integer width')
    return value


def _table(data, schema, *, integer):
    """Walk all records and retain native field spans, including unused rows."""
    _keys(schema, ('offset', 'length', 'count_width', 'endian', 'fields'),
          ('offset', 'length', 'count_width', 'fields'), 'Native scope table')
    offset, length = schema['offset'], schema['length']
    width, endian = _width(schema['count_width'], 'Native table count'), schema.get('endian', 'little')
    if (type(offset) is not int or type(length) is not int or offset < 0
            or length < width or offset > len(data) - length or endian not in ('little', 'big')):
        raise ValueError('Native scope table is outside the script bytes')
    count = integer(data, offset, width, endian)
    fields = schema['fields']
    if not 0 < count <= MAX_NODES or not isinstance(fields, list) or not 1 <= len(fields) <= 32:
        raise ValueError('Native scope table count or fields are invalid')
    names, minimum = set(), 0
    for field in fields:
        if not isinstance(field, dict):
            raise ValueError('Native scope table fields must be objects')
        kind = field.get('kind')
        if kind == 'uint':
            _keys(field, ('kind', 'name', 'width', 'endian'), ('kind', 'name', 'width'), 'Native uint field')
            minimum += _width(field['width'], 'Native uint field')
            if field.get('endian', endian) not in ('little', 'big'):
                raise ValueError('Native uint field has unsupported endian')
        elif kind == 'text':
            _keys(field, ('kind', 'name', 'length_width', 'unit', 'encoding', 'endian', 'terminator_width'),
                  ('kind', 'name', 'length_width', 'encoding'), 'Native text field')
            minimum += _width(field['length_width'], 'Native text count')
            if (type(field.get('unit', 1)) is not int or not 1 <= field.get('unit', 1) <= 4
                    or field['encoding'] not in ENCODINGS or field.get('endian', endian) not in ('little', 'big')
                    or type(field.get('terminator_width', 0)) is not int or field.get('terminator_width', 0) not in (0, 1, 2)):
                raise ValueError('Native text field has unsupported count or encoding')
        elif kind == 'fixed':
            _keys(field, ('kind', 'length'), ('kind', 'length'), 'Native fixed field')
            if type(field['length']) is not int or not 0 < field['length'] <= 4096:
                raise ValueError('Native fixed field has invalid length')
            minimum += field['length']
            continue
        else:
            raise ValueError('Unsupported native scope table field kind')
        name = _name(field['name'], 'Native field name')
        if name in names:
            raise ValueError('Native scope table field names must be unique')
        names.add(name)
    if count * minimum > length - width:
        raise ValueError('Native scope table count exceeds its complete record boundary')
    cursor, end, records = offset + width, offset + length, []
    for _ in range(count):
        values, proofs = {}, {}
        for field in fields:
            kind = field['kind']
            if kind == 'fixed':
                cursor += field['length']
            elif kind == 'uint':
                size, order = field['width'], field.get('endian', endian)
                if cursor > end - size:
                    raise ValueError('Native scope integer exceeds its complete table boundary')
                values[field['name']] = integer(data, cursor, size, order)
                proofs[field['name']] = dict(offset=cursor, width=size, endian=order)
                cursor += size
            else:
                size, order = field['length_width'], field.get('endian', endian)
                if cursor > end - size:
                    raise ValueError('Native scope text count exceeds its complete table boundary')
                byte_length = integer(data, cursor, size, order) * field.get('unit', 1)
                cursor += size
                terminator = field.get('terminator_width', 0)
                if byte_length < terminator or byte_length > end - cursor:
                    raise ValueError('Native scope text exceeds its complete table boundary')
                if terminator and data[cursor + byte_length - terminator:cursor + byte_length] != b'\0' * terminator:
                    raise ValueError('Native scope text lacks its declared terminator')
                text_length = byte_length - terminator
                values[field['name']] = data[cursor:cursor + text_length].decode(field['encoding'])
                proofs[field['name']] = dict(offset=cursor, length=text_length, encoding=field['encoding'])
                cursor += byte_length
            if cursor > end:
                raise ValueError('Native scope record exceeds its complete table boundary')
        records.append(dict(values=values, proofs=proofs))
    if cursor != end:
        raise ValueError('Native scope table walk did not end at its complete boundary')
    return records


def _code_region(data, schema, *, integer):
    _keys(schema, ('offset', 'tag_hex', 'length_width', 'endian'),
          ('offset', 'tag_hex', 'length_width'), 'Native code region')
    offset = schema['offset']
    tag, width = _hex(schema['tag_hex'], 32, 'Native code tag'), _width(schema['length_width'], 'Native code length')
    if type(offset) is not int or not 0 <= offset <= len(data) - len(tag) - width:
        raise ValueError('Native code region header is outside the script bytes')
    if data[offset:offset + len(tag)] != tag:
        raise ValueError('Native code region tag disagrees with its actual bytes')
    size = integer(data, offset + len(tag), width, schema.get('endian', 'little'))
    base = offset + len(tag) + width
    if not 0 < size <= len(data) - base:
        raise ValueError('Native code region length is outside the script bytes')
    return base, size


def _sequence(data, schema, *, integer):
    _keys(schema, ('offset', 'fields'), ('offset', 'fields'), 'Native instruction sequence')
    cursor, fields = schema['offset'], schema['fields']
    if (type(cursor) is not int or not 0 <= cursor < len(data) or not isinstance(fields, list)
            or not 3 <= len(fields) <= MAX_FIELDS or not isinstance(fields[0], dict)
            or fields[0].get('kind') != 'bytes' or not isinstance(fields[-1], dict)
            or fields[-1].get('kind') != 'bytes'):
        raise ValueError('Native sequence needs an actual start and bounded opcode bytes around its operands')
    start, values = cursor, {}
    for field in fields:
        if not isinstance(field, dict):
            raise ValueError('Native instruction fields must be objects')
        kind = field.get('kind')
        if kind == 'bytes':
            _keys(field, ('kind', 'hex'), ('kind', 'hex'), 'Native instruction bytes')
            literal = _hex(field['hex'], 4096, 'Native instruction bytes')
            if cursor > len(data) - len(literal) or data[cursor:cursor + len(literal)] != literal:
                raise ValueError('Native instruction opcode bytes disagree with the script')
            cursor += len(literal)
        elif kind in ('uint', 'table_ref'):
            _keys(field, ('kind', 'name', 'width', 'endian', 'table'),
                  ('kind', 'name', 'width'), 'Native instruction operand')
            name, width = _name(field['name'], 'Native operand name'), _width(field['width'], 'Native operand')
            if name in values:
                raise ValueError('Native instruction operand names must be unique')
            order = field.get('endian', 'little')
            values[name] = dict(value=integer(data, cursor, width, order), offset=cursor,
                                width=width, endian=order, table=field.get('table'))
            cursor += width
        else:
            raise ValueError('Unsupported native instruction sequence field kind')
        if cursor - start > MAX_SEQUENCE_BYTES:
            raise ValueError('Native instruction sequence exceeds its bounded byte budget')
    return cursor, values


def attest_native_nodes(data, verified_rows, *, span, integer, view, profiles=None):
    """Return position -> host records from each verified row's evidence.native.

    ``data`` uses the same byte-coordinate space as ``verified_rows``. The host
    must ignore adapter-supplied ``_native`` fields and install only this result
    in canonical dialogue. The complete script/table is hashed and walked once
    per call; no model-provided current-node result or return-address constant
    is accepted.
    """
    del view  # Script declarations use the already verified shared byte view.
    if profiles is None:
        profiles = {}
    if not isinstance(profiles, dict) or len(profiles) > 64:
        raise ValueError('Native profiles must be a bounded schema object')
    digest, tables, addresses, result, total = hashlib.sha256(data).hexdigest(), {}, {}, {}, 0
    table_hashes, profile_hashes = {}, {}

    def table_identity(schema):
        identity = id(schema)
        if identity not in table_hashes:
            table_hashes[identity] = _hash(schema)
        return table_hashes[identity]

    def records_for(schema):
        table_hash = table_identity(schema)
        if table_hash not in tables:
            tables[table_hash] = _table(data, schema, integer=integer)
        return table_hash, tables[table_hash]

    for row in verified_rows:
        proof = row.get('evidence', {})
        declarations = proof.get('native') if isinstance(proof, dict) else None
        if declarations is None:
            continue
        declarations = declarations if isinstance(declarations, list) else [declarations]
        if not 1 <= len(declarations) <= 32:
            raise ValueError('Native node declarations must be a bounded nonempty list')
        if any(isinstance(item, dict) and item.get('kind') == 'kag_label_resume' for item in declarations):
            if len(declarations) != 1 or declarations[0].get('kind') != 'kag_label_resume':
                raise ValueError('KAG resume label rows require exactly one audited label declaration')
            position = row.get('position')
            if position in result:
                raise ValueError('Duplicate KAG resume label position')
            result[position] = [KAG['attest_label'](data, row, declarations[0])]
            total += 1
            if total > MAX_NODES:
                raise ValueError('Native node attestations exceed the bounded record budget')
            continue
        if any(isinstance(item, dict) and item.get('kind') == 'typed_record' for item in declarations):
            if not all(isinstance(item, dict) and item.get('kind') == 'typed_record' for item in declarations):
                raise ValueError('Typed record declarations cannot mix coordinate kinds in one row')
            position = row.get('position')
            records = [TYPED['attest_record'](data, row, item) for item in declarations]
            if len({record['schema_sha256'] for record in records}) != len(records) or position in result:
                raise ValueError('Duplicate typed native record declaration or position')
            result[position] = records
            total += len(records)
            if total > MAX_NODES:
                raise ValueError('Native node attestations exceed the bounded record budget')
            continue
        binding = proof.get('reference')
        position, index = row.get('position'), proof.get('index')
        if (not isinstance(binding, dict) or binding.get('mode') == 'ordinal'
                or type(position) is not int or position < 0 or type(index) is not int
                or index < 0 or not isinstance(proof.get('table'), str)):
            raise ValueError('Native return links need an independently verified table operand and instruction position')
        base, operand = binding.get('base', 0), binding.get('operand_offset')
        if type(base) is not int or base < 0 or type(operand) is not int or operand < 0:
            raise ValueError('Native return link needs the verified instruction operand address')
        expected_offset = base + position + operand
        expected_width, expected_endian = binding.get('width', 4), binding.get('endian', 'little')
        if integer(data, expected_offset, expected_width, expected_endian) != index:
            raise ValueError('Native link operand disagrees with its independently verified table index')
        records = []
        for native in declarations:
            shared_hash = None
            if isinstance(native, dict) and 'profile' in native:
                _keys(native, ('profile', 'scope_index'), ('profile', 'scope_index'), 'Shared native proof')
                profile_id = _name(native['profile'], 'Native profile ID')
                template = profiles.get(profile_id)
                if not isinstance(template, dict):
                    raise ValueError('Unknown native proof profile')
                if ('scope' in template or not isinstance(template.get('scope_range'), dict)
                        or 'index' in template['scope_range'] or not isinstance(template.get('sequence'), dict)
                        or 'offset' in template['sequence']):
                    raise ValueError('Shared native profile must leave scope, index and sequence offset to the host')
                selected = native['scope_index']
                _, table = records_for(template['scope_range'].get('table'))
                if type(selected) is not int or not 0 <= selected < len(table):
                    raise ValueError('Shared native scope index is outside its complete record table')
                scope_field = template['scope_range'].get('scope_field')
                if not isinstance(scope_field, str) or scope_field not in table[selected]['proofs']:
                    raise ValueError('Shared native scope field is absent from its native record')
                if profile_id not in profile_hashes:
                    profile_hashes[profile_id] = _hash(template)
                shared_hash = profile_hashes[profile_id]
                native = dict(template, scope=table[selected]['proofs'][scope_field],
                    scope_range=dict(template['scope_range'], index=selected),
                    sequence=dict(template['sequence'], offset=base + position))
            _keys(native, ('kind', 'scope', 'scope_range', 'sequence', 'text_operand', 'code_region'),
                  ('kind', 'scope', 'scope_range', 'sequence', 'text_operand'), 'Native node proof')
            if native['kind'] != 'call_return':
                raise ValueError('Unsupported native node kind')
            if 'code_region' not in native:
                raise ValueError('Native return links require an independently bounded code_region')
            scope_proof, bounds = native['scope'], native['scope_range']
            _keys(bounds, ('table', 'index', 'start_field', 'scope_field', 'base', 'scope_transform'),
                  ('table', 'index', 'start_field', 'scope_field', 'base'), 'Native scope range')
            if type(bounds['base']) is not int or bounds['base'] != base:
                raise ValueError('Native scope base must equal the independently verified instruction base')
            table_hash, table = records_for(bounds['table'])
            selected = bounds['index']
            if type(selected) is not int or not 0 <= selected < len(table):
                raise ValueError('Native scope index is outside the complete record table')
            start_field, scope_field = bounds['start_field'], bounds['scope_field']
            if not isinstance(start_field, str) or not isinstance(scope_field, str):
                raise ValueError('Native scope field selectors must name native table fields')
            source_record = table[selected]
            start, scope = source_record['values'].get(start_field), source_record['values'].get(scope_field)
            if type(start) is not int or start < 0 or not isinstance(scope, str):
                raise ValueError('Native scope name and address must belong to the same complete record')
            _name(scope, 'Native scope name')
            native_scope_proof = source_record['proofs'].get(scope_field)
            if (not isinstance(scope_proof, dict) or scope_proof.get('view')
                    or any(scope_proof.get(key) != native_scope_proof.get(key) for key in ('offset', 'length', 'encoding'))
                    or span(data, scope_proof) != scope):
                raise ValueError('Native scope span must be the name field in that same address record')
            scope = _scope_name(scope, bounds.get('scope_transform'))
            address_key = (table_hash, start_field)
            if address_key not in addresses:
                actual = [entry['values'].get(start_field) for entry in table]
                if any(type(value) is not int or value < 0 for value in actual):
                    raise ValueError('Native scope addresses must be native nonnegative scalars')
                addresses[address_key] = sorted(set(actual))
            ordered = addresses[address_key]
            after = bisect.bisect_right(ordered, start)
            code_base, code_size = _code_region(data, native['code_region'], integer=integer)
            if code_base != base:
                raise ValueError('Native code region base disagrees with the verified instruction base')
            if after < len(ordered):
                end = ordered[after]
            else:
                end = code_size
            if not 0 <= start < end <= code_size or not start <= position < end:
                raise ValueError('Native instruction is outside its actual function scope range')
            sequence = native['sequence']
            if not isinstance(sequence, dict) or sequence.get('offset') != base + position:
                raise ValueError('Native sequence must start at the independently verified instruction position')
            sequence_end, operands = _sequence(data, sequence, integer=integer)
            text_operand = operands.get(native['text_operand']) if isinstance(native['text_operand'], str) else None
            if (text_operand is None or text_operand['offset'] != expected_offset
                    or text_operand['value'] != index or text_operand['width'] != expected_width
                    or text_operand['endian'] != expected_endian
                    or text_operand['table'] is not None and text_operand['table'] != proof['table']):
                raise ValueError('Native sequence text operand must be the same verified table reference, not an equal value elsewhere')
            returned = sequence_end - base
            if not position < returned <= end:
                raise ValueError('Native return address must follow this exact sequence inside its function scope')
            schema_hash = (_hash(dict(profile=shared_hash, scope_index=selected, position=position))
                           if shared_hash is not None else _hash(native))
            record = dict(version=VERSION, kind='call_return', scope=scope, value=returned, position=position,
                          script_sha256=digest, schema_sha256=schema_hash)
            if record in records:
                raise ValueError('Duplicate native node attestation')
            records.append(record)
            total += 1
            if total > MAX_NODES:
                raise ValueError('Native node attestations exceed the bounded record budget')
        if position in result:
            raise ValueError('Native node declarations repeat a script position')
        result[position] = records
    # A PSB reference proves a message field exists, never its runtime role.
    # Install this even when a model omits evidence.native entirely.
    for row in verified_rows:
        proof = row.get('evidence', {})
        if (isinstance(proof, dict) and proof.get('kind') == 'typed_field'
                and proof.get('document', {}).get('syntax') == 'psb'):
            selected = TYPED['replay_field'](data, {key:proof[key] for key in ('kind','document','path','type')})
            if (selected['type'] != 'string' or selected['value'] != row.get('text')
                    or selected['span']['offset'] != row.get('position')):
                raise ValueError('PSB message-reference attestation needs the exact complete-tree field')
            result.setdefault(row['position'], []).append(dict(version=1, kind='psb_message_reference',
                message_reference_only=True, position=row['position'], script_sha256=digest,
                document_sha256=selected['document_sha256']))
            total += 1
            if total > MAX_NODES:
                raise ValueError('Native node attestations exceed the bounded record budget')
    return result


def resolve_native_node(data, link, dialogue, *, span, integer, view):
    """Join saved scope and scalar to exactly one host-attested dialogue row."""
    if isinstance(link, dict) and link.get('kind') == 'typed_record':
        # Public for diagnostics: a structural tuple join deliberately does not
        # confer current-state eligibility. save_facts enforces that boundary.
        return TYPED['resolve_record'](data, link, dialogue)
    _keys(link, ('kind', 'scope', 'position'), ('kind', 'scope', 'position'), 'Saved native node link')
    if link['kind'] != 'call_return':
        raise ValueError('Unsupported saved native node link kind')
    scope = _name(span(data, link['scope']), 'Saved native scope')
    pointer = link['position']
    if not isinstance(pointer, dict) or 'encoding' in pointer:
        raise ValueError('Saved native coordinate needs scalar byte evidence')
    raw = view(data, pointer.get('view', []))
    value = integer(raw, pointer.get('offset'), pointer.get('width', 4), pointer.get('endian', 'little'))
    matches, examined = [], 0
    for row in dialogue:
        records = row.get('_native', [])
        if not isinstance(records, list) or len(records) > 32:
            raise ValueError('Canonical native node records must be a bounded list')
        matched = False
        for record in records:
            examined += 1
            if examined > MAX_NODES:
                raise ValueError('Canonical native node index exceeds its bounded record budget')
            if (not isinstance(record, dict) or record.get('version') != VERSION
                    or record.get('position') != row.get('position')
                    or not isinstance(record.get('script_sha256'), str)
                    or not re.fullmatch('[0-9a-f]{64}', record['script_sha256'])
                    or not isinstance(record.get('schema_sha256'), str)
                    or not re.fullmatch('[0-9a-f]{64}', record['schema_sha256'])):
                raise ValueError('Canonical native node record is invalid or is bound to another row')
            if record.get('kind') == link['kind'] and record.get('scope') == scope and record.get('value') == value:
                matched = True
        if matched:
            matches.append(row)
    if len(matches) != 1:
        raise ValueError('Saved native scope and return address must identify exactly one independently attested script node; matches=' + str(len(matches)))
    return matches[0]
