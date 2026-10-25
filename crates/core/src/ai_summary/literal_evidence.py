"""Bounded serialized data literals with independently derived field spans.

This is a fixed data-only subset, never a script interpreter or model grammar.
"""
import copy
import functools
import hashlib
import pathlib
import re
import runpy


_CODECS = runpy.run_path(str(pathlib.Path(__file__).with_name('bytecodecs.py')))


MAX_BYTES = 32 * 1024 * 1024
MAX_NODES = 200_000
MAX_DEPTH = 64
MAX_TOKEN_CHARACTERS = 128
MAX_STRING_CHARACTERS = 1_048_576
MAX_PATH = 32
ENCODINGS = frozenset(('ascii', 'utf-8', 'utf-16-le', 'utf-16-be'))
UINT64_MAX = 2**64 - 1
DECIMAL = re.compile(r'(?:0|[1-9][0-9]{0,19})\Z', re.ASCII)
NUMBER = re.compile(r'-?(?:0[xX][0-9a-fA-F]+(?:\.[0-9a-fA-F]*)?[pP][+-]?[0-9]+'
                    r'|(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)', re.ASCII)

CONTRACT = dict(
    document=dict(syntax='tjs_const', encoding='ascii|utf-8|utf-16-le|utf-16-be', view=[]),
    field=dict(kind='structured_field', document='<document>', record=['<key or integer index>'],
               field='<direct object member>', type='string|text_integer'),
    syntax='Fixed serialized data subset: optional (const) objects/arrays, quoted keys/strings, '
           'decimal integers/reals, hex-float, void/null and bounded block comments. '
           'Not a general TJS interpreter. No expressions, arbitrary grammar, trailing commas or unknown escapes.',
    integer='The complete selected integer token is 0 or [1-9][0-9]*, at most 20 ASCII digits and <=2**64-1. '
            'No signs, leading zeroes, whitespace, exponent, hexadecimal, string coercion or binary fallback.',
    scope='Every linked source/scope/coordinate field must use one shared document and selected object record. '
          'Selectors establish structure, not which engine state is current.',
    limits=dict(document_bytes=MAX_BYTES, nodes=MAX_NODES, depth=MAX_DEPTH,
                string_characters=MAX_STRING_CHARACTERS, numeric_token_characters=MAX_TOKEN_CHARACTERS,
                path_components=MAX_PATH))


class _Node:
    __slots__ = ('kind', 'value', 'start', 'end')

    def __init__(self, kind, value, start, end):
        self.kind, self.value, self.start, self.end = kind, value, start, end


class _Parser:
    def __init__(self, text):
        self.text, self.position, self.nodes = text, 0, []

    def fail(self, reason):
        raise ValueError('Structured literal evidence ' + reason + ' at character ' + str(self.position))

    def node(self, kind, value, start):
        if len(self.nodes) >= MAX_NODES:
            self.fail('exceeds its node budget')
        node = _Node(kind, value, start, self.position)
        self.nodes.append(node)
        return node

    def whitespace(self):
        while self.position < len(self.text):
            if self.text[self.position] in ' \t\r\n':
                self.position += 1
            elif self.text.startswith('/*', self.position):
                self.position += 2
                depth = 1
                while depth:
                    if self.position >= len(self.text):
                        self.fail('contains an unterminated block comment')
                    if self.text.startswith('/*', self.position):
                        depth += 1
                        if depth > MAX_DEPTH:
                            self.fail('exceeds its comment depth budget')
                        self.position += 2
                    elif self.text.startswith('*/', self.position):
                        depth -= 1
                        self.position += 2
                    else:
                        self.position += 1
            else:
                break

    def string(self):
        start = self.position
        self.position += 1
        characters = []
        escapes = {'"': '"', "'": "'", '\\': '\\', 'n': '\n', 'r': '\r',
                   't': '\t', 'b': '\b', 'f': '\f', 'v': '\v', 'a': '\a', '0': '\0'}
        while self.position < len(self.text):
            character = self.text[self.position]
            self.position += 1
            if character == '"':
                following = self.position
                while following < len(self.text) and self.text[following] in ' \t\r\n':
                    following += 1
                if self.text.startswith('"', following):
                    # TJS lexical concatenation of adjacent quoted strings.
                    self.position = following + 1
                    continue
                try:
                    value = ''.join(characters).encode('utf-16-le', 'surrogatepass').decode('utf-16-le', 'strict')
                except UnicodeError:
                    self.fail('contains an invalid escaped surrogate sequence')
                return self.node('string', value, start)
            if character == '\\':
                if self.position >= len(self.text):
                    self.fail('contains an unsupported or incomplete string escape')
                if self.text[self.position] in 'xX':
                    self.position += 1
                    digits = self.position
                    while (self.position < len(self.text) and self.position - digits < 4
                           and self.text[self.position] in '0123456789abcdefABCDEF'):
                        self.position += 1
                    if self.position == digits:
                        self.fail('requires hexadecimal digits in a hex string escape')
                    character = chr(int(self.text[digits:self.position], 16))
                else:
                    if self.text[self.position] not in escapes:
                        self.fail('contains an unsupported or incomplete string escape')
                    character = escapes[self.text[self.position]]
                    self.position += 1
                    if character == '\0' and self.position < len(self.text) and self.text[self.position] in '0123456789':
                        self.fail('does not support octal escapes')
            elif ord(character) < 32:
                self.fail('contains an unescaped string control character')
            characters.append(character)
            if len(characters) > MAX_STRING_CHARACTERS:
                self.fail('exceeds its string character budget')
        self.fail('contains an unterminated string')

    def value(self, depth=0):
        if depth > MAX_DEPTH:
            self.fail('exceeds its depth budget')
        self.whitespace()
        start = self.position
        const = self.text.startswith('(const)', self.position)
        if const:
            self.position += 7
            self.whitespace()
        if self.text.startswith('%[', self.position):
            self.position += 2
            value = {}
            self.whitespace()
            if self.text.startswith(']', self.position):
                self.position += 1
                return self.node('object', value, start)
            while True:
                self.whitespace()
                if not self.text.startswith('"', self.position):
                    self.fail('requires a quoted object key')
                key = self.string().value
                if key in value:
                    self.fail('contains a duplicate decoded object key')
                self.whitespace()
                if not self.text.startswith('=>', self.position):
                    self.fail('requires the object member separator')
                self.position += 2
                value[key] = self.value(depth + 1)
                self.whitespace()
                if self.text.startswith(']', self.position):
                    self.position += 1
                    return self.node('object', value, start)
                if not self.text.startswith(',', self.position):
                    self.fail('requires an object item separator or closing delimiter')
                self.position += 1
        if self.text.startswith('[', self.position):
            self.position += 1
            value = []
            self.whitespace()
            if self.text.startswith(']', self.position):
                self.position += 1
                return self.node('array', value, start)
            while True:
                value.append(self.value(depth + 1))
                self.whitespace()
                if self.text.startswith(']', self.position):
                    self.position += 1
                    return self.node('array', value, start)
                if not self.text.startswith(',', self.position):
                    self.fail('requires an array item separator or closing delimiter')
                self.position += 1
        if const:
            self.fail('allows the const prefix only on objects and arrays')
        if self.text.startswith('"', self.position):
            return self.string()
        if self.text.startswith('void', self.position):
            self.position += 4
            return self.node('void', None, start)
        if self.text.startswith('null', self.position):
            self.position += 4
            return self.node('null', None, start)
        match = NUMBER.match(self.text, self.position)
        if match is None:
            self.fail('contains unsupported syntax')
        token = match.group()
        if len(token) > MAX_TOKEN_CHARACTERS:
            self.fail('exceeds its numeric token budget')
        self.position = match.end()
        if re.fullmatch(r'-?(?:0|[1-9][0-9]*)', token, re.ASCII):
            return self.node('integer', int(token), start)
        # Non-coordinate real values remain lexical data. No floating evaluation.
        return self.node('real', token, start)

    def parse(self):
        if self.text.startswith('\ufeff'):
            self.position = 1
        root = self.value()
        self.whitespace()
        if self.position != len(self.text):
            self.fail('has trailing syntax after the root value')
        return root


@functools.lru_cache(maxsize=2)
def _parse(raw, encoding):
    text = raw.decode(encoding, 'strict')
    if text.encode(encoding, 'strict') != raw:
        raise ValueError('Structured literal evidence requires an exact text encoding roundtrip')
    parser = _Parser(text)
    root = parser.parse()
    # Translate all token boundaries in a single forward pass. UTF-16 astral
    # characters consume four bytes; multiplying Python character indices by two
    # would produce incorrect native spans.
    marks = sorted({position for node in parser.nodes for position in (node.start, node.end)} | {len(text)})
    offsets, previous, byte_position = {}, 0, 0
    for mark in marks:
        byte_position += len(text[previous:mark].encode(encoding, 'strict'))
        offsets[mark], previous = byte_position, mark
    if byte_position != len(raw):
        raise ValueError('Structured literal evidence byte boundary conversion disagrees with its input')
    return root, text, offsets


def _document(data, schema):
    if (not isinstance(schema, dict) or set(schema) - {'syntax', 'encoding', 'view'}
            or schema.get('syntax') != 'tjs_const' or schema.get('encoding') not in ENCODINGS):
        raise ValueError('Structured document needs supported syntax, strict encoding and optional standard view')
    if not isinstance(data, bytes):
        raise ValueError('Structured document input must be bytes')
    steps = schema.get('view', [])
    if not isinstance(steps, list) or len(steps) > 8:
        raise ValueError('Structured document needs a bounded standard byte view')
    if steps:
        raw = _CODECS['view'](data, copy.deepcopy(steps))
    else:
        raw = data
    if not isinstance(raw, bytes) or not 0 < len(raw) <= MAX_BYTES:
        raise ValueError('Structured document exceeds its byte budget or is empty')
    root, text, offsets = _parse(raw, schema['encoding'])
    return root, text, offsets, raw


def _span(node, offsets, encoding):
    return dict(offset=offsets[node.start], length=offsets[node.end] - offsets[node.start], encoding=encoding)


def parse_document(data, schema):
    """Return a fresh typed tree with token spans in the fully replayed view."""
    root, text, offsets, raw = _document(data, schema)
    def public(node):
        if node.kind == 'object':
            value = {key: public(child) for key, child in node.value.items()}
        elif node.kind == 'array':
            value = [public(child) for child in node.value]
        else:
            value = node.value
        return dict(type=node.kind, value=value, span=_span(node, offsets, schema['encoding']))
    return dict(root=public(root), document=copy.deepcopy(schema), bytes=len(raw),
                sha256=hashlib.sha256(raw).hexdigest())


def replay_field(data, proof):
    """Replay one immediate property of one selected object, never an offset."""
    required = {'kind', 'document', 'record', 'field', 'type'}
    if not isinstance(proof, dict) or set(proof) != required or proof.get('kind') != 'structured_field':
        raise ValueError('Structured field needs kind, document, record, field and type; byte offsets are not selectors')
    path, field, expected = proof['record'], proof['field'], proof['type']
    if (not isinstance(path, list) or len(path) > MAX_PATH
            or any(not (isinstance(part, str) and len(part.encode('utf-8')) <= 4096
                        or type(part) is int and 0 <= part <= 2**32 - 1) for part in path)
            or not isinstance(field, str) or len(field.encode('utf-8')) > 4096
            or expected not in ('string', 'text_integer')):
        raise ValueError('Structured field needs a bounded object path, immediate member and supported type')
    root, text, offsets, raw = _document(data, proof['document'])
    node = root
    for part in path:
        if node.kind == 'object' and isinstance(part, str) and part in node.value:
            node = node.value[part]
        elif node.kind == 'array' and type(part) is int and part < len(node.value):
            node = node.value[part]
        else:
            raise ValueError('Structured record selector does not identify an existing typed node')
    if node.kind != 'object' or field not in node.value:
        raise ValueError('Structured field must be an immediate existing member of the selected object record')
    selected = node.value[field]
    token = text[selected.start:selected.end]
    if expected == 'string':
        if selected.kind != 'string':
            raise ValueError('Structured string field must select an actual quoted string token')
        value = selected.value
    else:
        if selected.kind != 'integer' or DECIMAL.fullmatch(token) is None:
            raise ValueError('Structured text_integer field needs a complete canonical ASCII uint64 decimal token')
        value = int(token)
        if value > UINT64_MAX:
            raise ValueError('Structured text_integer field exceeds uint64')
    span = _span(selected, offsets, proof['document']['encoding'])
    if proof['document'].get('view'):
        span['view'] = copy.deepcopy(proof['document']['view'])
    return dict(type=expected, value=value, token=token, span=span,
                record=copy.deepcopy(path), field=field, document=copy.deepcopy(proof['document']),
                document_sha256=hashlib.sha256(raw).hexdigest())


def read_structured_field(data, proof):
    return replay_field(data, proof)['value']
