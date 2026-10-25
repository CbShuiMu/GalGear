"""Legacy audited engine profiles, separate from generic byte evidence.

These bounded compatibility checks retain the already supported WillPlus WS2
and raw NXSC layouts. They are not a registry of universal engine decoders.
AI-generated adapters use generic node-link evidence; an unknown profile must
never inherit the NXSC layout or these profiles' validation status.

The caller supplies its generic integer reader explicitly so this module has
no dependency on, or circular import with, the shared evidence verifier.
"""
import re


NATIVE_MESSAGE_LINK_CONTRACT = dict(
    required_for='Every target_script-scoped speaker/narration entry, named or null; global name tables remain unscoped.',
    native_link=dict(kind='willplus_ws2_name_message_config1_cp932',
                     name_command_offset='<raw input byte offset>', message_command_offset='<raw input byte offset>'),
    id_evidence=dict(offset='message_command_offset+1', width=4, endian='little'),
    name_proof='Named evidence.reference (or a direct name span) must cover the exact CP932 name field. '
               'Narration requires that same name field to be empty, with its actual one-byte NUL span.',
    structure='Supported local profile: SetDisplayName 0x15, CP932 NUL name, zero config; immediately '
              'DisplayMessage 0x14, DWORD little-endian ID, NUL layer, nonempty CP932 NUL text, zero config.',
    limits='Raw input coordinates only. Name <=1024 bytes, layer <=256 bytes, text <=32768 bytes. '
           'This verifies the cited local command link, not the whole instruction stream or every WS2 version. '
           'Missing or unsupported scoped links need proof; nearby NULs or integers never establish narration.')


def _native_cstring_end(data, offset, maximum, encoding, field, nonempty=False):
    if type(offset) is not int or not 0 <= offset < len(data):
        raise ValueError(field + ' is outside the raw input')
    end = data.find(b'\0', offset, min(len(data), offset + maximum + 1))
    if end < 0 or nonempty and end == offset:
        raise ValueError(field + ' needs its bounded, complete NUL-terminated field')
    data[offset:end].decode(encoding)
    return end


def native_message_link(data, record, *, integer):
    """Validate only a cited documented local command link, never extract rows.

    The supported config-byte variant follows SetDisplayName/DisplayMessage in
    advhd_ws2_tools 2253327e02868ef5e84d963e8e34e2439f7e6c18 (>1.06).
    No claim is made that arbitrary offsets establish a complete VM parse.
    """
    link = record.get('native_link')
    if not isinstance(link, dict) or set(link) != {'kind', 'name_command_offset', 'message_command_offset'}:
        raise ValueError('Scoped message needs native_link with kind, name_command_offset and message_command_offset; needs_proof')
    if link['kind'] != 'willplus_ws2_name_message_config1_cp932':
        raise ValueError('Unsupported native_link.kind; scoped message needs a supported local command-link proof')
    name, message = link['name_command_offset'], link['message_command_offset']
    if type(name) is not int or not 0 <= name < len(data) or data[name] != 0x15:
        raise ValueError('native_link.name_command_offset must identify the actual SetDisplayName command')
    if type(message) is not int or not 0 <= message < len(data) or data[message] != 0x14:
        raise ValueError('native_link.message_command_offset must identify the actual DisplayMessage command')
    name_start = name + 1
    name_end = _native_cstring_end(data, name_start, 1024, 'cp932', 'native_link name field')
    if name_end + 2 != message or data[name_end + 1] != 0:
        raise ValueError('native_link name terminator and zero config must immediately precede that same message command')
    identifier_proof = record.get('id_evidence')
    if (not isinstance(identifier_proof, dict) or identifier_proof.get('view')
            or 'encoding' in identifier_proof or type(identifier_proof.get('offset')) is not int
            or identifier_proof['offset'] != message + 1 or type(identifier_proof.get('width')) is not int
            or identifier_proof['width'] != 4
            or identifier_proof.get('endian') != 'little'):
        raise ValueError('native_link id_evidence must be the raw DWORD little-endian field at message_command_offset+1')
    if type(record.get('id')) is not int or integer(data, message + 1, 4, 'little') != record['id']:
        raise ValueError('native_link message ID disagrees with the cited command DWORD')
    layer_end = _native_cstring_end(data, message + 5, 256, 'cp932', 'native_link layer field')
    text_end = _native_cstring_end(data, layer_end + 1, 32768, 'cp932', 'native_link message text', nonempty=True)
    if text_end + 1 >= len(data) or data[text_end + 1] != 0:
        raise ValueError('native_link message text must end at its NUL terminator and zero config byte')
    proof = record.get('evidence')
    reference = proof.get('reference') if isinstance(proof, dict) and 'resource' in proof else proof
    if not isinstance(reference, dict) or reference.get('view') or isinstance(proof, dict) and proof.get('view'):
        raise ValueError('native_link name evidence must use the same raw input name field')
    expected_length = 1 if record.get('text') is None else name_end - name_start
    if (type(reference.get('offset')) is not int or type(reference.get('length')) is not int
            or reference['offset'] != name_start or reference['length'] != expected_length):
        raise ValueError('native_link name evidence must cover exactly the actual name field, never a nearby NUL or other token')
    if record.get('text') is None:
        if name_end != name_start or reference.get('encoding') not in ('ascii', 'cp932', 'utf-8'):
            raise ValueError('native_link narration requires the explicitly empty one-byte native name field')
        if isinstance(proof, dict) and 'resource' in proof:
            raise ValueError('native_link narration must cite its own actual empty-name field, not a global name entry')
    elif name_end == name_start or reference.get('encoding') != 'cp932':
        raise ValueError('native_link named entries require the exact nonempty CP932 native name field')


def _nxsc_visible_caption(text):
    """The supported NXSC renderer projection, never arbitrary model rules."""
    if not isinstance(text, str) or not text:
        raise ValueError('NXSC current-caption field must be nonempty text')
    if any(ord(character) < 32 and character not in '\t\n\r' for character in text):
        raise ValueError('NXSC current-caption contains unsupported control characters')
    if re.search(r'%(?![KP])[A-Za-z]|\\(?![nr])[A-Za-z]', text):
        raise ValueError('NXSC current-caption contains unsupported renderer controls')
    projected = re.sub(r'%[KP]', '', text.replace('\\n', '\n').replace('\\r', '\r'))
    projected = ''.join(projected.split())
    if not projected:
        raise ValueError('NXSC current-caption is empty after renderer controls')
    return projected


def _nxsc_current_caption(data, *, integer):
    """Recognize only the audited raw NXSC fixed-prefix caption layout.

    The 4-byte field after magic remains opaque. The following 12 reserved
    bytes must be zero; unsupported prefix variants fail closed. The caption
    is a u32 little-endian UTF-16 code-unit count after the 20-byte prefix.
    No title, script basename, game offset, sample position or node is chosen.
    """
    if data[:4] != b'NXSC':
        return None
    if len(data) < 24 or data[8:20] != b'\0' * 12:
        raise ValueError('Unsupported or malformed NXSC current-caption header layout')
    units = integer(data, 20)
    if not 0 < units <= 100000 or units * 2 > len(data) - 24:
        raise ValueError('Malformed NXSC current-caption header count or bounds')
    try:
        caption = data[24:24 + units * 2].decode('utf-16-le')
    except UnicodeError as error:
        raise ValueError('Malformed NXSC current-caption UTF-16 field') from error
    return dict(visible=_nxsc_visible_caption(caption), end=24 + units * 2)


def _verify_nxsc_current_node(data, facts, proof, dialogue, header, *, integer):
    """Reject stale/spliced records; never replace the parser's chosen node."""
    source, position, anchor = (proof.get(key) for key in ('source', 'position', 'anchor'))
    if any(not isinstance(field, dict) or field.get('view') for field in (source, position, anchor)):
        raise ValueError('NXSC current-node proof requires one raw save record in a shared byte coordinate space')
    if source.get('encoding') != 'utf-16-le' or anchor.get('encoding') != 'utf-16-le':
        raise ValueError('NXSC current record source and caption need UTF-16-LE byte spans')
    source_offset, source_bytes = source.get('offset'), source.get('length')
    if (type(source_offset) is not int or type(source_bytes) is not int or source_bytes <= 0
            or source_bytes % 2 or source_offset - 4 < header['end']):
        raise ValueError('NXSC source proof must follow the header in its own complete counted native record')
    if integer(data, source_offset - 4) * 2 != source_bytes:
        raise ValueError('NXSC source span disagrees with its native UTF-16 length prefix; do not scan past the counted field')
    position_offset = source_offset + source_bytes
    if (position.get('offset') != position_offset or position.get('width', 4) != 4
            or position.get('endian', 'little') != 'little'):
        raise ValueError('NXSC native position proof must immediately follow that same counted source field')
    text_bytes = integer(data, position_offset + 4) * 2
    if (not text_bytes or anchor.get('offset') != position_offset + 8
            or anchor.get('length') != text_bytes or text_bytes > len(data) - position_offset - 8):
        raise ValueError('NXSC anchor proof must cover the complete counted text in the same source/position record')
    if _nxsc_visible_caption(facts['anchors'][0]) != header['visible']:
        raise ValueError('NXSC selected node caption disagrees with the native current-caption header; '
                         'a valid historical tuple or greatest/first/last file offset does not prove the current node')
    nodes = set()
    for row in dialogue:
        try:
            visible = _nxsc_visible_caption(row.get('text'))
        except ValueError:
            continue
        if visible == header['visible']:
            nodes.add((row.get('source'), row.get('position')))
    if nodes != {(facts['source'], facts['line'])}:
        raise ValueError('NXSC native current-caption identifies multiple or conflicting transcript nodes; do not choose first or last')
