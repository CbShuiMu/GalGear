"""Audited standard KAG bookmark resume entries, never displayed captions.

Public semantics: KAGParser Store records storageName/storageShortName/curLabel;
Restore reloads StorageName then GoToLabel(CurLabel), not saved curLine/curPos.
Label indexing mirrors the original tab/newline, omitted-name and alias rules.
https://github.com/krkrz/KAGParser/blob/c2269b26b390bd09aa1f2b39d7d779cb79c26f2e/KAGParser.cpp
https://github.com/krkrz/kag3/blob/1f3ab309106d210e3169bbbe0fb4e066ae463b42/data/system/MainWindow.tjs

Extensions replacing scenario loading or bookmark restoration need a different
profile. This module does not execute tags, macros, inline TJS or story choices.
"""
import copy
import functools
import hashlib
import json
import pathlib
import re
import runpy


_BASE = pathlib.Path(__file__).parent
_CODECS = runpy.run_path(str(_BASE / 'bytecodecs.py'))
_TYPED = runpy.run_path(str(_BASE / 'typed_evidence.py'))
PROFILE = 'kag_parser_label_v1'
MAX_BYTES = 32 * 1024 * 1024
MAX_LINES = 500_000
ENCODINGS = ('utf-8', 'utf-16-le', 'utf-16-be', 'cp932', 'gb18030', 'ascii')
_NEWLINE = re.compile(r'\r\n|\r|\n')
CONTRACT = dict(
    profile=PROFILE,
    helper='galgear_kag.label_index(data,{encoding:<strict codec>,view:[]}) returns the complete native label '
           'index. Each labels item has alias, heading, line (zero-based physical line), position (original '
           'byte offset of the first *), text (complete original label line), evidence (exact TEXT span). '
           'Use every label entry needed for resume coverage; labels are markers, not dialogue messages.',
    script_native=dict(kind='kag_label_resume', encoding='<same strict codec>', label='<exact alias from complete label_index>'),
    script='A label row has position/text/evidence exactly from the helper and speaker=null. '
           'The host reparses the entire effective original KS and signs only matching labels. '
           'Duplicate names use :2/:3 aliases; omitted names reuse the previous named label. '
           'Pool spans, arbitrary ordinals and model-supplied label maps cannot replace this scan.',
    saved_link=dict(kind='kag_label_resume', document='<complete tjs_const schema>', record=['core', 'mainConductor']),
    saved='Only the standard public bookmark core.mainConductor is eligible. The host reads the exact '
          'storageName, storageShortName and nonempty curLabel fields from that object. '
          'History snapshots, call frames, custom field aliases, curLine/curPos, preview captions and '
          'last-displayed text cannot select a restore entry. Empty curLabel is unsupported in this profile.',
    facts='Return {anchors:[],evidence:{node_link:<saved_link>}}. source/line may be omitted; '
          'the host fills them from the unique verified source+label entry. Model-provided resume is discarded. '
          'Only independent success emits resume={profile:"kag_parser_label_v1",state_role:"resume_node",'
          'source:<canonical source>,label:<exact saved alias>,position:<native label byte offset>}.',
    role='resume_node proves a bookmark restore entry, not the last displayed or next executed story message. '
         'No caption is invented, anchors remain empty, and macros/branches/custom framework semantics remain unknown. '
         'A framework wrapper entry cannot establish a separate plugin/ScenePlayer story coordinate.')


def _fail(reason):
    raise ValueError('KAG resume evidence ' + reason)


@functools.lru_cache(maxsize=4)
def _index(raw, encoding):
    if not 0 < len(raw) <= MAX_BYTES:
        _fail('script is empty or exceeds its complete byte budget')
    if encoding not in ENCODINGS:
        _fail('script requires a supported strict character encoding')
    try:
        text = raw.decode(encoding, 'strict')
    except UnicodeError as error:
        raise ValueError('KAG resume evidence script has invalid character encoding') from error
    if text.encode(encoding, 'strict') != raw or '\0' in text:
        _fail('script needs an exact encoding roundtrip without embedded NULs')
    # TVP text streams strip the initial BOM before KAG scenario caching.
    position = 1 if text.startswith('\ufeff') else 0
    byte_position = len(text[:position].encode(encoding))
    line_number, previous, counters, aliases, labels = 0, None, {}, set(), []
    while position < len(text):
        match = _NEWLINE.search(text, position)
        end = match.start() if match else len(text)
        following = match.end() if match else len(text)
        content_start = position
        while content_start < end and text[content_start] == '\t':
            content_start += 1
        line = text[content_start:end]
        if len(line) >= 2 and line.startswith('*'):
            label, separator, heading = line.partition('|')
            if label == '*':
                if previous is None:
                    _fail('first label cannot omit its name')
                label = previous
            previous = label
            if len(label.encode()) > 4096 or '\0' in label:
                _fail('label exceeds its identifier budget')
            counters[label] = counters.get(label, 0)+1
            alias = label if counters[label] == 1 else label+':'+str(counters[label])
            if alias in aliases:
                _fail('explicit labels collide with generated duplicate aliases')
            aliases.add(alias)
            offset = byte_position+len(text[position:content_start].encode(encoding))
            labels.append(dict(alias=alias, heading=heading if separator else '', line=line_number,
                position=offset, text=line,
                evidence=dict(offset=offset, length=len(line.encode(encoding)), encoding=encoding)))
        byte_position += len(text[position:following].encode(encoding))
        position = following
        line_number += 1
        if line_number > MAX_LINES:
            _fail('script exceeds its complete line budget')
    return dict(profile=PROFILE, encoding=encoding, lines=line_number, labels=labels,
                script_sha256=hashlib.sha256(raw).hexdigest())


def label_index(data, schema):
    if (not isinstance(schema, dict) or set(schema)-{'encoding', 'view'} or 'encoding' not in schema):
        _fail('label index needs encoding and an optional standard complete-script view')
    raw = _CODECS['view'](data, copy.deepcopy(schema.get('view', [])))
    result = copy.deepcopy(_index(raw, schema['encoding']))
    if schema.get('view'):
        for label in result['labels']:
            label['evidence']['view'] = copy.deepcopy(schema['view'])
    return result


def attest_label(data, row, declaration):
    if (not isinstance(declaration, dict) or set(declaration) != {'kind', 'encoding', 'label'}
            or declaration.get('kind') != 'kag_label_resume'):
        _fail('script proof needs kind, encoding and exact native label alias')
    label = declaration['label']
    if not isinstance(label, str):
        _fail('script label alias must be an actual string')
    index = _index(data, declaration['encoding'])
    matches = [entry for entry in index['labels'] if entry['alias'] == label]
    if len(matches) != 1:
        _fail('label alias does not identify one entry in the complete native script')
    selected = matches[0]
    proof = row.get('evidence')
    if (row.get('position') != selected['position'] or row.get('text') != selected['text']
            or row.get('speaker') is not None or not isinstance(proof, dict)
            or proof.get('view') or any(proof.get(key) != value for key, value in selected['evidence'].items())):
        _fail('label row needs its complete original label line, byte offset, strict span and null speaker')
    return dict(version=1, kind='kag_label_resume', profile=PROFILE, role='resume_node', scope=label,
                value=selected['position'], position=selected['position'],
                script_sha256=index['script_sha256'],
                schema_sha256=hashlib.sha256(json.dumps(declaration, sort_keys=True,
                                                       ensure_ascii=False).encode()).hexdigest())


def _storage_name(value):
    if not isinstance(value, str) or not value or len(value.encode()) > 4096 or any(c in value for c in '\0\r\n'):
        _fail('storage name must be a bounded nonempty native string')
    text = value.replace('\\', '/').replace('>', '/')
    if any(part in ('.', '..') for part in text.split('/')):
        _fail('storage identifiers cannot use path traversal components')
    return text.casefold()


def resolve_resume(data, link, dialogue):
    if (not isinstance(link, dict) or set(link) != {'kind', 'document', 'record'}
            or link.get('kind') != 'kag_label_resume' or link.get('record') != ['core', 'mainConductor']
            or not isinstance(link.get('document'), dict) or link['document'].get('syntax') != 'tjs_const'):
        _fail('requires the audited standard bookmark core.mainConductor; arbitrary snapshots or field mappings are unsupported')
    parsed = _TYPED['parse_document'](data, link['document'])
    record = _TYPED['select'](parsed['root'], ['core', 'mainConductor'])
    if record['type'] != 'object':
        _fail('standard mainConductor must be a complete original object')
    fields = {}
    for name in ('storageName', 'storageShortName', 'curLabel'):
        node = _TYPED['select'](record, [name])
        if node['type'] != 'string':
            _fail('standard ' + name + ' must be a native string field')
        fields[name] = node['value']
    source = _storage_name(fields['storageName'])
    short = _storage_name(fields['storageShortName'])
    if '/' in short or source.rsplit('/', 1)[-1] != short:
        _fail('storageName and storageShortName disagree in the original saved object')
    label = fields['curLabel']
    if not label.startswith('*') or len(label.encode()) > 4096 or any(c in label for c in '\0\r\n'):
        _fail('requires a bounded nonempty native curLabel; first-line and custom restore variants are unsupported')
    matches = []
    for row in dialogue:
        candidate = _storage_name(row.get('source'))
        if candidate != source and not candidate.endswith('/'+source):
            continue
        for native in row.get('_native', []):
            if (not isinstance(native, dict) or native.get('kind') != 'kag_label_resume'
                    or native.get('profile') != PROFILE or native.get('version') != 1
                    or native.get('role') != 'resume_node' or native.get('scope') != label
                    or type(native.get('position')) is not int or native['position'] != row.get('position')
                    or native.get('value') != row.get('position')):
                continue
            if any(not isinstance(native.get(key), str) or re.fullmatch('[0-9a-f]{64}', native[key]) is None
                   for key in ('script_sha256', 'schema_sha256')):
                _fail('canonical label attestation is missing its original script/schema binding')
            matches.append(row)
            break
    if len(matches) != 1:
        _fail('saved native source and label must identify exactly one host-attested restore entry; matches=' + str(len(matches)))
    row = matches[0]
    return dict(row=row, resume=dict(profile=PROFILE, state_role='resume_node', source=row['source'],
                                    label=label, position=row['position']))
