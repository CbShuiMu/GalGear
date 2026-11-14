"""Native route evidence independent of model prose and dialogue adjacency.

Cotopha function records are decoded structurally. Direct-call byte patterns
remain candidates because a full instruction-boundary decoder is unavailable.
Every retained fact carries its input hash and file offset; truncation is explicit.
"""
import bisect
import collections
import hashlib
import json
import pathlib
import re
import runpy
import struct
import tempfile


ADAPTER = runpy.run_path(str(pathlib.Path(__file__).with_name('entis_adapter.py')))
CALL = re.compile(rb'\x11(.{4})\x00\x04(.{4})', re.DOTALL)


def cotopha(data, source, rows):
    module = ADAPTER['parse_csx'](data)
    base = module['sections']['image']['offset']
    functions = sorted((f for f in module['functions'] if f['declared_length'] > 0), key=lambda item: item['address'])
    addresses = [item['address'] for item in functions]
    by_address = {item['address']: item for item in functions}

    def owner(position):
        index = bisect.bisect_right(addresses, position) - 1
        if index < 0:
            return None
        function = functions[index]
        end = min(function['address'] + function['declared_length'], len(module['code']),
                  addresses[index+1] if index+1 < len(addresses) else len(module['code']))
        if not function['address'] <= position < end:
            return None
        return function

    story = set()
    for row in rows:
        if type(row.get('position')) is int:
            function = owner(row['position'] - base)
            if function:
                story.add(function['address'])
    # Deduplicate repeated calls per function. Keep rare native callees first:
    # thousands of OutMsg/rendering sites otherwise hide menu/ending candidates.
    calls = {}
    for match in CALL.finditer(module['code']):
        arity, target = struct.unpack('<I', match[1])[0], struct.unpack('<I', match[2])[0]
        caller = owner(match.start())
        if (arity > 32 or caller is None or target not in by_address
                or owner(match.end()-1) is not caller
                or (caller['address'] not in story and target not in story)):
            continue
        key = (caller['address'], target, arity)
        if key not in calls:
            calls[key] = dict(kind='call_candidate', from_function=caller['name'],
                to_function=by_address[target]['name'], position=base + match.start(),
                target_position=base + target, arity=arity, certainty='candidate',
                instruction_boundary_verified=False, occurrences=0, sample_positions=[])
        record = calls[key]
        record['occurrences'] += 1
        if len(record['sample_positions']) < 3:
            record['sample_positions'].append(base + match.start())
    frequency = collections.Counter(target for _, target, _ in calls)
    selected_calls = sorted(calls, key=lambda key: (frequency[key[1]], calls[key]['position']))[:1024]
    selected_addresses = story | {address for key in selected_calls for address in key[:2]}
    selected = [f for f in functions if f['address'] in selected_addresses]
    facts = [dict(kind='function', name=f['name'], position=base + f['address'],
                  name_proof=f['proof'], has_dialogue=f['address'] in story,
                  certainty='native_table') for f in selected[:2048]]
    facts.extend(calls[key] for key in selected_calls)
    header = dict(schema=1, source=source, sha256=hashlib.sha256(data).hexdigest(),
                  engine='cotopha', coverage='partial_static_candidates',
                  function_count=len(selected), call_candidate_count=len(calls),
                  omitted_functions=max(0, len(selected)-2048), omitted_calls=max(0, len(calls)-1024),
                  limitations='No complete instruction-boundary CFG, menu conditions, ending semantics or read history. Function tables and byte-pattern calls alone cannot establish playable routes. Repeated calls are grouped, at most three positions sampled per group. Mark derived connections inferred.')
    return [header] + [dict(source=source, **fact) for fact in facts]


def export(work, parts, resolve):
    """Inspect only accepted script inputs, never scan unrelated game files."""
    records = []
    for source, rows in sorted(parts.items()):
        path = resolve(source)
        if path is None:
            records.append(dict(source=source, coverage='unavailable', reason='Accepted input path unavailable'))
            continue
        try:
            if not 0 < path.stat().st_size <= 32 * 1024 * 1024:
                raise ValueError('Native route inspection limited to 32 MiB per input')
            with path.open('rb') as file:
                header = file.read(64)
            if header[:8] != b'Entis\x1a\0\0' or not header[16:56].startswith(b'Cotopha Image file'):
                records.append(dict(source=source, coverage='unavailable', reason='No registered native route reader for this header'))
                continue
            records.extend(cotopha(path.read_bytes(), source, rows))
        except (OSError, ValueError, struct.error) as error:
            records.append(dict(source=source, coverage='unavailable', reason=str(error)[:240]))
    output = work / 'route-evidence.jsonl'
    raw = ''.join(json.dumps(record, ensure_ascii=False, separators=(',', ':')) + '\n' for record in records).encode('utf-8')
    if len(raw) > 128 * 1024 * 1024:
        raise ValueError('Native route evidence exceeds 128 MiB')
    if output.is_symlink() or output.exists() and not output.is_file():
        raise ValueError('Native route evidence must be a regular file')
    with tempfile.NamedTemporaryFile(dir=work, delete=False) as file:
        temporary = pathlib.Path(file.name)
        file.write(raw)
    try:
        temporary.replace(output)
    finally:
        temporary.unlink(missing_ok=True)
    return hashlib.sha256(raw).hexdigest()
