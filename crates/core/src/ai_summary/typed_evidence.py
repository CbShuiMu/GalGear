"""Bounded typed documents, independent of an adapter's parser or field claims.

The PSB wire layout follows the public FreeMote and GARbro readers:
https://github.com/UlyssesWu/FreeMote/blob/master/FreeMote.Psb/Psb.cs
https://github.com/UlyssesWu/FreeMote/blob/master/FreeMote.Psb/PsbValues.cs
https://github.com/morkt/GARbro/blob/master/ArcFormats/Emote/ArcPSB.cs
This is a data reader, not a PSB virtual machine or a current-state oracle.
"""
import copy
import functools
import hashlib
import pathlib
import runpy
import struct
import json


_BASE = pathlib.Path(__file__).parent
_LITERAL = runpy.run_path(str(_BASE / 'literal_evidence.py'))
_CODECS = runpy.run_path(str(_BASE / 'bytecodecs.py'))
MAX_BYTES = 32 * 1024 * 1024
MAX_NODES = 500_000
MAX_DEPTH = 64
MAX_PATH = 64
MAX_STRING = 1_048_576
CONTRACT = dict(
    member_filter='Archived-script output may declare top-level member_filter:[standard byte-view steps]. '
                  'The host restores the ENTIRE original archive member, independently checks the native '
                  'archive checksum and byte length, then applies any internal output.view. '
                  'PSB typed proofs use that complete recovered member and an empty document.view. '
                  'A parsed PSB header/root alone does not attest an archive filter.',
    document='PSB: {syntax:"psb",view:[]} or serialized literal '
             '{syntax:"tjs_const",encoding:"utf-16-le"|"utf-8"|"ascii"|"utf-16-be",view:[]}. '
             'PSB versions 2 and 3 must already be decrypted; encrypted or unknown versions fail closed. '
             'The host walks the complete root, names, string and resource tables, not a selected string pool.',
    field=dict(kind='typed_field', document='<document>', path=['<object key or array index>'],
               type='string|integer|boolean|null|void|array|object'),
    identity='A PSB string field is identified by its typed reference node in the object tree, '
             'not by its string-pool bytes. span.offset is the native serialized reference offset. '
             'Integer fields preserve signed wire values; coordinate callers must require uint64 separately.',
    boundary='A complete typed tree proves structure and original field values. It does not prove '
             'which field is current, whether a point is a line, or that a history/preview is a current node.')
CONTRACT['psb_layers'] = dict(
    document='{syntax:"psb",view:[],frame?:{codec:"mdf"},crypto?:{codec:"psb_decrypt",key:<explicit u32>}}',
    order='Complete XP3 member receipt first, then fixed ordinary MDF frame, then fixed native PSB crypto, then entire typed root.',
    references='Positions remain native reference-node offsets in the complete restored PSB binary. No JSON rebuild, slice or patch.',
    boundary='An archived input using frame/crypto requires independent native outer-member authentication first; a wrong outer checksum cannot be moved into document.crypto. '
             'An XP3 checksum may cover internally encrypted PSB bytes. Internal crypto is not forced to match the outer checksum again. '
             'Native key is model-supplied from current evidence, never a preset; typed recorded_node remains diagnostic, not current/resume.')
CONTRACT['script_record'] = dict(kind='typed_record', record=['<complete record path>'],
    scope=['<absolute scope field path within that record>'], coordinate=['<absolute integer field path>'],
    text=['<same path as the row typed_field>'], speaker=['<optional exact name/null field path>'])
CONTRACT['saved_record'] = dict(kind='typed_record', document='<typed document schema>',
    record=['<complete saved record path>'], scope=['<relative scope field path>'],
    coordinate=['<relative integer field path>'], caption=['<relative caption field path>'], state_role='recorded_node')
CONTRACT['record_roles'] = ('typed_record joins prove one original scope/coordinate/text tuple and preserve '
    'the explicit recorded_node role. This is a diagnostic relationship, not current coverage. '
    'No model role=current flag or a matching historical tuple can promote it to a current node. '
    'A separately supported engine state profile must establish current/choice/resume semantics.')


def _fail(reason):
    raise ValueError('Typed document evidence ' + reason)


def _path(path):
    if (not isinstance(path, list) or len(path) > MAX_PATH
            or any(not (type(part) is int and 0 <= part <= 2**32 - 1
                        or isinstance(part, str) and len(part.encode()) <= 4096) for part in path)):
        _fail('needs a bounded path of object keys and integer array indices')
    return path


def select(root, path):
    """Select a typed node; never coerce a string index or boolean to an ordinal."""
    node = root
    for part in _path(path):
        if node['type'] == 'object' and isinstance(part, str) and part in node['value']:
            node = node['value'][part]
        elif node['type'] == 'array' and type(part) is int and part < len(node['value']):
            node = node['value'][part]
        else:
            _fail('path does not select an existing typed node')
    return node


def plain(node):
    """Return data only, keeping node identity available in the original tree."""
    if node['type'] == 'object':
        return {key: plain(value) for key, value in node['value'].items()}
    if node['type'] == 'array':
        return [plain(value) for value in node['value']]
    return node['value']


class _Psb:
    def __init__(self, data):
        self.data, self.nodes, self.cache, self.ranges, self.weights = data, 0, {}, {}, {}
        if len(data) < 40 or data[:4] != b'PSB\0':
            _fail('needs a complete native PSB header')
        self.version, flags = struct.unpack_from('<HH', data, 4)
        if self.version not in (2, 3) or flags != 0:
            _fail('supports only decrypted PSB versions 2 and 3 with zero flags')
        minimum = 40 if self.version == 2 else 44
        if len(data) < minimum:
            _fail('has a truncated versioned PSB header')
        header = struct.unpack_from('<8I', data, 8)
        header_length, names, strings, string_data, chunks, lengths, chunk_data, root = header
        if header_length != minimum:
            _fail('has an unsupported PSB header length')
        points = (names, strings, string_data, chunks, lengths, chunk_data, root)
        if (any(not minimum <= point <= len(data) for point in points)
                or any(point == len(data) for point in (names, strings, chunks, lengths, root))
                or len({names, strings, chunks, lengths, root}) != 5):
            _fail('has invalid or overlapping PSB section offsets')
        # Empty string/resource payloads may share their endpoint with a table.
        self.boundaries = sorted(set(points) | {len(data)})
        self.root_offset, self.root_end = root, self.end(root)
        self.claim(0, minimum)
        first, position = self.packed(names, self.end(names))
        parents, position = self.packed(position, self.end(names))
        terminals, position = self.packed(position, self.end(names))
        if position != self.end(names):
            _fail('PSB name-table walk does not end at its complete section boundary')
        self.names = self.names_for(first, parents, terminals)
        offsets, position = self.packed(strings, self.end(strings))
        if position != self.end(strings):
            _fail('PSB string-offset table has trailing or missing bytes')
        self.strings, decoded_strings, string_bytes = [], {}, 0
        for relative in offsets:
            start = string_data + relative
            if start in decoded_strings:
                self.strings.append(decoded_strings[start])
                continue
            end = self.data.find(b'\0', start, min(self.end(string_data), start + MAX_STRING + 1))
            if start < string_data or start >= self.end(string_data) or end < start:
                _fail('PSB string offset or terminator is outside its complete pool')
            try:
                text = data[start:end].decode('utf-8', 'strict')
            except UnicodeError as error:
                raise ValueError('Typed document evidence PSB string pool has invalid UTF-8') from error
            string_bytes += end-start
            if string_bytes > MAX_BYTES:
                _fail('PSB string suffix references exceed their total decoded byte budget')
            decoded_strings[start] = (text, dict(offset=start, length=end-start, encoding='utf-8'))
            self.strings.append(decoded_strings[start])
        self.chunk_offsets, position = self.packed(chunks, self.end(chunks))
        if position != self.end(chunks):
            _fail('PSB chunk-offset table has trailing or missing bytes')
        self.chunk_lengths, position = self.packed(lengths, self.end(lengths))
        if len(self.chunk_lengths) != len(self.chunk_offsets):
            _fail('PSB resource tables disagree or have incomplete boundaries')
        self.finish_chunk_lengths(position, self.end(lengths), chunk_data)
        self.resources = []
        for offset, length in zip(self.chunk_offsets, self.chunk_lengths):
            start = chunk_data + offset
            if start < chunk_data or length > len(data) - start:
                _fail('PSB resource exceeds its native payload boundary')
            self.resources.append(dict(offset=start, length=length,
                                       sha256=hashlib.sha256(data[start:start+length]).hexdigest()))

    def end(self, offset):
        return next((point for point in self.boundaries if point > offset), len(self.data))

    def take(self, offset, size, end):
        if not 0 <= offset <= end <= len(self.data) or not 0 <= size <= end - offset:
            _fail('PSB field exceeds its complete section boundary')
        return self.data[offset:offset+size]

    def claim(self, offset, size):
        previous = self.ranges.get(offset)
        if previous is not None and previous != size:
            _fail('PSB fields reuse one offset with conflicting types or lengths')
        self.ranges[offset] = size

    def finish_chunk_lengths(self, position, end, chunk_data):
        if position == end:
            return
        # FreeMote's writer calls DataAlign(..., 16) immediately after the
        # complete ChunkLengths array and before OffsetChunkData. This is a
        # specific native padding rule, never arbitrary trailing-byte skipping.
        padding = end-position
        if (end != chunk_data or not 0 < padding <= 15 or end != (position+15)//16*16
                or self.take(position, padding, end) != b'\0'*padding):
            _fail('PSB chunk-length table has nonzero, excessive or unaligned trailing bytes')
        self.claim(position, padding)

    def packed(self, offset, end):
        start = offset
        count_width = self.take(offset, 1, end)[0] - 12
        if not 1 <= count_width <= 8:
            _fail('PSB packed array needs a native count-width tag')
        offset += 1
        count = int.from_bytes(self.take(offset, count_width, end), 'little')
        offset += count_width
        width = self.take(offset, 1, end)[0] - 12
        offset += 1
        # The wire width tag is NumberN8 + EntryLength. EntryLength=0 is
        # valid: an empty table has no payload, and a bounded nonempty zero-
        # width array represents count zero values in the public PSB reader.
        if not 0 <= width <= 8 or count > MAX_NODES or count * width > end - offset:
            _fail('PSB packed array count, element width or complete bounds are invalid')
        values = [int.from_bytes(self.take(offset + index * width, width, end), 'little')
                  for index in range(count)]
        position = offset + count * width
        self.claim(start, position-start)
        return values, position

    def names_for(self, bases, parents, terminals):
        result, work = [], 0
        if len(bases) != len(parents):
            _fail('PSB name trie arrays have different sizes')
        for terminal in terminals:
            if terminal >= len(parents):
                _fail('PSB name terminal is outside its complete trie')
            cursor, seen, characters = parents[terminal], set(), []
            if cursor >= len(bases) or bases[cursor] != terminal:
                _fail('PSB name terminal is not its native zero-byte trie edge')
            while cursor:
                work += 1
                if work > MAX_NODES * 8:
                    _fail('PSB name trie exceeds its total edge traversal budget')
                if cursor >= len(parents) or cursor in seen or len(seen) >= 4096:
                    _fail('PSB name trie is cyclic, excessive or out of bounds')
                seen.add(cursor)
                parent = parents[cursor]
                if parent >= len(bases):
                    _fail('PSB name parent is outside its complete trie')
                character = cursor - bases[parent]
                if not 1 <= character <= 255:
                    _fail('PSB name trie has an invalid byte edge')
                characters.append(character)
                cursor = parent
            try:
                result.append(bytes(reversed(characters)).decode('utf-8', 'strict'))
            except UnicodeError as error:
                raise ValueError('Typed document evidence PSB name trie contains invalid UTF-8') from error
        if len(set(result)) != len(result):
            _fail('PSB name table contains duplicate decoded names')
        return result

    def node(self, offset, depth=0):
        if offset in self.cache:
            return self.cache[offset]
        if depth > MAX_DEPTH or self.nodes >= MAX_NODES:
            _fail('PSB root exceeds its node/depth budget')
        if not self.root_offset <= offset < self.root_end:
            _fail('PSB value offset points outside its complete root section')
        self.nodes += 1
        tag = self.take(offset, 1, self.root_end)[0]
        size, extra, weight = 1, {}, 1
        if tag == 1:
            kind, value = 'null', None
        elif tag in (2, 3):
            # Native PsbObjType: False=0x02, True=0x03. Keep independent
            # CLI-generated fixtures; old readers disagree on these tags.
            kind, value = 'boolean', tag == 3
        elif 4 <= tag <= 12:
            size = 1 + tag - 4
            kind = 'integer'
            value = int.from_bytes(self.take(offset+1, size-1, self.root_end), 'little', signed=True)
        elif 21 <= tag <= 24:
            size = 1 + tag - 20
            index = int.from_bytes(self.take(offset+1, size-1, self.root_end), 'little')
            if index >= len(self.strings):
                _fail('PSB string reference exceeds its complete string table')
            kind, (value, proof) = 'string', self.strings[index]
            extra = dict(string_index=index, string_span=proof)
        elif 25 <= tag <= 28:
            size = 1 + tag - 24
            index = int.from_bytes(self.take(offset+1, size-1, self.root_end), 'little')
            if index >= len(self.resources):
                _fail('PSB resource reference exceeds its complete resource tables')
            kind, value = 'resource', self.resources[index]
        elif tag in (29, 30, 31):
            size = {29: 1, 30: 5, 31: 9}[tag]
            payload = self.take(offset+1, size-1, self.root_end)
            kind, value = 'real', 0.0 if tag == 29 else struct.unpack('<f' if tag == 30 else '<d', payload)[0]
            if not (-float('inf') < value < float('inf')):
                _fail('PSB nonfinite numbers are unsupported evidence')
        elif tag in (32, 33):
            first, cursor = self.packed(offset+1, self.root_end)
            if tag == 33:
                children, cursor = self.packed(cursor, self.root_end)
                if len(children) != len(first) or any(index >= len(self.names) for index in first):
                    _fail('PSB object name and child arrays disagree')
                keys = [self.names[index] for index in first]
                if len(set(keys)) != len(keys):
                    _fail('PSB object contains duplicate decoded keys')
            else:
                children = first
            size = cursor-offset
            # Native unsigned offsets are relative to the end of the complete
            # child-array headers. They cannot point backward into their header.
            values = []
            for relative in children:
                child = self.node(cursor+relative, depth+1)
                weight += self.weights[cursor+relative]
                if weight > MAX_NODES:
                    _fail('PSB shared subtree references exceed their expanded node budget')
                values.append(child)
            kind, value = ('object', dict(zip(keys, values))) if tag == 33 else ('array', values)
        else:
            _fail('contains an unsupported PSB value tag ' + str(tag))
        self.take(offset, size, self.root_end)
        # Collection headers include packed arrays that are separately claimed.
        # Only claim the tag there, so a child may not overlap any header bytes.
        self.claim(offset, 1 if tag in (32, 33) else size)
        node = dict(type=kind, value=value, span=dict(offset=offset, length=size), **extra)
        self.cache[offset] = node
        self.weights[offset] = weight
        return node

    def parse(self):
        root = self.node(self.root_offset)
        if root['type'] not in ('object', 'array'):
            _fail('PSB root must be a complete object or array')
        end = 0
        for offset, size in sorted(self.ranges.items()):
            if offset < end:
                _fail('PSB typed fields or section headers overlap')
            end = offset+size
        return root


@functools.lru_cache(maxsize=2)
def _parse_psb(raw):
    return _Psb(raw).parse()


def _parse(data, schema):
    if not isinstance(schema, dict) or not isinstance(data, bytes):
        _fail('needs a document schema and original bytes')
    if schema.get('syntax') == 'tjs_const':
        # The literal module caches token parsing; this additional cache avoids
        # rebuilding the complete public tree for every selected dialogue row.
        result = _parse_literal(data, json.dumps(schema, sort_keys=True, separators=(',', ':')))
        return result['root'], result['sha256'], result['bytes']
    if set(schema) - {'syntax', 'view', 'frame', 'crypto'} or schema.get('syntax') != 'psb':
        _fail('has an unknown syntax or unsupported document fields')
    if 'view' in schema and (not isinstance(schema['view'], list) or schema['view']):
        _fail('PSB documents require a complete input and empty view; use only fixed frame/crypto layers')
    raw = data
    if 'frame' in schema:
        if schema['frame'] != {'codec': 'mdf'}:
            _fail('PSB frame must be exactly {codec:"mdf"}; offsets, seeds and unknown shells are unsupported')
        raw = _CODECS['decode']('mdf', raw)
    if 'crypto' in schema:
        crypto = schema['crypto']
        if not isinstance(crypto, dict) or set(crypto) != {'codec', 'key'} or crypto['codec'] != 'psb_decrypt':
            _fail('PSB crypto needs exactly codec:"psb_decrypt" and explicit unsigned32 key')
        raw = _CODECS['decode']('psb_decrypt', raw, key=crypto['key'])
    if not 0 < len(raw) <= MAX_BYTES:
        _fail('exceeds its complete document byte budget')
    return _parse_psb(raw), hashlib.sha256(raw).hexdigest(), len(raw)


@functools.lru_cache(maxsize=2)
def _parse_literal(data, schema_json):
    return _LITERAL['parse_document'](data, json.loads(schema_json))


def parse_document(data, schema):
    root, digest, size = _parse(data, schema)
    return dict(root=copy.deepcopy(root), document=copy.deepcopy(schema), sha256=digest, bytes=size)


def replay_field(data, proof):
    if (not isinstance(proof, dict) or set(proof) != {'kind', 'document', 'path', 'type'}
            or proof.get('kind') != 'typed_field'):
        _fail('field needs kind, document, path and type; arbitrary byte offsets are not selectors')
    root, digest, size = _parse(data, proof['document'])
    node = select(root, proof['path'])
    if proof['type'] not in ('string', 'integer', 'boolean', 'null', 'void', 'array', 'object') or node['type'] != proof['type']:
        _fail('selected field does not have its declared native type')
    return dict(type=node['type'], value=plain(node), span=copy.deepcopy(node['span']),
                node=copy.deepcopy(node), path=copy.deepcopy(proof['path']),
                document=copy.deepcopy(proof['document']), document_sha256=digest)


def _record_path(path, root_path, label):
    _path(path)
    if len(path) <= len(root_path) or path[:len(root_path)] != root_path:
        _fail(label + ' must select a field inside the same complete native record')
    return path


def attest_record(data, row, native):
    """Bind a row to a complete native tuple, without inventing its state role."""
    required = {'kind', 'record', 'scope', 'coordinate', 'text'}
    if (not isinstance(native, dict) or not required <= native.keys()
            or set(native) - required - {'speaker'} or native.get('kind') != 'typed_record'):
        _fail('native record needs kind, record, scope, coordinate and text paths')
    proof = row.get('evidence')
    if not isinstance(proof, dict) or proof.get('kind') != 'typed_field':
        _fail('native record requires an independently verified typed_field row')
    root, digest, _ = _parse(data, proof['document'])
    record_path = _path(native['record'])
    record = select(root, record_path)
    if record['type'] not in ('object', 'array'):
        _fail('native record selector must identify a complete object or tuple')
    nodes = {key: select(root, _record_path(native[key], record_path, key))
             for key in ('scope', 'coordinate', 'text')}
    if (nodes['scope']['type'] != 'string' or not nodes['scope']['value']
            or len(nodes['scope']['value'].encode()) > 4096
            or nodes['coordinate']['type'] != 'integer'
            or not 0 <= nodes['coordinate']['value'] <= 2**64-1
            or nodes['text']['type'] != 'string' or nodes['text']['value'] != row.get('text')
            or native['text'] != proof.get('path')
            or nodes['text']['span']['offset'] != row.get('position')):
        _fail('native record needs exact original scope/integer/text fields and the same row reference')
    speaker_status = 'unresolved'
    if 'speaker' in native:
        speaker = select(root, _record_path(native['speaker'], record_path, 'speaker'))
        value = speaker['value'] if speaker['type'] == 'string' and speaker['value'] else None
        if speaker['type'] not in ('string', 'null', 'void') or value != row.get('speaker'):
            _fail('native record speaker must equal its actual name or explicit empty/null narration field')
        speaker_status = 'named' if value else 'narration'
    elif row.get('speaker') is not None:
        _fail('native record must bind its nonempty displayed speaker to the same record')
    return dict(version=1, kind='typed_record', scope=nodes['scope']['value'],
                value=nodes['coordinate']['value'], position=row['position'], role='recorded_node',
                speaker_status=speaker_status, script_sha256=hashlib.sha256(data).hexdigest(),
                document_sha256=digest,
                schema_sha256=hashlib.sha256(json.dumps(native, sort_keys=True, ensure_ascii=False,
                                            separators=(',', ':')).encode()).hexdigest())


def resolve_record(data, link, dialogue):
    """Replay saved tuple fields and join structurally, retaining recorded role."""
    if (not isinstance(link, dict) or set(link) != {'kind', 'document', 'record', 'scope', 'coordinate', 'caption', 'state_role'}
            or link.get('kind') != 'typed_record' or link.get('state_role') != 'recorded_node'):
        _fail('saved typed record requires explicit recorded_node role and bounded typed field selectors; '
              'current, resume, last_displayed or unknown cannot be inferred from a matching caption')
    root, digest, _ = _parse(data, link['document'])
    path = _path(link['record'])
    if select(root, path)['type'] not in ('object', 'array'):
        _fail('saved record must select a complete original object or tuple')
    fields = {key: select(root, _record_path(path+_path(link[key]), path, key))
              for key in ('scope', 'coordinate', 'caption')}
    if (fields['scope']['type'] != 'string' or fields['coordinate']['type'] != 'integer'
            or not 0 <= fields['coordinate']['value'] <= 2**64-1
            or fields['caption']['type'] != 'string'):
        _fail('saved record needs exact string scope/caption and a nonnegative native integer coordinate')
    matches = []
    for row in dialogue:
        if row.get('text') != fields['caption']['value']:
            continue
        for native in row.get('_native', []):
            if (isinstance(native, dict) and native.get('version') == 1 and native.get('kind') == 'typed_record'
                    and native.get('position') == row.get('position')
                    and native.get('role') == 'recorded_node'
                    and native.get('scope') == fields['scope']['value']
                    and native.get('value') == fields['coordinate']['value']):
                matches.append(row)
                break
    if len(matches) != 1:
        _fail('saved typed scope/coordinate/caption must identify exactly one host-attested record; matches=' + str(len(matches)))
    return dict(row=matches[0], state_role='recorded_node', current_verified=False,
                document_sha256=digest,
                instruction='The original tuple matches structurally. Establish engine state-role semantics separately; '
                            'do not promote history, last displayed text or a resume marker to a current message.')
