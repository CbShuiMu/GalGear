"""将已验证的台词与说话人表导出为带来源的 UTF-8 JSON。"""
import hashlib
import json
from pathlib import Path
import runpy
import time

READS = runpy.run_path(str(Path(__file__).with_name('analysis_reads.py')))
NATIVE = ('ast_tables', 'siglus_scene', 'psb')
READERS = (*NATIVE, 'script', 'speakers')


def decoder(agent, value):
    if not isinstance(value, dict) or set(value) - {'reader', 'program', 'speaker_tests'}:
        raise ValueError('Select a dialogue decoder')
    reader = value.get('reader')
    if reader not in READERS:
        raise ValueError('Select a supported dialogue reader')
    program, dependencies = value.get('program'), value.get('speaker_tests', [])
    if reader in ('script', 'speakers'):
        if not isinstance(program, str) or not 0 < len(program) <= 65536:
            raise ValueError('Provide a bounded pure extract(data, metadata) program')
        agent.api['validate_program'](program)
    elif program is not None:
        raise ValueError('Installed readers do not accept a custom program')
    if (not isinstance(dependencies, list) or len(dependencies) > 16
            or any(not isinstance(key, str) for key in dependencies)
            or len(set(dependencies)) != len(dependencies)
            or dependencies and reader != 'script'):
        raise ValueError('Select up to 16 distinct companion table tests for a script decoder')
    return dict(value, speaker_tests=dependencies)


def native_rows(agent, path, reader):
    skill = agent.host.roots['skill']
    if reader == 'psb':
        # Use only the fixed PSB / MDF document contract, never a caller-selected byte view.
        with path.open('rb') as stream:
            if stream.read(4) not in (b'PSB\0', b'mdf\0'):
                raise ValueError('Expected a PSB script or its MDF wrapper')
        result = READS['read_one'](skill, path, 'structured')
        return [
            dict(variant, scene_index=scene['scene_index'], scene=scene['label'],
                 record_index=row['record_index'])
            for scene in result['scene_dialogue']
            for row in scene['records']
            for variant in row['variants']
        ]
    result = agent.api['SKILL_READERS']['run'](
        agent.host.reader_command, skill, path, reader, timeout=agent.deadline - time.monotonic())
    # AST also reports choice text. Siglus includes the temporary absolute filename.
    return [{key: item for key, item in row.items() if key != 'source'}
            for row in result['dialogue'] if row.get('kind') != 'literal-ast-choice']


def decode(agent, path, recipe, origin):
    if time.monotonic() >= agent.deadline:
        raise TimeoutError('Dialogue extraction exceeded its runtime limit')
    agent.stamp(path)
    with path.open('rb') as stream:
        data = stream.read(READS['MAX_BYTES'] + 1)
    if not 0 < len(data) <= READS['MAX_BYTES']:
        raise ValueError('Dialogue input must be 1..32 MiB')
    file_hash = hashlib.sha256(data).hexdigest()
    selected = recipe['dialogue']
    reader = selected['reader']
    resources = {}
    for key in selected['speaker_tests']:
        if key not in agent.speaker_resources:
            raise ValueError('Missing verified companion speaker table: ' + key)
        resources[key] = agent.speaker_resources[key]
    if reader in NATIVE:
        rows = native_rows(agent, path, reader)
    else:
        verifier = agent.api['EVIDENCE']
        result = agent.api['run_parser'](selected['program'], data, dict(
            source=origin['member'], speaker_resources=resources))
        if reader == 'speakers':
            _, rows = verifier['speaker_metadata'](data, result)
        else:
            member = verifier['view'](data, result.get('member_filter', [])) if isinstance(result, dict) else data
            regions = agent.api['WORKFLOW']['required_table_regions'](agent.host.roots['skill'], member)
            rows = verifier['script_rows'](data, result, required_table_regions=regions,
                                           speaker_resources=resources)
            rows = [{key: row[key] for key in ('text', 'speaker', 'position', 'evidence')} for row in rows]
    if reader != 'speakers':
        rows = [row for row in rows if row['text'].strip()]
    if not rows:
        raise ValueError('The selected input produced no verified ' + ('speaker entries' if reader == 'speakers' else 'dialogue'))
    if agent.files_api['digest'](path) != file_hash:
        raise ValueError('Dialogue input changed while decoding')
    return rows, dict(origin, reader=reader, sha256=file_hash)


def extract(agent, path, recipe, name):
    name = name.replace('\\', '/')
    origin = dict(mode=recipe['mode'], path=Path(recipe['path']).as_posix(), member=name)
    rows, source = decode(agent, path, recipe, origin)
    is_table = source['reader'] == 'speakers'
    if is_table and len(recipe['members']) == 1:
        agent.speaker_resources[recipe['id']] = rows
    filename = 'speakers.json' if is_table else 'dialogue.json'
    if agent.is_batch:
        # Use logical game/archives coordinates, so temporary unpack paths cannot
        # split or merge outputs for the same member across recipes.
        group = hashlib.sha256(json.dumps(origin, sort_keys=True).encode()).hexdigest()[:24]
        relative = Path('speakers' if is_table else 'scripts') / group / filename
        body = dict(schema=1, source=source, **({'entries': rows} if is_table else {'rows': rows}))
        agent.files_api['write_member'](agent, relative, body, source)
    return dict(format=filename, records=len(rows), sha256=source['sha256'])
