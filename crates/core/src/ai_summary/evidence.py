"""Format-neutral byte evidence checks. Never executes a model's verification code."""
import hashlib
import json
import pathlib
import runpy
import re
import zlib

CODECS = runpy.run_path(str(pathlib.Path(__file__).with_name("bytecodecs.py")))
LITERALS = runpy.run_path(str(pathlib.Path(__file__).with_name("literal_evidence.py")))
TYPED = runpy.run_path(str(pathlib.Path(__file__).with_name("typed_evidence.py")))
KAG = runpy.run_path(str(pathlib.Path(__file__).with_name("kag_evidence.py")))
ENGINE_PROFILES = runpy.run_path(str(pathlib.Path(__file__).with_name("engine_profiles.py")))
ADAPTERS = runpy.run_path(str(pathlib.Path(__file__).with_name("adapter_evidence.py")))
NATIVE_ARCHIVES = runpy.run_path(str(pathlib.Path(__file__).with_name('archive_evidence.py')))
MAX_ROWS = 500_000
SAVE_VERIFICATION_VERSION = 3
SCRIPT_VERIFICATION_VERSION = 3
ENCODINGS = {'utf-8', 'utf-16-le', 'utf-16-be', 'cp932', 'gb18030', 'ascii'}
NATIVE_MESSAGE_LINK_CONTRACT = ENGINE_PROFILES['NATIVE_MESSAGE_LINK_CONTRACT']
KAG_RESUME_CONTRACT = KAG['CONTRACT']


def view(data, steps):
    return CODECS['view'](data, steps)


def integer(data, offset, width=4, endian='little'):
    if type(offset) is not int or type(width) is not int or width not in (1, 2, 3, 4, 8) or endian not in ('little', 'big') or not 0 <= offset <= len(data) - width:
        raise ValueError('Integer evidence is outside the supplied bytes')
    return int.from_bytes(data[offset:offset + width], endian)


def structured_field(data, proof):
    """Replay a complete fixed data literal before selecting its typed field."""
    return LITERALS['replay_field'](data, proof)


def span(data, proof):
    if not isinstance(proof, dict):
        raise ValueError('Supply text byte evidence {offset,length,encoding}')
    if proof.get('kind') == 'structured_field':
        if proof.get('type') != 'string':
            raise ValueError('Structured TEXT evidence needs type="string"; text_integer is not a text observation')
        return structured_field(data, proof)['value']
    if proof.get('kind') == 'typed_field':
        if set(proof) - {'kind', 'document', 'path', 'type', 'speaker', 'native'}:
            raise ValueError('Typed TEXT evidence has unsupported fields')
        selected = TYPED['replay_field'](data, {key: proof[key] for key in ('kind', 'document', 'path', 'type')})
        if selected['type'] != 'string':
            raise ValueError('Typed TEXT evidence must select an actual string node')
        return selected['value']
    data = view(data, proof.get('view', []))
    offset, length = proof.get('offset'), proof.get('length')
    encoding = proof.get('encoding')
    if type(offset) is not int or type(length) is not int or not 0 <= offset <= len(data) or not 0 <= length <= len(data) - offset or encoding not in ENCODINGS:
        raise ValueError('Invalid text byte evidence or encoding')
    # Strict decoding: replacement characters and normalization must not mask bad offsets.
    return data[offset:offset + length].decode(encoding)


def text_table(data, schema):
    """Walk a complete counted table, retaining ordinal IDs and exact text spans.

    fields: text(length_width,unit,encoding), skip(count_width,unit), fixed(length).
    This handles either field order without assuming any particular engine format.
    """
    offset, length = schema['offset'], schema['length']
    if schema.get('kind') == 'length_array':
        count = identifier(data, schema.get('count'))
        width, base, pool_length = schema.get('length_width'), schema.get('base'), schema.get('pool_length')
        unit, terminator = schema.get('unit', 1), schema.get('terminator_width', 0)
        if (type(count) is not int or not 0 < count <= MAX_ROWS or type(width) is not int
                or width not in (1, 2, 3, 4, 8) or type(offset) is not int or type(length) is not int
                or length != count * width or not 0 <= offset <= len(data) - length
                or type(base) is not int or type(pool_length) is not int
                or not 0 <= base <= len(data) - pool_length or pool_length < 0
                or type(unit) is not int or not 1 <= unit <= 64
                or type(terminator) is not int or terminator not in (0, 1, 2)
                or schema.get('encoding') not in ENCODINGS):
            raise ValueError('Length table needs native count evidence, exact packed-array bounds and a bounded text pool')
        steps = [dict(op='slice', offset=base, length=pool_length)] + schema.get('pool_view', [])
        pool, position, result = view(data, steps), 0, []
        for index in range(count):
            size = integer(data, offset + index * width, width, schema.get('endian', 'little')) * unit
            if size < terminator or size > len(pool) - position:
                raise ValueError('Length table record exceeds its text pool: index=' + str(index)
                                 + ', record_bytes=' + str(size) + ', remaining_bytes=' + str(len(pool)-position))
            if terminator and pool[position+size-terminator:position+size] != b'\0' * terminator:
                raise ValueError('Length table record lacks its declared NUL terminator: index=' + str(index))
            proof = dict(offset=position, length=size-terminator, encoding=schema['encoding'], view=steps)
            # Decode from the already transformed pool once; reapplying an XOR
            # or decompression for every row turns full extraction quadratic.
            result.append((span(pool, dict(offset=position, length=size-terminator, encoding=schema['encoding'])), proof))
            position += size
        if position != len(pool):
            raise ValueError('Length table walk did not end exactly at its complete text-pool boundary')
        return result
    if schema.get('kind') == 'offset_array':
        count_proof = schema.get('count')
        count = identifier(data, count_proof)
        width, base, pool_length = schema.get('offset_width'), schema.get('base'), schema.get('pool_length')
        if (type(count) is not int or not 0 < count <= MAX_ROWS or type(width) is not int
                or width not in (1, 2, 3, 4, 8) or type(offset) is not int or type(length) is not int
                or length != count * width or not 0 <= offset <= len(data) - length
                or type(base) is not int or type(pool_length) is not int
                or not 0 <= base <= len(data) - pool_length or pool_length <= 0
                or schema.get('encoding') not in ('utf-8', 'cp932', 'gb18030', 'ascii')):
            raise ValueError('Offset table needs native count evidence, exact array bounds and a bounded single-byte NUL string pool')
        result = []
        for index in range(count):
            relative = integer(data, offset + index * width, width, schema.get('endian', 'little'))
            if relative >= pool_length:
                raise ValueError('Offset table string pointer exceeds its declared string pool: index=' + str(index))
            start = base + relative
            end = data.find(b'\0', start, base + pool_length)
            if end < start:
                raise ValueError('Offset table string has no NUL terminator inside its declared pool: index=' + str(index))
            proof = dict(offset=start, length=end-start, encoding=schema['encoding'])
            result.append((span(data, proof), proof))
        return result
    if type(length) is not int or length < 1 or not 0 <= offset <= len(data) - length:
        raise ValueError('Table bounds exceed supplied bytes')
    fields = schema['fields']
    if not isinstance(fields, list) or not 1 <= len(fields) <= 8 or sum(f.get('kind') == 'text' for f in fields) != 1:
        raise ValueError('Table needs exactly one text field and at most eight fields')
    width = schema.get('count_width', 4)
    endian = schema.get('endian', 'little')
    count = integer(data, offset, width, endian)
    if not 0 < count <= MAX_ROWS:
        raise ValueError('Invalid table record count')
    pos, end, result = offset + width, offset + length, []
    for _ in range(count):
        text = None
        for field in fields:
            kind = field['kind']
            if kind == 'fixed':
                size = field['length']
            elif kind in ('text', 'skip'):
                w = field.get('length_width' if kind == 'text' else 'count_width', 4)
                n = integer(data, pos, w, endian)
                pos += w
                unit = field.get('unit', 1)
                if type(unit) is not int or not 1 <= unit <= 64:
                    raise ValueError('Invalid field size unit')
                size = n * unit
            else:
                raise ValueError('Unknown table field kind')
            if type(size) is not int or not 0 <= size <= end - pos:
                raise ValueError('Table record overruns its declared boundary: table_offset=' + str(offset)
                                 + ', table_length=' + str(length) + ', record_offset=' + str(pos)
                                 + ', field_bytes=' + str(size) + ', remaining_bytes=' + str(end-pos))
            if kind == 'text':
                proof = dict(offset=pos, length=size, encoding=field['encoding'])
                text = (span(data, proof), proof)
            pos += size
        result.append(text)
    if pos != end:
        raise ValueError('Table walk did not end exactly at its declared boundary')
    return result


def identifier(data, proof):
    """Read a native speaker ID as either a scalar or an exact text span."""
    if not isinstance(proof, dict):
        raise ValueError('Speaker ID needs native integer or text byte evidence')
    if proof.get('kind') == 'typed_field':
        selected = TYPED['replay_field'](data, proof)
        value = selected['value']
        if (selected['type'] not in ('string', 'integer') or type(value) not in (str, int)
                or type(value) is int and not 0 <= value <= 2**64-1):
            raise ValueError('Typed speaker ID requires an actual string or unsigned integer field')
        return value
    if 'encoding' in proof:
        return span(data, proof)
    raw = view(data, proof.get('view', []))
    return integer(raw, proof.get('offset'), proof.get('width', 4), proof.get('endian', 'little'))


def external_speaker(data, proof, resources, table_index=None, source=None):
    if not isinstance(proof, dict):
        raise ValueError('External speaker requires a verified resource/entry and native reference')
    resource = (resources or {}).get(proof.get('resource'), {})
    record = resource.get(proof.get('entry'))
    if not isinstance(record, dict):
        raise ValueError('External speaker requires a byte-verified companion entry from load_script_metadata')
    reference = proof.get('reference')
    if isinstance(reference, dict) and reference.get('mode') == 'table_index':
        if type(table_index) is not int:
            raise ValueError('Cross-script speaker IDs need an independently verified current text-table index')
        if not source or record.get('_source') != source:
            raise ValueError('Message IDs are local to each script. Reload this companion with target_script pointing to the current transcript; never reuse another script\'s matching numeric ID')
        native = table_index
    else:
        native = identifier(data, reference)
    if type(native) is not type(record['id']) or native != record['id']:
        raise ValueError('Script speaker ID does not match the byte-verified companion ID')
    return record['text']


_SPEAKER_SPAN_SCHEMA = ('text span {offset:<byte offset>,length:<byte length>,encoding:"utf-8"|"utf-16-le"|'
                        '"utf-16-be"|"cp932"|"gb18030"|"ascii"}; the exact decoded span must equal text')
_SPEAKER_ID_SCHEMA = ('native scalar {offset:<byte offset>,width:1|2|3|4|8,endian:"little"|"big"} '
                      'or native text span {offset,length,encoding}; encoding is a text codec, '
                      'never uint32-le. Read the actual ID bytes without stripping prefixes')
_SPEAKER_RESOURCE_SCHEMA = ('{resource:<verified global resource key>,entry:<verified entry key>,'
                            'reference:<native token scalar or text span in this companion input>}. '
                            'Use metadata["speaker_resources"][resource][entry]={id,text}; '
                            'there is no extra speakers nesting. table_index is for scoped transcript '
                            'rows, not a companion-to-name-table reference')
_SPEAKER_LOCALIZED_SCHEMA = ('Direct byte evidence cannot prove a different localized display name. '
                             'First verify the localized name-table entry, then use '
                             + _SPEAKER_RESOURCE_SCHEMA + '. Keep the localized display name in text '
                             'and prove the original native token with reference')


class SpeakerMetadataError(ValueError):
    """A bounded structural failure locator that never echoes a name or token."""
    def __init__(self, cause, path, entry_index, expected, observed):
        self.diagnostic = dict(path=path, entry_index=entry_index, expected=expected, observed=observed)
        super().__init__(cause + ' at ' + path + '. Expected ' + expected
                         + '. Observed structural fields: ' + json.dumps(observed, separators=(',', ':')))


class ScriptRowEvidenceError(SpeakerMetadataError):
    """Reuse the host's safe evidence-error envelope for an exact script row."""
    def __init__(self, cause, row_index, field, position, expected, observed, category=None):
        super().__init__(cause, 'rows[' + str(row_index) + '].' + field, row_index, expected, observed)
        self.diagnostic.pop('entry_index', None)
        self.diagnostic.update(row_index=row_index, position=position if type(position) is int else None)
        if category is not None:
            self.category = category
            self.diagnostic['category'] = category


def _script_table_link_hint(required=False):
    hint = ('Declared tables are not connected to this row: evidence.table/index is missing. '
            'Raw spans read input bytes after only output.view and their own evidence.view; '
            'table.pool_view is not automatically applied to raw spans. ')
    if required:
        return hint + 'Known indexed regions still require the complete matching table and actual instruction operand reference; ordinal mode cannot replace it'
    return (hint + 'If the native message coordinate has been independently proved to be a table index, use '
            '{position:n,evidence:{table:<key>,index:n,reference:{mode:"ordinal"}}}. '
            'For bytecode instruction coordinates keep the actual operand reference instead')


def _script_resource_choices(resources, source=None, scoped_only=False):
    """Expose resource keys and numeric native IDs, never displayed names."""
    result = []
    for key, records in (resources or {}).items():
        relevant = [record for record in records.values() if isinstance(record, dict)
                    and (not scoped_only or source and record.get('_source') == source)]
        if not relevant:
            continue
        result.append(dict(resource=str(key)[:128], records=len(relevant),
                           native_ids=sorted({record['id'] for record in relevant if type(record.get('id')) is int})[:20],
                           id_types=sorted({type(record.get('id')).__name__ for record in relevant})))
    return result[:20]


def _speaker_entry_path(key, index):
    # Mechanical IDs are useful locators. Arbitrary keys may themselves contain
    # a displayed name, so preserve their position/hash instead of logging them.
    if isinstance(key, str) and re.fullmatch(r'(?:[0-9]{1,20}|(?:message|entry|speaker|name|m)[-_:]?[0-9]{1,20})', key):
        return 'speakers[' + json.dumps(key) + ']'
    fingerprint = hashlib.sha256(str(key).encode('utf-8')).hexdigest()[:12]
    return 'speakers[entry_index=' + str(index) + ',key_sha256=' + fingerprint + ']'


def _speaker_proof_location(field, proof, scalar=False):
    if not isinstance(proof, dict):
        return field
    if 'encoding' in proof and (not isinstance(proof['encoding'], str) or proof['encoding'] not in ENCODINGS):
        return field + '.encoding'
    if type(proof.get('offset')) is not int:
        return field + '.offset'
    if 'encoding' in proof and type(proof.get('length')) is not int:
        return field + '.length'
    if scalar and 'encoding' not in proof:
        if type(proof.get('width', 4)) is not int or proof.get('width', 4) not in (1, 2, 3, 4, 8):
            return field + '.width'
        if proof.get('endian', 'little') not in ('little', 'big'):
            return field + '.endian'
    return field


def _speaker_observed(proof):
    """Only byte coordinates and schema/type information may enter diagnostics."""
    if not isinstance(proof, dict):
        return dict(type=type(proof).__name__)
    result = dict(type='object')
    for key in ('offset', 'length', 'width'):
        if key in proof:
            result[key] = proof[key] if type(proof[key]) is int else dict(type=type(proof[key]).__name__)
    for key, allowed in (('encoding', ENCODINGS), ('endian', ('little', 'big')), ('mode', ('table_index',))):
        if key in proof:
            result[key] = proof[key] if isinstance(proof[key], str) and proof[key] in allowed else 'unsupported'
    for key in ('resource', 'entry', 'reference'):
        if key in proof:
            result[key + '_type'] = type(proof[key]).__name__
    return result


_native_cstring_end = ENGINE_PROFILES['_native_cstring_end']


def native_message_link(data, record):
    return ENGINE_PROFILES['native_message_link'](data, record, integer=integer)


def speaker_metadata(data, output, speaker_resources=None, require_native_link=False):
    """Verify companion names and their native IDs without trusting helper JSON."""
    if not isinstance(output, dict):
        raise ValueError('Companion metadata must be a JSON object')
    # The legacy arbitrary mapping remains useful for non-speaker parser inputs,
    # but only the explicit envelope can establish externally verified names.
    if 'values' not in output or 'speakers' not in output:
        if require_native_link:
            raise ValueError('Scoped companion metadata needs values/speakers with supported native_link proofs; needs_proof')
        return output, {}
    values, records = output['values'], output['speakers']
    if not isinstance(values, dict) or not isinstance(records, dict) or len(records) > MAX_ROWS:
        raise ValueError('Speaker metadata needs values and speakers objects')
    psb_metadata = _looks_like_psb(data) or NATIVE_ARCHIVES['requires_document_member_verification'](output)
    if psb_metadata and output.get('view'):
        raise ValueError('PSB speaker metadata requires complete typed documents without global output.view')
    if output.get('view') and (require_native_link or any(isinstance(record, dict) and 'native_link' in record for record in records.values())):
        raise ValueError('Scoped native_link proofs require original raw input coordinates, without output view transforms')
    raw = view(data, output.get('view', []))
    schemas = output.get('tables', {})
    if not isinstance(schemas, dict) or len(schemas) > 32:
        raise ValueError('Invalid companion speaker table schemas')
    tables = {key: text_table(raw, schema) for key, schema in schemas.items()}
    verified = {}
    psb_document_sha256 = None
    for entry_index, (key, record) in enumerate(records.items()):
        path = _speaker_entry_path(key, entry_index)
        field, expected, inspected = '', 'a record object with id, text, evidence and native id_evidence', record
        try:
            if not isinstance(key, str) or not isinstance(record, dict):
                raise ValueError('Speaker entries must have string keys and record objects')
            text, native = record.get('text'), record.get('id')
            if ((text is not None and (not isinstance(text, str) or not text.strip() or len(text.encode()) > 1024))
                    or type(native) not in (int, str)):
                field = 'id' if type(native) not in (int, str) else 'text'
                expected = 'native string/integer id and an exact displayed name (or null only with actual empty-name proof)'
                inspected = dict()
                raise ValueError('Speaker entries need an actual displayed name and native string/integer ID')
            if require_native_link or 'native_link' in record:
                field, expected, inspected = 'native_link', 'a supported local name/message command link tying the exact name field and native ID; see verification_contract', record.get('native_link')
                native_message_link(raw, record)
            proof = record.get('evidence')
            field, expected, inspected = 'evidence', _SPEAKER_SPAN_SCHEMA, proof
            if psb_metadata:
                id_proof = record.get('id_evidence')
                for location, item in (('evidence', proof), ('id_evidence', id_proof)):
                    if (not isinstance(item, dict) or item.get('kind') != 'typed_field'
                            or item.get('document', {}).get('syntax') != 'psb'):
                        raise ValueError('PSB speaker names and IDs require complete typed_field references; pools and raw spans cannot replace the root')
                    selected = TYPED['replay_field'](raw, item)
                    if psb_document_sha256 is not None and selected['document_sha256'] != psb_document_sha256:
                        raise ValueError('PSB speaker metadata must use one complete decoded document for every name and ID')
                    psb_document_sha256 = selected['document_sha256']
                    if location == 'evidence':
                        if selected['type'] not in ('string', 'null', 'void'):
                            raise ValueError('PSB speaker name requires an actual string or explicit empty-name field')
                        actual = selected['value']
                if identifier(raw, id_proof) != native:
                    raise ValueError('Companion speaker ID does not match its native typed field')
            elif isinstance(proof, dict) and 'resource' in proof:
                expected = _SPEAKER_RESOURCE_SCHEMA
                field = 'evidence.resource'
                resource = (speaker_resources or {}).get(proof.get('resource'), {})
                field = ('evidence.resource' if not resource else
                         'evidence.entry' if not isinstance(resource.get(proof.get('entry')), dict) else
                         'evidence.reference.mode' if isinstance(proof.get('reference'), dict)
                         and proof['reference'].get('mode') == 'table_index' else
                         _speaker_proof_location('evidence.reference', proof.get('reference'), scalar=True))
                inspected = proof.get('reference') if field.startswith('evidence.reference') else proof
                actual = external_speaker(raw, proof, speaker_resources)
                inspected = record.get('id_evidence')
                field, expected = _speaker_proof_location('id_evidence', inspected, scalar=True), _SPEAKER_ID_SCHEMA
                if identifier(raw, inspected) != native:
                    raise ValueError('Companion message ID does not match its native bytes')
            elif isinstance(proof, dict) and 'table' in proof:
                expected = '{table:<complete independently walked table key>,index:<integer ordinal within that table>}'
                field = 'evidence.table'
                table, index = tables.get(proof['table']), proof.get('index')
                field = 'evidence.table' if table is None else 'evidence.index'
                if table is None or type(index) is not int or not 0 <= index < len(table):
                    raise ValueError('Companion name requires a valid complete-table ordinal')
                actual = table[index][0]
                if 'id_evidence' not in record:
                    field, expected = 'id', 'id equal to the independently walked table index, or supply ' + _SPEAKER_ID_SCHEMA
                    if type(native) is not int or native != index:
                        raise ValueError('Ordinal speaker ID must equal the independently walked table index')
                else:
                    inspected = record['id_evidence']
                    field, expected = _speaker_proof_location('id_evidence', inspected, scalar=True), _SPEAKER_ID_SCHEMA
                    if identifier(raw, inspected) != native:
                        raise ValueError('Companion speaker ID does not match its native bytes')
            else:
                field = _speaker_proof_location('evidence', proof)
                actual = span(raw, proof)
                inspected = record.get('id_evidence')
                field, expected = _speaker_proof_location('id_evidence', inspected, scalar=True), _SPEAKER_ID_SCHEMA
                if identifier(raw, inspected) != native:
                    raise ValueError('Companion speaker ID does not match its native bytes')
            field, inspected = 'evidence', proof
            if text is None:
                expected = ('the actual NUL empty-name span: length 1 for a single-byte codec or 2 for UTF-16, '
                            'plus the native message id_evidence; a zero-length span/missing name is not narration')
                if actual not in ('\0', '\0\0', None):
                    raise ValueError('Narration needs the actual NUL empty-name field and native message ID; a missing mapping is not narration')
            elif actual != text:
                expected = _SPEAKER_SPAN_SCHEMA + '. ' + _SPEAKER_LOCALIZED_SCHEMA
                raise ValueError('Companion speaker name does not match its native bytes')
            verified[key] = dict(id=native, text=text)
        except (ValueError, TypeError, KeyError, IndexError, OverflowError, zlib.error) as error:
            if isinstance(error, UnicodeError):
                cause = 'Invalid text byte evidence or encoding'
            elif isinstance(error, (TypeError, KeyError, IndexError, OverflowError)):
                cause = 'Invalid companion byte evidence field types or schema'
            elif isinstance(error, zlib.error):
                cause = 'Invalid compressed companion byte evidence'
            else:
                cause = str(error)
            observed = dict(_speaker_observed(inspected), input_bytes=len(raw))
            raise SpeakerMetadataError(cause, path + ('.' + field if field else ''), entry_index,
                                       expected, observed) from error
    return values, verified


def _looks_like_psb(data):
    # Recognize a common whole-member mask only to prevent legacy pool-span
    # downgrade. This does not decrypt, choose a mask or attest an archive filter.
    if not isinstance(data, bytes) or len(data) < 4:
        return False
    # A standard MDF containing PSB cannot hide the root behind output.view.
    # At most the existing view-step budget can be expressed by an adapter.
    for _ in range(8):
        if data[:4] != b'mdf\0':
            break
        try:
            data = CODECS['decode']('mdf', data)
        except ValueError:
            return True  # Recognized but invalid/unknown MDF never becomes a raw pool.
    if data[:4] == b'mdf\0':
        return True  # Budget exhaustion is unsupported, never a legacy-span downgrade.
    if len(data) < 4:
        return False
    key = data[0] ^ ord('P')
    return bytes(value ^ key for value in data[:4]) == b'PSB\0'


def script_rows(data, output, required_table_regions=None, speaker_resources=None, source=None,
                required_speaker_scope=False):
    """Verify text bindings and any host-identified complete indexed regions."""
    if required_table_regions is not None and not isinstance(required_table_regions, list):
        raise ValueError('Required indexed table regions must be a list of {offset,length}')
    required = set()
    for region in required_table_regions or []:
        if (not isinstance(region, dict) or type(region.get('offset')) is not int
                or type(region.get('length')) is not int or region['offset'] < 0
                or region['length'] < 1 or region['offset'] > len(data) - region['length']):
            raise ValueError('Required indexed table region exceeds the original input bytes')
        required.add((region['offset'], region['length']))
    if required and (not isinstance(output, dict) or output.get('view')):
        raise ValueError('Known indexed script regions require {rows,tables} in the original byte view; direct spans cannot replace complete string tables. Call the verification_contract TOOL.')
    original_psb = _looks_like_psb(data)
    if isinstance(output, list):
        rows, schemas = output, {}
    elif isinstance(output, dict):
        rows, schemas = output.get('rows'), output.get('tables', {})
        data = view(data, output.get('member_filter', []))
        original_psb = original_psb or _looks_like_psb(data)
        if original_psb and output.get('view'):
            raise ValueError('PSB scripts require the complete checksum-verified member; output.view cannot slice or replace its typed root. Put whole-member archive decoding in member_filter.')
        data = view(data, output.get('view', []))
    else:
        raise ValueError('Return rows with byte evidence, optionally {rows,tables}')
    if not isinstance(rows, list) or not rows or len(rows) > MAX_ROWS or not isinstance(schemas, dict) or len(schemas) > 32:
        raise ValueError('Invalid transcript or table schemas')
    typed_psb = original_psb or data[:4] == b'PSB\0'
    matching_tables = set()
    if required:
        covered = set()
        for name, schema in schemas.items():
            if (isinstance(schema, dict) and type(schema.get('offset')) is int
                    and type(schema.get('length')) is int):
                region = (schema['offset'], schema['length'])
                if region in required:
                    if schema.get('kind') in ('offset_array', 'length_array'):
                        raise ValueError('Known counted table regions require complete interleaved record schemas; indirect arrays cannot replace their original structure')
                    matching_tables.add(name)
                    covered.add(region)
        if covered != required:
            raise ValueError('Declare complete table schemas with offset/length exactly matching every required indexed region; do not skip empty or unreferenced string records. Call the verification_contract TOOL.')
    tables = {key: text_table(data, schema) for key, schema in schemas.items()}
    scoped = {}
    for resource, records in (speaker_resources or {}).items():
        for entry, record in records.items():
            if source and record.get('_source') == source:
                native = record['id']
                if type(native) is not int:
                    raise ValueError('Scoped message bindings require native integer message IDs')
                if native in scoped and scoped[native][2] != record['text']:
                    raise ValueError('Conflicting displayed names for this script/message ID')
                scoped[native] = (resource, entry, record['text'])
    scope_required = required_speaker_scope or bool(scoped)
    seen, scoped_messages = set(), set()
    psb_document_sha256 = None
    for row_index, row in enumerate(rows):
        field, expected, inspected = 'position', 'nonnegative native integer coordinate', None
        observed = dict(input_bytes=len(data))
        category = None
        try:
            if not isinstance(row, dict) or type(row.get('position')) is not int or row['position'] < 0:
                raise ValueError('Each dialogue needs integer position and nonempty text')
            field, expected = 'text', 'nonempty exact decoded text of at most 4096 characters'
            if not isinstance(row.get('text'), str) or not row['text'].strip() or len(row['text']) > 4096:
                raise ValueError('Each dialogue needs integer position and nonempty text')
            field, expected = 'speaker', 'an actual byte-verified displayed name or null with verified narration when scoped'
            if 'speaker' not in row or row['speaker'] is not None and (not isinstance(row['speaker'], str) or not row['speaker'].strip() or len(row['speaker'].encode()) > 1024):
                raise ValueError('Each dialogue needs speaker: an actual name or null')
            field, expected = 'position', 'a unique native coordinate for this transcript'
            if row['position'] in seen:
                raise ValueError('Duplicate dialogue position')
            seen.add(row['position'])
            proof = row.get('evidence')
            field, expected, inspected = 'evidence', 'direct {offset,length,encoding} or indexed {table,index,reference}', proof
            if not isinstance(proof, dict) or not proof:
                raise ValueError('Each script row needs evidence: direct {offset,length,encoding} or indexed {table,index,reference}. Call the verification_contract TOOL for the output schema; do not repeat binary scans to fix missing output fields.')
            if typed_psb and proof.get('kind') != 'typed_field':
                raise ValueError('PSB messages need typed_field paths to actual message references in the complete root. A string-pool byte span, unique text, table ordinal or hash cannot prove a message or its coordinate.')
            if required_speaker_scope and not scoped:
                category = 'speaker_metadata'
                field, expected = 'evidence.speaker', 'nonempty byte-verified speaker/narration records for this exact target_script'
                observed.update(required_speaker_scope=True, scoped_records=0)
                raise ValueError('Required target_script speaker/narration bindings are empty or missing. '
                                 'Repair and reload the corresponding load_script_metadata helper for this exact target_script, '
                                 'including actual empty-name NUL evidence for narration. '
                                 'Unresolved null speakers, other scripts\' names, or omitting the chapter cannot satisfy this requirement')
            if scope_required and 'table' not in proof:
                field, expected = 'evidence', 'indexed table/index evidence connected to this target_script\'s verified native message bindings'
                raise ValueError('Scoped transcript rows require table/index and their own native speaker/narration binding; '
                                 'direct spans cannot bypass a required message scope')
            if required:
                expected = ('a table matching a required complete region and its actual instruction operand reference'
                            if 'table' in proof else _script_table_link_hint(True))
                field = 'evidence.table' if 'table' in proof else 'evidence'
                if not isinstance(proof.get('table'), str) or proof['table'] not in matching_tables:
                    raise ValueError('Known indexed script rows must reference a table matching a required complete region; direct spans or unrelated tables cannot prove the string index.')
                binding = proof.get('reference')
                field, expected, inspected = 'evidence.reference', 'actual instruction operand {base,operand_offset,width,endian}; ordinal cannot replace this reference', binding
                if (not isinstance(binding, dict) or binding.get('mode') == 'ordinal'
                        or 'operand_offset' not in binding):
                    raise ValueError('Known indexed script rows need reference with an actual operand_offset; ordinal positions cannot replace instruction-to-string operand evidence. Call the verification_contract TOOL.')
            if proof.get('kind') == 'typed_field':
                field, expected = 'evidence.path', 'a complete typed-document string reference and its original serialized node position'
                if typed_psb and (proof.get('document', {}).get('syntax') != 'psb'
                                  or proof.get('document', {}).get('view')):
                    raise ValueError('PSB message proofs must use the complete recovered member with document.syntax="psb" and an empty document.view; sliced pools cannot replace the native tree')
                selected = TYPED['replay_field'](data, {key: proof[key] for key in ('kind', 'document', 'path', 'type')})
                if proof.get('document', {}).get('syntax') == 'psb':
                    digest = selected['document_sha256']
                    if psb_document_sha256 is not None and digest != psb_document_sha256:
                        raise ValueError('Every PSB row must use one complete decoded document; per-row crypto/frame roots disagree')
                    psb_document_sha256 = digest
                    inline = proof.get('speaker')
                    if isinstance(inline, dict) and inline.get('kind') == 'typed_field':
                        _, spoken_digest, _ = TYPED['_parse'](data, inline['document'])
                        if spoken_digest != psb_document_sha256:
                            raise ValueError('Inline PSB speaker and message must use the same complete decoded document')
                if selected['type'] != 'string' or row['position'] != selected['span']['offset']:
                    raise ValueError('Typed message position must equal its actual serialized reference-node offset, never its string-pool byte offset or an invented line ordinal')
                actual = selected['value']
            elif 'table' in proof:
                field, expected, inspected = 'evidence.table', 'a declared, completely verified text table key', proof
                table = tables.get(proof['table'])
                if table is None:
                    raise ValueError('Unknown text table or out-of-bounds string index')
                field, expected = 'evidence.index', 'a native integer ordinal within the complete text table'
                index = proof.get('index')
                observed['table_count'] = len(table)
                if type(index) is not int or not 0 <= index < len(table):
                    raise ValueError('Unknown text table or out-of-bounds string index')
                observed['native_index'] = index
                actual, _ = table[index]
                field, expected, inspected = 'evidence.reference', '{mode:"ordinal"} for proved message-index coordinates, otherwise actual instruction operand {base,operand_offset,width,endian}', proof.get('reference')
                binding = proof['reference']
                if binding.get('mode') == 'ordinal':
                    field, expected = 'position', 'the independently verified table ordinal used in evidence.index'
                    if row['position'] != index:
                        raise ValueError('Message index must equal the independently verified table ordinal')
                else:
                    base = binding.get('base', 0)
                    operand = binding['operand_offset']
                    if type(base) is not int or base < 0 or type(operand) is not int or operand < 0:
                        raise ValueError('Invalid instruction-to-string reference')
                    read_index = integer(data, base + row['position'] + operand, binding.get('width', 4), binding.get('endian', 'little'))
                    if read_index != index:
                        raise ValueError('Instruction string index does not match the text table ordinal: '
                                         + 'operand_offset=' + str(base + row['position'] + operand)
                                         + ', actual_index=' + str(read_index) + ', declared_index=' + str(index)
                                         + ', table_count=' + str(len(table))
                                         + '. Keep the full table including empty entries; correct this reference instead of rescanning the container.')
            else:
                field, expected, inspected = _speaker_proof_location('evidence', proof), _SPEAKER_SPAN_SCHEMA, proof
                if tables:
                    expected += '. ' + _script_table_link_hint()
                actual = span(data, proof)
                field = 'position'
                if row['position'] != proof['offset']:
                    expected = 'the direct evidence.offset byte coordinate; indexed coordinates require table/reference evidence'
                    if tables:
                        expected += '. ' + _script_table_link_hint()
                    raise ValueError('Direct-text position must equal its byte offset; indexed formats need table/reference evidence')
            field, inspected = 'evidence', proof
            if actual != row['text']:
                expected = (_script_table_link_hint() if tables and 'table' not in proof
                            else 'the exact decoded string without stripping or normalization')
                raise ValueError('Text does not match independently decoded bytes at its actual string index: '
                                 + 'position=' + str(row['position']) + ', declared_utf8_bytes=' + str(len(row['text'].encode()))
                                 + ', actual_utf8_bytes=' + str(len(actual.encode()))
                                 + (', index=' + str(proof.get('index')) if 'table' in proof else ', offset=' + str(proof.get('offset'))))
            speaker_proof = proof.get('speaker')
            field, expected, inspected = 'evidence.speaker', 'verified speaker bytes or {resource,entry,reference}; scoped narration also needs its own empty-name record', speaker_proof
            scoped_row = scope_required
            # Check keys before a null/name comparison so a stale scoped alias is
            # diagnosed as a resource selection error, without echoing names.
            if isinstance(speaker_proof, dict) and ('resource' in speaker_proof or scoped_row):
                field = 'evidence.speaker.resource'
                expected = 'a loaded resource key' + (' scoped to this transcript via target_script' if scoped_row else '')
                records = (speaker_resources or {}).get(speaker_proof.get('resource'))
                if not isinstance(records, dict) or (scoped_row and not any(
                        isinstance(record, dict) and record.get('_source') == source for record in records.values())):
                    raise ValueError('External speaker requires an available byte-verified resource for the current transcript')
                field, expected = 'evidence.speaker.entry', 'a verified entry in the selected resource with the current native message ID when scoped'
                cited = records.get(speaker_proof.get('entry'))
                if not isinstance(cited, dict):
                    raise ValueError('External speaker requires a byte-verified companion entry from load_script_metadata')
                if scoped_row and (cited.get('_source') != source or cited.get('id') != proof['index']):
                    observed['native_index'] = proof['index']
                    if type(cited.get('id')) is int:
                        observed['cited_native_id'] = cited['id']
                    raise ValueError('Script speaker ID does not match the byte-verified companion ID or current transcript scope; each scoped message needs its own verified speaker or empty-name narration proof')
            if scoped_row:
                native = proof['index']
                observed['native_index'] = native
                scoped_record = scoped.get(native)
                field, expected = 'evidence.speaker', 'this message index\'s verified scoped speaker or actual empty-name narration record'
                if scoped_record is None:
                    raise ValueError('Missing speaker/narration proof for this script\'s message index ' + str(native)
                                     + '. Load the complete matching original script; exclude verified non-message auxiliary strings instead of treating missing mappings as narration')
                cited = ((speaker_resources or {}).get(speaker_proof.get('resource'), {}).get(speaker_proof.get('entry'))
                         if isinstance(speaker_proof, dict) else None)
                if not isinstance(cited, dict) or cited.get('_source') != source or cited.get('id') != native:
                    raise ValueError('Every scoped message needs its own verified speaker or empty-name narration proof; do not silently replace an available name with null')
                field, expected = 'speaker', 'the exact displayed name or narration null from this message index\'s verified scoped record'
                if row['speaker'] != scoped_record[2]:
                    raise ValueError('Every scoped message needs its own verified speaker or empty-name narration proof; do not silently replace an available name with null')
                scoped_messages.add(native)
            if isinstance(speaker_proof, dict) and 'resource' in speaker_proof:
                field, expected, inspected = 'evidence.speaker.reference', 'native scalar/text-span ID bytes, or {mode:"table_index"} for a matching target_script scoped entry', speaker_proof.get('reference')
                actual_name = external_speaker(data, speaker_proof, speaker_resources,
                                               proof.get('index') if 'table' in proof else None, source=source)
                field, expected = 'speaker', 'the exact verified displayed name or verified narration null'
                if row['speaker'] != actual_name:
                    raise ValueError('Mapped speaker must equal its verified displayed name; do not discard it as null')
            elif row['speaker'] is not None:
                field, expected, inspected = _speaker_proof_location('evidence.speaker', speaker_proof), _SPEAKER_SPAN_SCHEMA, speaker_proof
                if span(data, speaker_proof) != row['speaker']:
                    raise ValueError('Speaker name lacks matching byte evidence; use null when unresolved')
        except (ValueError, TypeError, KeyError, IndexError, AttributeError, OverflowError, zlib.error) as error:
            if isinstance(error, UnicodeError):
                cause = 'Invalid text byte evidence or encoding'
            elif isinstance(error, (TypeError, KeyError, IndexError, AttributeError, OverflowError)):
                cause = 'Invalid script row byte evidence field types or schema'
            elif isinstance(error, zlib.error):
                cause = 'Invalid compressed script row byte evidence'
            else:
                cause = str(error)
            observed.update(_speaker_observed(inspected))
            if field in ('evidence.speaker.resource', 'evidence.speaker.entry'):
                # Enumerate potentially large companion maps only on failure.
                observed['available_resources'] = _script_resource_choices(speaker_resources, source, scoped_row)
            raise ScriptRowEvidenceError(cause, row_index, field, row.get('position') if isinstance(row, dict) else None,
                                         expected, observed, category=category) from error
    missing = sorted(set(scoped) - scoped_messages)
    if missing:
        raise ValueError('Transcript omitted verified native messages from its own companion: ' + str(missing[:20])
                         + '. Preserve every message and its speaker/narration binding')
    return rows


_nxsc_visible_caption = ENGINE_PROFILES['_nxsc_visible_caption']


def _nxsc_current_caption(data):
    return ENGINE_PROFILES['_nxsc_current_caption'](data, integer=integer)


def _verify_nxsc_current_node(data, facts, proof, dialogue, header):
    return ENGINE_PROFILES['_verify_nxsc_current_node'](data, facts, proof, dialogue, header, integer=integer)


_SAVE_TEXT_SHAPE = ('native TEXT span {offset:<byte offset>,length:<byte length>,'
                    'encoding:"utf-8"|"utf-16-le"|"utf-16-be"|"cp932"|"gb18030"|"ascii",view?:[<standard steps>]}')
_SAVE_SCALAR_SHAPE = ('native scalar {offset:<byte offset>,width:1|2|3|4|8,'
                      'endian:"little"|"big",view?:[<standard steps>]}')
_SAVE_VIEW_SHAPE = 'a bounded sequence of supported standard byte-view steps'
_SAVE_NODE_LINK_SHAPE = ('{kind:"call_return",scope:<native TEXT span>,'
                         'position:<native scalar>} identifying one host-attested script node')


class SaveFieldError(ValueError):
    """Add a save field locator without changing the worker's stderr protocol."""
    def __init__(self, cause, field, expected):
        super().__init__(cause + ' at ' + field + '. Expected ' + expected)


def _save_check(operation, field, expected):
    try:
        return operation()
    except SaveFieldError:
        raise
    except (ValueError, TypeError, KeyError, OverflowError) as error:
        raise SaveFieldError(str(error), field, expected) from None


def _save_span(data, proof, field):
    return _save_check(lambda: span(data, proof), field, _SAVE_TEXT_SHAPE)


def save_facts(data, facts, dialogue):
    """Accept only byte-grounded current captions and save-recorded history.

    Every caption requires native source and position proofs. The scoped NXSC
    policy additionally verifies its independently read root current caption.
    The host constructs history itself, so a script prefix cannot masquerade as backlog.
    """
    if not isinstance(facts, dict) or not isinstance(facts.get('anchors'), list):
        raise SaveFieldError('Save parser must return facts with anchors', 'facts.anchors', 'an array of current captions')
    if set(facts) - {'source', 'line', 'scene', 'anchors', 'variables', 'warnings', 'evidence', 'history', 'resume'}:
        raise SaveFieldError('Save facts contain unknown fields; follow verification_contract', 'facts',
                             'only source,line,scene,anchors,variables,warnings,evidence,history,resume')
    # A model-supplied result is never a receipt. Only the independent audited
    # profile branch below can emit a verified resume record.
    facts.pop('resume', None)
    scene = facts.get('scene', '')
    if not isinstance(scene, str):
        observed_type = 'null' if scene is None else type(scene).__name__
        raise ValueError('facts.scene must be a string of at most 1024 UTF-8 bytes. '
                         'When unavailable, omit facts.scene or use an empty string; do not return null. '
                         'Observed type=' + observed_type)
    scene_bytes = len(scene.encode('utf-8'))
    if scene_bytes > 1024:
        raise ValueError('facts.scene must be a string of at most 1024 UTF-8 bytes. '
                         'When unavailable, omit facts.scene or use an empty string. '
                         'Observed type=str, utf8_bytes=' + str(scene_bytes))
    warnings = facts.get('warnings', [])
    if not isinstance(warnings, list) or len(warnings) > 32 or any(not isinstance(w, str) for w in warnings):
        raise ValueError('Invalid save warnings')
    nxsc_header = _nxsc_current_caption(data)
    proof = facts.pop('evidence', {})
    if not isinstance(proof, dict):
        raise SaveFieldError('Save byte evidence must be an object with native field proofs', 'evidence',
                             'an object containing native anchor/source/position or node_link proofs')
    facts.pop('history', None)
    if facts.get('scene') and _save_span(data, proof.get('scene'), 'evidence.scene') != facts['scene']:
        raise SaveFieldError('Scene label needs matching save byte evidence', 'evidence.scene',
                             _SAVE_TEXT_SHAPE + ' matching facts.scene exactly')
    link = proof.get('node_link')
    if isinstance(link, dict) and link.get('kind') == 'kag_label_resume':
        if nxsc_header is not None:
            raise ValueError('NXSC current-caption profile cannot be replaced by a KAG resume entry')
        if facts['anchors']:
            raise SaveFieldError('KAG label resume is an entry coordinate, not a current caption',
                                 'facts.anchors', 'an empty array; keep last-displayed text separate')
        result = _save_check(lambda: KAG['resolve_resume'](data, link, dialogue),
                             'evidence.node_link', 'the audited complete standard KAG bookmark and unique source/label entry')
        row = result['row']
        if ('source' in facts and facts['source'] != row['source']
                or 'line' in facts and (type(facts['line']) is not int or facts['line'] != row['position'])):
            raise SaveFieldError('KAG resume source/position disagrees with the independently selected label',
                                 'facts.source/facts.line', 'omit them or exactly match the verified canonical label entry')
        facts.update(source=row['source'], line=row['position'], resume=result['resume'], history=[])
        return facts
    if not facts['anchors']:
        if nxsc_header is not None:
            raise ValueError('NXSC native current-caption requires a verified current node; '
                             'empty anchors or scene-only coverage cannot hide it')
        return facts
    typed_link = proof.get('node_link')
    if isinstance(typed_link, dict) and typed_link.get('kind') == 'typed_record':
        diagnostic = _save_check(lambda: TYPED['resolve_record'](data, typed_link, dialogue),
                                 'evidence.node_link', 'an exact typed saved-record relationship')
        raise SaveFieldError('Typed saved record matched, but its engine current/choice/resume role is unverified; '
                             'recorded_node is a diagnostic relationship, not current coverage',
                             'evidence.node_link.state_role',
                             'a separately supported engine state profile; history, preview and resume markers '
                             'cannot become current by setting a model role flag')
    # Structured native fields currently establish observations, not an audited
    # engine's current-state selector or script-coordinate relation. Refuse any
    # mixed legacy proof rather than accepting a coincidental binary position.
    current_fields = [('evidence.' + field, proof.get(field)) for field in ('anchor', 'source', 'position')]
    link = proof.get('node_link')
    if isinstance(link, dict):
        current_fields.extend(('evidence.node_link.' + field, link.get(field)) for field in ('scope', 'position'))
    for field, item in current_fields:
        if isinstance(item, dict) and item.get('kind') in ('structured_field', 'typed_field'):
            category = 'Structured' if item.get('kind') == 'structured_field' else 'Typed'
            raise SaveFieldError(category + ' current-node evidence is unsupported without a verified typed node and audited state profile',
                                 field, 'an independently supported native link and current-state profile; '
                                        'structured fields currently prove observations or scene-only diagnostics only')
    if (_looks_like_psb(data) or any(isinstance(item, dict) and any(
            isinstance(step, dict) and step.get('op') in ('mdf', 'psb_decrypt')
            for step in (item.get('view', []) if isinstance(item.get('view', []), list) else []))
            for _, item in current_fields)):
        raise SaveFieldError('PSB current-node evidence requires an independently supported engine state profile',
                             'evidence', 'complete typed data or decoded byte spans prove observations, not a current-state selector')
    if facts.get('scene'):
        anchor_proof, scene_proof = proof.get('anchor'), proof.get('scene')
        if (isinstance(anchor_proof, dict) and isinstance(scene_proof, dict)
                and (anchor_proof.get('view', []), anchor_proof.get('offset'), anchor_proof.get('length'))
                == (scene_proof.get('view', []), scene_proof.get('offset'), scene_proof.get('length'))):
            raise ValueError('A scene label and current caption need distinct saved fields. One preview field cannot prove both. If only a scene is known, return anchors=[] with scene evidence and warnings; do not promote scene-only coverage to a node.')
    if not isinstance(facts.get('source'), str) or len(facts['source'].encode()) > 4096 or type(facts.get('line')) is not int or not 0 <= facts['line'] <= 2**64 - 1:
        source_invalid = not isinstance(facts.get('source'), str) or len(facts['source'].encode()) > 4096
        raise SaveFieldError('Save facts need the exact current source and integer position',
                             'facts.source' if source_invalid else 'facts.line',
                             'a source string of at most 4096 UTF-8 bytes' if source_invalid else 'an integer from 0 through 2**64-1')
    if len(facts['anchors']) != 1:
        raise SaveFieldError('Current anchor needs exact byte evidence in this save', 'facts.anchors',
                             'exactly one current caption string')
    if _save_span(data, proof.get('anchor'), 'evidence.anchor') != facts['anchors'][0]:
        raise SaveFieldError('Current anchor needs exact byte evidence in this save',
                             'evidence.anchor' if isinstance(facts['anchors'][0], str) else 'facts.anchors[0]',
                             _SAVE_TEXT_SHAPE + ' matching facts.anchors[0] exactly')
    def stem(text):
        return text.replace('\\', '/').rsplit('/', 1)[-1].rsplit('.', 1)[0].casefold()
    if 'node_link' in proof:
        if nxsc_header is not None:
            raise ValueError('NXSC current-node proof requires its original raw counted record; native links cannot bypass the audited profile')
        link = proof['node_link']
        link_field, link_shape = 'evidence.node_link', _SAVE_NODE_LINK_SHAPE
        if isinstance(link, dict) and link.get('kind') == 'call_return':
            if set(link) == {'kind', 'position'}:
                link_field, link_shape = 'evidence.node_link.scope', _SAVE_TEXT_SHAPE
            elif (set(link) == {'kind', 'scope'}
                    or set(link) == {'kind', 'scope', 'position'}
                    and (not isinstance(link['position'], dict) or 'encoding' in link['position'])):
                link_field, link_shape = 'evidence.node_link.position', _SAVE_SCALAR_SHAPE
        selected = _save_check(
            lambda: ADAPTERS['resolve_native_node'](data, link, dialogue,
                span=lambda raw, item: _save_span(raw, item, 'evidence.node_link.scope'),
                integer=lambda raw, *args, **kwargs: _save_check(
                    lambda: integer(raw, *args, **kwargs), 'evidence.node_link.position', _SAVE_SCALAR_SHAPE),
                view=lambda raw, steps: _save_check(
                    lambda: view(raw, steps), 'evidence.node_link.position.view', _SAVE_VIEW_SHAPE)),
            link_field, link_shape)
        if (selected.get('source'), selected.get('position'), selected.get('text')) != (facts['source'], facts['line'], facts['anchors'][0]):
            raise ValueError('Saved native node link disagrees with the declared current source, position or caption')
        source, native = facts['source'], facts['line']
    else:
        source = _save_span(data, proof.get('source'), 'evidence.source')
        if source != facts['source'] and stem(source) != stem(facts['source']):
            raise SaveFieldError('Current caption needs the saved native script source matching facts.source',
                                 'evidence.source', _SAVE_TEXT_SHAPE + ' matching facts.source or its native same-stem alias')
        pointer = proof.get('position')
        if not isinstance(pointer, dict):
            raise SaveFieldError('Current caption needs native position byte evidence even when its text is unique',
                                 'evidence.position', _SAVE_SCALAR_SHAPE)
        raw = _save_check(lambda: view(data, pointer.get('view', [])), 'evidence.position.view', _SAVE_VIEW_SHAPE)
        native = _save_check(lambda: integer(raw, pointer.get('offset'), pointer.get('width', 4), pointer.get('endian', 'little')),
                             'evidence.position', _SAVE_SCALAR_SHAPE)
        if native != facts['line']:
            raise SaveFieldError('Current save position byte evidence disagrees with facts.line', 'evidence.position',
                                 _SAVE_SCALAR_SHAPE + ' equal to facts.line')
    matches = [r for r in dialogue if r['text'] == facts['anchors'][0]
               and r['position'] == native and r['source'] == facts['source']]
    if len(matches) != 1 or (matches[0]['source'], matches[0]['position']) != (facts.get('source'), facts.get('line')):
        counts = dict(exact=len(matches),
            source=sum(r['source'] == facts['source'] for r in dialogue),
            position=sum(r['position'] == native for r in dialogue),
            anchor=sum(r['text'] == facts['anchors'][0] for r in dialogue),
            source_position=sum(r['source'] == facts['source'] and r['position'] == native for r in dialogue),
            source_anchor=sum(r['source'] == facts['source'] and r['text'] == facts['anchors'][0] for r in dialogue),
            anchor_position=sum(r['text'] == facts['anchors'][0] and r['position'] == native for r in dialogue),
            native_source_alias=sum(isinstance(r['source'], str) and stem(r['source']) == stem(source) for r in dialogue))
        if counts['exact'] > 1:
            reason, fields = 'ambiguous_exact_node', ['facts.source', 'facts.line', 'facts.anchors[0]']
        elif not counts['source']:
            reason, fields = 'source_identity_absent', ['facts.source']
        elif not counts['source_position']:
            reason, fields = 'source_position_absent', ['facts.source', 'facts.line']
        elif not counts['exact']:
            reason, fields = 'anchor_mismatch_for_source_position', ['facts.anchors[0]']
        else:
            reason, fields = 'node_identity_disagreement', ['facts.source', 'facts.line']
        diagnostic = dict(category='save_node_join', reason=reason, fields=fields, counts=counts,
            facts_source_present=bool(counts['source']), native_source_alias_present=bool(counts['native_source_alias']),
            instruction='Return facts.source verbatim from the verified metadata["dialogue"] source field. '
                        'Its save byte proof may use the native same-stem alias. facts.line and anchors[0] must still '
                        'have exact byte evidence and match the same verified node; the host does not select a candidate.')
        raise ValueError('Current save node is ambiguous or disagrees with independent byte evidence; '
                         + json.dumps(diagnostic, ensure_ascii=False, separators=(',', ':')))
    if nxsc_header is not None:
        _verify_nxsc_current_node(data, facts, proof, dialogue, nxsc_header)
    if any(isinstance(record, dict) and (
            record.get('kind') == 'psb_message_reference' and record.get('message_reference_only') is True
            or record.get('role') in ('recorded_node', 'resume_node'))
            for record in matches[0].get('_native', [])):
        raise SaveFieldError('Message-reference or recorded-node observations cannot establish the current engine state',
                             'evidence', 'an independently supported current/choice/resume profile; omitting native declarations cannot upgrade PSB messages')
    history = []
    spans = proof.get('history', [])
    if not isinstance(spans, list) or len(spans) > 10000:
        raise SaveFieldError('Invalid save backlog evidence', 'evidence.history', 'an array of at most 10000 native TEXT spans')
    previous_end = -1
    for index, item in enumerate(spans):
        field = 'evidence.history[' + str(index) + ']'
        text = _save_span(data, item, field)
        if item['offset'] < previous_end:
            raise SaveFieldError('Backlog evidence must follow nonoverlapping save record order', field,
                                 'a native TEXT span after the preceding save record')
        previous_end = item['offset'] + item['length']
        candidates = [r for r in dialogue if r['text'] == text]
        if len(candidates) != 1:
            raise SaveFieldError('Backlog text does not uniquely identify an extracted node', field,
                                 'a native TEXT span identifying exactly one extracted node')
        row = candidates[0]
        history.append(dict(source=row['source'], position=row['position']))
    if history and history[-1] != dict(source=matches[0]['source'], position=matches[0]['position']):
        raise ValueError('Save backlog must end at the current node')
    facts['history'] = history
    return facts
