"""Invoke installed read-only format readers as observations, never validation.

Only this registry selects code. Model arguments select a reader and bounded
data options, never a module path, executable, Python source or shell command.
"""
import hashlib
import json
import os
from pathlib import Path
import runpy
import re
import subprocess
import struct
import sys
import tempfile

MAX_BYTES = 32 * 1024 * 1024
READERS = {
    'ast_tables': ('scripts/parse_ast_tables.py', 'engine/artemis.md'),
    'siglus_scene': ('scripts/parse_siglus_flow.py', 'engine/siglus.md'),
    'siglus_save': ('tools/siglus-ssu/source/siglus_ssu/sav.py', 'engine/siglus.md'),
    'artemis_save': ('tools/artemis-save/artemis_save_cli.py', 'commands/artemis-save.md'),
}


def script_reader(header):
    """Suggest a reader from bytes only; its full parser must still validate."""
    if len(header) >= 132 and int.from_bytes(header[:4], 'little') == 132:
        return 'siglus_scene'
    if re.match(rb'\A(?:\xef\xbb\xbf)?\s*(?:astver\s*=\s*\d|ast\s*=\s*\{)', header):
        return 'ast_tables'
    return None


def save_reader(data):
    """Suggest only known container headers; no save-progress classification."""
    if len(data) >= 16 and data[:4] in (b'BOWS', b'BOWG', b'BOWX') and int.from_bytes(data[4:8], 'little') in (1002, 1003):
        return 'artemis_save'
    if len(data) >= 40:
        major, minor, year, month, day, weekday, hour, minute, second, ms = struct.unpack_from('<10i', data)
        if (major == 1 and 0 <= minor <= 32 and 1970 <= year <= 2100 and 1 <= month <= 12
                and 1 <= day <= 31 and 0 <= weekday <= 6 and 0 <= hour <= 23
                and 0 <= minute <= 59 and 0 <= second <= 59 and 0 <= ms <= 999):
            return 'siglus_save'
    return None


def text_container(data):
    """Locate a framed TJS text stream, optionally after a complete plain bitmap."""
    offset = 0
    if len(data) >= 54 and data[:2] == b'BM':
        start = int.from_bytes(data[10:14], 'little')
        dib, width, height, planes, depth, compression = struct.unpack_from('<IiiHHI', data, 14)
        if (dib < 40 or start < 14 + dib or planes != 1 or width <= 0 or not height
                or depth not in (1, 4, 8, 16, 24, 32) or compression != 0):
            return None
        offset = start + ((width * depth + 31) // 32) * 4 * abs(height)
    if offset >= len(data) or data[offset:offset + 2] not in (b'\xfe\xfe', b'\xff\xfe'):
        return None
    view = [dict(op='slice', offset=offset, length=len(data) - offset)] if offset else []
    return dict(syntax='tjs_const', encoding='utf-16-le', view=view + [dict(op='tjs_text_stream')])


def entry(skill, reader):
    if reader not in READERS:
        raise ValueError('Unknown installed reader; use one of ' + ', '.join(READERS))
    path = (skill / READERS[reader][0]).resolve()
    if not path.is_relative_to(skill) or not path.is_file():
        raise ValueError('Installed Skill lacks the selected reader')
    return path


def worker(config):
    skill = Path(config['skill']).resolve()
    reader = config['reader']
    script = entry(skill, reader)
    path = Path(config['input']).resolve()
    if not path.is_file() or not 0 < path.stat().st_size <= MAX_BYTES:
        raise ValueError('Reader input must be a nonempty regular file of at most 32 MiB')
    data = path.read_bytes()
    if reader == 'siglus_scene' and (len(data) < 132 or int.from_bytes(data[:4], 'little') != 132):
        hint = 'PSB/MDF data requires inspect_structured and the PSB document contract.' if data[:4] in (b'PSB\0', b'mdf\0') else 'Inspect the actual header and format catalog before choosing another reader.'
        raise ValueError('Input does not have the supported Siglus DAT header. ' + hint)
    options = config.get('options') or {}
    allowed = {'allow_duplicate_keys'} if reader == 'ast_tables' else {'const_profile'} if reader == 'siglus_scene' else set()
    if not isinstance(options, dict) or set(options) - allowed:
        raise ValueError('Unsupported reader option')
    if 'allow_duplicate_keys' in options and not isinstance(options['allow_duplicate_keys'], bool):
        raise ValueError('allow_duplicate_keys must be boolean')
    profile = options.get('const_profile', 0)
    if not isinstance(profile, int) or isinstance(profile, bool) or not 0 <= profile <= 16:
        raise ValueError('const_profile must be an integer in 0..16')
    if reader.startswith('siglus_'):
        sys.path.insert(0, str(skill / 'tools/siglus-ssu/source'))
    if reader == 'siglus_save':
        from siglus_ssu import sav
        header = sav._try_parse_local(data)
        if header is None or not header.get('data_size'):
            raise ValueError('No supported Siglus local-save header/payload')
        if not isinstance(header.get('org_size'), int) or not 0 < header['org_size'] <= MAX_BYTES:
            raise ValueError('Siglus expanded save exceeds the size limit')
        offset = 40 + 7 * 256 * (2 if header['wide'] else 1) + 256 * 4 + 4
        raw = sav._unpack_tnm_data(data[offset:offset + header['data_size']])
        payload, _ = sav._parse_local_payload(raw)
        result = dict(header=header, payload=payload,
                      limitation='Active local_save and selection history are separate observations. A script-coordinate/current-node relation still needs independent verification.')
    elif reader == 'artemis_save':
        helper=runpy.run_path(str(script))
        result = helper['inspect'](data, max_bytes=MAX_BYTES)
        # The installed reader already validated the complete compressed body.
        # Expose text spans from that same body, never guess a zlib magic/header
        # or promote an embedded caption/history string to active engine state.
        saved=helper['load_reader']().IndexedSave(data)
        pattern=re.compile(rb'(?:[\x20-\x7e]|[\xc2-\xdf][\x80-\xbf]|[\xe0-\xef][\x80-\xbf]{2}|[\xf0-\xf4][\x80-\xbf]{3}){4,}')
        candidates=[]
        for match in pattern.finditer(saved.fields):
            try:
                text=match[0].decode('utf-8')
            except UnicodeError:
                continue
            if len(text)<=4096 and all(c.isprintable() for c in text):
                candidates.append(dict(offset=match.start(),length=len(match[0]),text=text))
        result['text_candidates']=candidates
        result['candidate_scope']='UTF-8 spans in the validated decompressed field body; offsets refer to that body. These may be history/cache, not current text.'
    elif reader == 'ast_tables':
        helper = runpy.run_path(str(script))
        text = data.decode('utf-8-sig')
        # Some literal AST exports prepend two declarative header assignments.
        # Strip only this complete grammar, never arbitrary Lua statements.
        header = re.match(r'\A\s*astver\s*=\s*\d+(?:\.\d+)?\s*\r?\n\s*astname\s*=\s*"[^"\\\r\n]*"\s*\r?\n', text)
        offset = len(data) - len(text.encode('utf-8'))
        if header:
            offset += len(text[:header.end()].encode('utf-8'))
            text = text[header.end():]
        with tempfile.TemporaryDirectory(prefix='galgear-ast-reader-') as temporary:
            copy = Path(temporary) / path.name
            copy.write_text(text, encoding='utf-8')
            rows, nodes = helper['extract'](copy, path.name, options.get('allow_duplicate_keys', False))
        tree = helper['parse'](text, allow_duplicate_keys=options.get('allow_duplicate_keys', False))
        result = dict(dialogue=rows, static_nodes=nodes, ast_tree=tree, input_view=dict(offset=offset, length=len(data)-offset),
                      limitation='Literal text tables and static block references do not establish message-consumer order or save position.')
        projected=runpy.run_path(str(Path(__file__).with_name('ast_observations.py')))['project'](tree,path.name)
        result['dialogue'].extend(projected.pop('dialogue'))
        result.update(projected)
    else:
        from siglus_ssu import const
        const.set_profile(profile)
        rows, nodes, edges, checks, bundle = runpy.run_path(str(script))['parse_scene'](path, path.stem)
        result = dict(dialogue=rows, flow=dict(nodes=nodes, edges=edges), checks=checks, const_profile=profile,
                      header=bundle['header'], instructions=bundle['trace'], strings=bundle['str_list'],
                      command_ranges=bundle['command_ranges'], metadata=bundle['meta'])
    result.update(source_sha256=hashlib.sha256(data).hexdigest(), reader=reader,
                  analysis_only=True, verified_current_node=False)
    encoded = json.dumps(result, ensure_ascii=False, allow_nan=False).encode('utf-8')
    if len(encoded) > MAX_BYTES:
        raise ValueError('Reader output exceeds 32 MiB')
    return result


def run(command, skill, path, reader, options=None, timeout=40):
    """Run a fixed installed reader in a disposable, time-bounded process."""
    entry(skill, reader)
    if timeout <= 0:
        raise ValueError('No time remains for the installed reader')
    request = dict(skill=str(skill), input=str(path), reader=reader, options=options or {})
    completed = subprocess.run([*command, '--skill-reader'], input=json.dumps(request).encode(),
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=min(40, timeout),
        creationflags=0x08000000 if os.name == 'nt' else 0)
    if completed.returncode:
        raise ValueError('Installed reader failed: ' + completed.stderr.decode('utf-8', 'replace')[-1800:])
    if len(completed.stdout) > MAX_BYTES:
        raise ValueError('Installed reader output exceeds 32 MiB')
    return json.loads(completed.stdout)
