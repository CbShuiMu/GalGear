"""Read-only native XP3 member checksums, never model-supplied checksums.

The bounded index layout follows GARbro ArcXP3.cs at
b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0. Fixed native info/segment/checksum
fields remain strict for every issued receipt. Complete entries with disagreeing
info/segment lengths remain quarantined identity candidates; they cannot issue a
receipt or make a different member look uniquely identified. Unknown complete
name tails remain opaque; no name or encoding is guessed.
Checksum agreement binds a whole member, not dialogue or engine semantics.
"""
import hashlib
import pathlib
import struct
import zlib


MAGIC = b'XP3\r\n \n\x1a\x8bg\x01'
MAX_INDEX = 64 * 1024 * 1024
MAX_MEMBERS = 100_000
MAX_SEGMENTS = 4096


def _fail(message):
    raise ValueError('Native archive evidence ' + message)


def _inflate(data, expected):
    if not 0 <= expected <= MAX_INDEX:
        _fail('index exceeds the bounded limit')
    inflater = zlib.decompressobj()
    value = inflater.decompress(data, expected + 1)
    if (len(value) != expected or not inflater.eof or inflater.unused_data
            or inflater.unconsumed_tail):
        _fail('compressed index is incomplete or has trailing bytes')
    return value


def _chunks(data, start=0, end=None):
    end = len(data) if end is None else end
    offset = start
    while offset < end:
        if end - offset < 12:
            _fail('has a truncated native chunk header')
        tag, length = data[offset:offset + 4], struct.unpack_from('<Q', data, offset + 4)[0]
        begin, finish = offset + 12, offset + 12 + length
        if finish > end:
            _fail('chunk exceeds its complete native boundary')
        yield tag, begin, finish
        offset = finish


def read_xp3_index(archive):
    """Walk complete File records without decoding unknown native name tails."""
    archive = pathlib.Path(archive)
    before = archive.stat()
    with archive.open('rb') as handle:
        def read(offset, length):
            if not 0 <= offset <= before.st_size or not 0 <= length <= MAX_INDEX or offset + length > before.st_size:
                _fail('index read exceeds the actual archive')
            handle.seek(offset)
            result = handle.read(length)
            if len(result) != length:
                _fail('archive changed or is truncated')
            return result

        header = read(0, 19)
        if header[:11] != MAGIC:
            _fail('requires a standard XP3 file; embedded/custom indices are unsupported')
        offset = struct.unpack_from('<Q', header, 11)[0]
        if offset < 19:
            _fail('has an invalid native index offset')
        # Standard XP3 version-2 indirection is not a data decoder or key.
        if read(offset, 4) == b'\x80\x00\x00\x00':
            offset = struct.unpack('<Q', read(offset + 9, 8))[0]
        kind = read(offset, 1)[0]
        if kind == 0:
            length = struct.unpack('<Q', read(offset + 1, 8))[0]
            index = read(offset + 9, length)
        elif kind == 1:
            packed, unpacked = struct.unpack('<QQ', read(offset + 1, 16))
            index = _inflate(read(offset + 17, packed), unpacked)
        else:
            _fail('requires a complete supported single native index')
    if (before.st_size, before.st_mtime_ns) != (archive.stat().st_size, archive.stat().st_mtime_ns):
        _fail('archive changed while reading the index')
    members, entries = {}, []
    index_sha256 = hashlib.sha256(index).hexdigest()
    for tag, begin, finish in _chunks(index):
        if tag != b'File':
            continue
        fields = {}
        for section, start, stop in _chunks(index, begin, finish):
            if section in (b'info', b'segm', b'adlr'):
                if section in fields:
                    _fail('contains duplicate native member metadata')
                fields[section] = index[start:stop]
        if set(fields) != {b'info', b'segm', b'adlr'}:
            _fail('member is missing complete native info/segments/checksum')
        info, segment_data, checksum = fields[b'info'], fields[b'segm'], fields[b'adlr']
        if len(info) < 20 or len(checksum) != 4:
            _fail('has malformed member info/checksum')
        flags, unpacked, packed = struct.unpack_from('<IQQ', info)
        name = None
        name_tail = info[20:]
        if len(name_tail) >= 2:
            count = struct.unpack_from('<H', name_tail)[0]
            if count and len(name_tail) == 2 + count * 2:
                try:
                    name = name_tail[2:].decode('utf-16-le', 'strict').replace('\\', '/')
                except UnicodeDecodeError:
                    pass
        if name is not None and (name.startswith('/') or '\x00' in name or '..' in name.split('/')):
            _fail('member path is not a safe relative native name')
        if not segment_data or len(segment_data) % 28 or len(segment_data) // 28 > MAX_SEGMENTS:
            _fail('has an incomplete or unbounded segment table')
        segments = []
        for position in range(0, len(segment_data), 28):
            compressed, source, size, stored = struct.unpack_from('<IQQQ', segment_data, position)
            if (compressed not in (0, 1) or source < 19 or source + stored > before.st_size
                    or not compressed and size != stored):
                _fail('segment is outside the archive or has inconsistent lengths')
            segments.append(dict(compressed=bool(compressed), offset=source, size=size, packed=stored))
        segment_size = sum(item['size'] for item in segments)
        segment_packed = sum(item['packed'] for item in segments)
        entry_id = hashlib.sha256(bytes.fromhex(index_sha256) + struct.pack('<Q', begin)
                                  + index[begin:finish]).hexdigest()
        metadata = dict(entry_id=entry_id, index_record_offset=begin, index_record_length=finish-begin,
                        name=name, name_kind='counted_utf16le' if name is not None else 'opaque_native_tail',
                        name_tail_hex=name_tail.hex(), flags=flags, size=unpacked, packed=packed,
                        segment_size=segment_size, segment_packed=segment_packed,
                        size_consistent=segment_size == unpacked and segment_packed == packed,
                        adler32=struct.unpack('<I', checksum)[0], segments=segments)
        if name is not None:
            identity = name.casefold()
            if identity in members:
                _fail('contains an ambiguous duplicate native member name')
            members[identity] = metadata
        entries.append(metadata)
        if len(entries) > MAX_MEMBERS:
            _fail('contains too many native members')
    if not entries:
        _fail('contains no supported native members')
    return dict(archive=str(archive.resolve()), size=before.st_size, modified_ns=before.st_mtime_ns,
                index_sha256=index_sha256, members=members, entries=entries)


def _raw_member(archive, metadata):
    """Read all native segments for identity, including quarantined candidates."""
    if metadata['segment_size'] > 32 * 1024 * 1024:
        _fail('raw member exceeds the bounded script limit')
    pieces = []
    with pathlib.Path(archive).open('rb') as handle:
        for segment in metadata['segments']:
            if segment['packed'] > MAX_INDEX:
                _fail('packed member segment exceeds the bounded input limit')
            handle.seek(segment['offset'])
            data = handle.read(segment['packed'])
            if len(data) != segment['packed']:
                _fail('native segment is truncated')
            if segment['compressed']:
                data = _inflate(data, segment['size'])
            if len(data) != segment['size']:
                _fail('native segment output length disagrees with its index')
            pieces.append(data)
    value = b''.join(pieces)
    if len(value) != metadata['segment_size']:
        _fail('complete raw member length disagrees with its native segment table')
    return value


def identify_member(archive, member, original=None):
    """Bind a readable extraction to an actual native member, never by size alone."""
    if not isinstance(member, str):
        _fail('needs an actual extracted member identity')
    index = read_xp3_index(archive)
    metadata = index['members'].get(member.replace('\\', '/').casefold())
    identity = 'exact_native_name'
    if original is not None and not isinstance(original, bytes):
        _fail('raw member identity requires the exact extracted input bytes')
    if metadata is not None and original is not None and _raw_member(archive, metadata) != original:
        _fail('extracted input differs from its complete native segments; an unverified earlier filter is unsupported')
    if metadata is None and original is not None:
        matches = []
        candidates = [candidate for candidate in index['entries'] if candidate['segment_size'] == len(original)]
        if len(candidates) > 1000 or sum(candidate['segment_packed'] for candidate in candidates) > 256 * 1024 * 1024:
            _fail('mapped-name identity exceeds the bounded raw candidate I/O limit; a verified native name mapping is required')
        for candidate in candidates:
            # Size only bounds the I/O. Equality of the complete decompressed
            # raw member establishes the identity; size/checksum alone cannot.
            # Quarantined entries participate so they cannot hide ambiguity.
            if _raw_member(archive, candidate) == original:
                matches.append(candidate)
                if len(matches) > 1:
                    _fail('complete raw bytes match multiple native members; mapped-name identity is ambiguous')
        if len(matches) == 1:
            metadata, identity = matches[0], 'unique_complete_native_segment_bytes'
    if metadata is None:
        _fail('cannot associate this extracted member with an exact native index name')
    if not metadata['size_consistent']:
        _fail('matched member is quarantined: info lengths disagree with its complete segment table; unsupported member cannot issue a receipt')
    after = pathlib.Path(archive).stat()
    if (after.st_size, after.st_mtime_ns) != (index['size'], index['modified_ns']):
        _fail('archive changed during whole-member identity replay')
    return dict(archive=index['archive'], index_sha256=index['index_sha256'], member_identity=identity,
                native=metadata, original_sha256=hashlib.sha256(original).hexdigest() if original is not None else None,
                original_member_bound=original is not None)


def native_member_metadata(archive, member, original):
    """Observe native declarations only after complete extracted-byte binding.

    No model checksum, transform, decoder parameter or restored output is an
    input. An original name alone is insufficient for this optional observation;
    already-decrypted tool output may legitimately be ineligible. This never
    replaces verify_member or attests an engine current/resume role.
    """
    if not isinstance(original, bytes):
        _fail('per-input native metadata requires the complete actual extracted bytes')
    identified = identify_member(archive, member, original=original)
    metadata = identified['native']
    return dict(format='xp3', adler32=metadata['adler32'], size=metadata['size'],
                packed=metadata['packed'], flags=metadata['flags'], entry_id=metadata['entry_id'],
                index_sha256=identified['index_sha256'], scope='native_declaration_only')


def verify_member(archive, member, restored, original=None):
    """Require exact full-member length and checksum from the actual index."""
    if not isinstance(restored, bytes):
        _fail('requires restored bytes')
    identified = identify_member(archive, member, original)
    metadata = identified['native']
    if len(restored) != metadata['size'] or zlib.adler32(restored) & 0xffffffff != metadata['adler32']:
        _fail('whole restored member does not match its native size and Adler-32; partial readable fields are insufficient')
    return dict(archive=identified['archive'], index_sha256=identified['index_sha256'], member_identity=identified['member_identity'],
                member=metadata['name'], entry_id=metadata['entry_id'], bytes=len(restored), adler32=metadata['adler32'],
                restored_sha256=hashlib.sha256(restored).hexdigest(), verified=True,
                limitation='Checksum binding verifies complete bytes, not the role/current-node semantics or an extraction filter for other inputs.')


def requires_document_member_verification(output, outer_member=None):
    """New fixed PSB layers in proof objects need an authenticated outer member.

    Walk only proof-bearing containers, not arbitrary user/resource values.
    This includes script text, inline speakers, companion bindings and node links.
    """
    pending = [output]
    has_view = False
    while pending:
        value = pending.pop()
        if isinstance(value, list):
            pending.extend(value)
        elif isinstance(value, dict):
            for name in ('view', 'pool_view', 'member_filter'):
                steps = value.get(name)
                has_view = has_view or isinstance(steps, list) and bool(steps)
                if isinstance(steps, list) and any(isinstance(step, dict) and step.get('op') in ('mdf', 'psb_decrypt') for step in steps):
                    return True
            document = value.get('document')
            if (value.get('kind') in ('typed_field', 'typed_record')
                    and isinstance(document, dict) and document.get('syntax') == 'psb'
                    and ('frame' in document or 'crypto' in document)):
                return True
            for name in ('evidence', 'speaker', 'node_link', 'rows', 'history', 'anchor', 'source', 'position', 'scope', 'reference', 'document', 'id_evidence'):
                child = value.get(name)
                if isinstance(child, (dict, list)):
                    pending.append(child)
            speakers = value.get('speakers')
            if isinstance(speakers, dict):
                pending.extend(speakers.values())
            tables = value.get('tables')
            if isinstance(tables, dict):
                pending.extend(tables.values())
    # Equivalent manual slice/zlib syntax cannot hide a declared MDF frame.
    return has_view and isinstance(outer_member, bytes) and outer_member[:4] == b'mdf\0'


def verify_document_member(archive, member, original, outer_member, output):
    """Authenticate the complete outer member before optional internal layers.

    outer_member is the original raw input or a separately restored XP3 member.
    It is never the PSB document.crypto/frame output. No expected checksum is
    accepted from an adapter, and an existing receipt cannot be supplied here.
    """
    if not requires_document_member_verification(output, outer_member):
        return None
    if not isinstance(original, bytes) or not isinstance(outer_member, bytes):
        _fail('document layers require complete original and outer-member bytes')
    if archive is None:
        if original != outer_member:
            _fail('loose input has no archive receipt for an invented outer transformation')
        receipt = None
    else:
        receipt = verify_member(archive, member, outer_member, original=original)
    if outer_member[:4] == b'mdf\0':
        if len(outer_member) < 8 or len(outer_member) > 32 * 1024 * 1024:
            _fail('ordinary MDF frame exceeds its complete input budget')
        size = struct.unpack_from('<I', outer_member, 4)[0]
        if size > 32 * 1024 * 1024:
            _fail('ordinary MDF output exceeds its byte budget')
        _inflate(outer_member[8:], size)
    return dict(receipt, stage='outer_member_before_document_layers') if receipt is not None else None
