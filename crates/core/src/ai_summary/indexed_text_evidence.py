"""Replay offset/length records and ordinal-derived transforms from raw bytes.

This proves table contents and ordinals, not message-consumer or save semantics.
All layout and transform parameters must be explicitly supplied as evidence.
"""

MAX_ROWS = 500_000
MAX_DECODED_BYTES = 64 * 1024 * 1024
WIDTHS = (1, 2, 3, 4, 8)
ENCODINGS = {'utf-8', 'utf-16-le', 'utf-16-be', 'cp932', 'gb18030', 'ascii'}
CONTRACT = dict(
    kind='offset_length_array', offset='<first native index record>',
    length='<count * record_size>', count=dict(offset='<native count field>', width=4, endian='little'),
    record_size='<native record stride, 2..64 bytes>',
    offset_field=dict(offset='<relative to each record>', width=4, endian='little', unit=1),
    length_field=dict(offset='<relative to each record>', width=4, endian='little', unit=1),
    base='<raw string pool start>', pool_length='<bounded raw pool length>', encoding='<strict encoding>',
    entry_xor='Optional {width:1|2|4|8,multiplier:<unsigned>,addend:<unsigned>,endian:little|big}. '
              'Key = (ordinal * multiplier + addend) modulo 2^(8*width); repeat key bytes over this entry only. '
              'All empty/unreferenced records retain their native ordinals. Parameters are observations, never supplied defaults.')


def _field(value, stride):
    if not isinstance(value, dict) or set(value) - {'offset', 'width', 'endian', 'unit'}:
        raise ValueError('Index field needs offset, width and optional endian/unit')
    offset, width = value.get('offset'), value.get('width')
    endian, unit = value.get('endian', 'little'), value.get('unit', 1)
    if (type(offset) is not int or type(width) is not int or width not in WIDTHS
            or not 0 <= offset <= stride - width or endian not in ('little', 'big')
            or type(unit) is not int or not 1 <= unit <= 64):
        raise ValueError('Index field exceeds its record or has invalid width/endian/unit')
    return offset, width, endian, unit


def _xor(spec):
    if spec is None:
        return None
    if not isinstance(spec, dict) or set(spec) != {'width', 'multiplier', 'addend', 'endian'}:
        raise ValueError('Ordinal XOR requires exactly width, multiplier, addend and endian')
    width, multiplier, addend, endian = (spec[key] for key in ('width', 'multiplier', 'addend', 'endian'))
    if type(width) is not int or width not in (1, 2, 4, 8) or endian not in ('little', 'big'):
        raise ValueError('Ordinal XOR key width/endian is invalid')
    mask = (1 << (8 * width)) - 1
    if any(type(value) is not int or not 0 <= value <= mask for value in (multiplier, addend)):
        raise ValueError('Ordinal XOR constants must fit their unsigned native key width')
    return width, multiplier, addend, endian, mask


def text_table(data, schema, *, read_count, apply_view):
    """Verify every native index record; keep transforms local to each entry."""
    allowed = {'kind', 'offset', 'length', 'count', 'record_size', 'offset_field', 'length_field',
               'base', 'pool_length', 'encoding', 'entry_xor'}
    if (not isinstance(schema, dict) or set(schema) - allowed or not (allowed - {'entry_xor'}) <= schema.keys()
            or schema['kind'] != 'offset_length_array'):
        raise ValueError('Offset/length table needs its exact complete index and pool schema; do not mix inline counted-table fields')
    count = read_count(data, schema['count'])
    offset, length, stride, base, pool_length = (schema[key] for key in ('offset', 'length', 'record_size', 'base', 'pool_length'))
    if (type(count) is not int or not 0 < count <= MAX_ROWS
            or any(type(value) is not int for value in (offset, length, stride, base, pool_length))
            or not 2 <= stride <= 64 or length != count * stride or not 0 <= offset <= len(data) - length
            or pool_length < 0 or not 0 <= base <= len(data) - pool_length
            or not isinstance(schema['encoding'], str) or schema['encoding'] not in ENCODINGS):
        raise ValueError('Offset/length table needs native count evidence and exact bounded index/pool ranges')
    offset_field, length_field = _field(schema['offset_field'], stride), _field(schema['length_field'], stride)
    if not (offset_field[0] + offset_field[1] <= length_field[0] or length_field[0] + length_field[1] <= offset_field[0]):
        raise ValueError('Index offset and length fields must not overlap')
    xor, total, result = _xor(schema.get('entry_xor')), 0, []
    for ordinal in range(count):
        record = offset + ordinal * stride
        values = []
        for field_offset, width, endian, unit in (offset_field, length_field):
            start = record + field_offset
            values.append(int.from_bytes(data[start:start + width], endian) * unit)
        relative, size = values
        if not 0 <= relative <= pool_length or not 0 <= size <= pool_length - relative:
            raise ValueError('Offset/length entry exceeds the declared pool: index=' + str(ordinal))
        total += size
        if total > MAX_DECODED_BYTES:
            raise ValueError('Offset/length table exceeds the total decoded-text budget')
        steps = [dict(op='slice', offset=base + relative, length=size)]
        if xor is not None:
            width, multiplier, addend, endian, mask = xor
            key = ((ordinal * multiplier + addend) & mask).to_bytes(width, endian)
            steps.append(dict(op='xor', key=key.hex()))
        decoded = apply_view(data, steps).decode(schema['encoding'])
        result.append((decoded, dict(offset=0, length=size, encoding=schema['encoding'], view=steps)))
    return result
