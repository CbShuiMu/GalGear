"""未知格式研究通道：模型只处理字节，宿主管理来源、隔离、输出与验证凭据。"""
import fnmatch
import contextlib
import base64
import hashlib
import json
import mmap
import os
from pathlib import Path
import re
import runpy
import subprocess
import time
import types
import urllib.parse
import urllib.request
import urllib.error
import uuid
import zipfile


MAX_INPUT = 192 * 1024 * 1024
MAX_OUTPUT = 64 * 1024 * 1024
NATIVE=runpy.run_path(str(Path(__file__).with_name('analysis_native.py')))


def source_coordinates(url):
    """只接受公开 GitHub 固定提交的源码路径，不接收凭据、查询或重定向入口。"""
    parsed=urllib.parse.urlsplit(url)
    if parsed.scheme!='https' or parsed.query or parsed.fragment or parsed.username or parsed.password or parsed.port:
        raise ValueError('Use an HTTPS source URL without credentials/query/fragment/port')
    parts=urllib.parse.unquote(parsed.path).strip('/').split('/')
    if parsed.netloc=='github.com' and len(parts)>=5 and parts[2]=='blob':
        parts=parts[:2]+parts[3:]
    elif parsed.netloc!='raw.githubusercontent.com':
        raise ValueError('Only GitHub blob or raw.githubusercontent.com source links are supported')
    if (len(parts)<4 or not re.fullmatch(r'[0-9a-fA-F]{40}',parts[2])
            or any(part in ('','..','.') or '\\' in part or any(ord(c)<32 for c in part) for part in parts)):
        raise ValueError('Source URL needs an immutable 40-character commit and a safe file path')
    return tuple(parts[:3]),'https://raw.githubusercontent.com/'+'/'.join(urllib.parse.quote(part,safe='') for part in parts)


def documented_revisions(agent):
    """允许访问安装 Skill 文档已引用的仓库版本；同版本的辅助源码也可继续读取。"""
    if hasattr(agent,'reference_revisions'):
        return agent.reference_revisions
    result,total=set(),0
    for root in ('skill','reverse','pack'):
        directory=agent.host.roots.get(root)
        if not directory:
            continue
        for entry in directory.rglob('*.md'):
            path=agent.host.path(root,str(entry.relative_to(directory)))
            size=path.stat().st_size
            total+=size
            if total>16*1024*1024:
                raise ValueError('Installed reference documents exceed the scan budget')
            for url in re.findall(r'https://(?:github\.com|raw\.githubusercontent\.com)/[^\s)\]<>"\']+',path.read_text('utf-8-sig',errors='replace')):
                try: result.add(source_coordinates(url)[0])
                except ValueError: continue
    agent.reference_revisions=result
    return result


def reference_bytes(agent,url,limit):
    """源码和目录使用相同的短重试策略，避免定义查找因一次断连退化为猜测。"""
    opener=urllib.request.build_opener(agent.api['TRANSPORT']['NoRedirect']())
    for attempt in range(2):
        try:
            with opener.open(urllib.request.Request(url,headers={'User-Agent':'GalGear-source-reader'}),
                             timeout=min(20,max(1,agent.deadline-time.monotonic()))) as response:
                return response.read(limit+1)
        except urllib.error.HTTPError:
            raise
        except (urllib.error.URLError,TimeoutError):
            if attempt or time.monotonic()>=agent.deadline:raise


def remote_source_tree(agent,revision,args):
    """列举已获准固定提交的源码路径，便于追踪跨文件类型定义。"""
    owner,repo,commit=revision
    url='https://api.github.com/repos/'+owner+'/'+repo+'/git/trees/'+commit+'?recursive=1'
    path=agent.host.path('work','reference-sources/tree-'+hashlib.sha256(url.encode()).hexdigest()+'.json')
    if path.exists():
        data=json.loads(path.read_text('utf-8'))
    else:
        raw=reference_bytes(agent,url,2*1024*1024)
        if len(raw)>2*1024*1024:raise ValueError('Repository tree exceeds 2 MiB')
        data=json.loads(raw)
        if data.get('truncated'):raise ValueError('Repository source tree is truncated; use an exact documented source path')
        path.parent.mkdir(parents=True,exist_ok=True)
        agent.api['atomic_json'](path,data)
    pattern=args.get('pattern','*').lower()
    paths=sorted(item['path'] for item in data.get('tree',[]) if item.get('type')=='blob'
        and is_source_path(item['path']) and fnmatch.fnmatchcase(item['path'].lower(),pattern))
    offset=max(0,int(args.get('offset',0)))
    return dict(members=[dict(path=name,url='https://raw.githubusercontent.com/'+owner+'/'+repo+'/'+commit+'/'+urllib.parse.quote(name,safe='/'))
                         for name in paths[offset:offset+200]],total=len(paths),
                next_offset=offset+200 if offset+200<len(paths) else None)


def is_source_path(path):
    return Path(path).suffix.lower() in ('.cs','.py','.rs','.c','.cpp','.h','.hpp','.md','.txt')


def remote_dependencies(agent,revision,url,source):
    """补齐普通辅助类及 partial 布局，远程读取与本地源码保持相同上下文。"""
    names=list(dict.fromkeys(re.findall(r'\bpartial\s+(?:class|struct)\s+(\w+)',source)
        + referenced_types(source) + re.findall(r'\bReadUnmanaged(?:Array)?<([A-Z]\w*)>',source)))
    family=Path(urllib.parse.urlsplit(url).path).name.split('.')[0]
    candidates=[]
    for name in names:
        tree=remote_source_tree(agent,revision,dict(pattern='*'+name+'*.cs'))
        candidates.extend(item for item in tree['members'] if item['url']!=url and
            (Path(item['path']).name.casefold()==(name+'.cs').casefold() or name==family and
             any(word in Path(item['path']).stem.lower().split('.') for word in ('structs','types','headers'))))
    candidates=list({item['url']:item for item in candidates}.values())
    candidates.sort(key=lambda item:Path(item['path']).name==family+'.cs')
    result,remaining=[],32000
    calls=set(re.findall(r'\.\s*(\w+)\s*\(',source))
    supplied=getattr(agent,'remote_reference_dependencies',set())
    for item in candidates[:6]:
        if item['url'] in supplied:
            result.append(dict(member=item['url'],text='',already_supplied=True))
            continue
        part=read_remote_reference(agent,dict(url=item['url'],length=min(24000,remaining)),include_dependencies=False)
        if part.get('cached_path'):
            full=agent.host.path('work',part['cached_path']).read_text('utf-8')
            excerpt=dependency_excerpt(full,calls,remaining)
            part.update(excerpt)
            if excerpt.get('excerpt'):part['next_offset']=None
        text=part.get('text','')
        if text:
            result.append(part)
            remaining-=len(text)
            if not part.get('next_offset'):supplied.add(item['url'])
        if remaining<=0:break
    agent.remote_reference_dependencies=supplied
    return result


def read_remote_reference(agent,args,*,include_dependencies=True):
    revision,url=source_coordinates(args['url'])
    if revision not in documented_revisions(agent):
        raise ValueError('This repository revision is not cited by the installed Skill; use its documented fixed source link')
    if args.get('pattern'):
        return remote_source_tree(agent,revision,args)
    relative='reference-sources/'+hashlib.sha256(url.encode()).hexdigest()+'.txt'
    path=agent.host.path('work',relative)
    if not path.exists():
        # 只读公开文本，不发送账号凭据；禁止把允许的源码域重定向到其他服务。
        try:
            raw=reference_bytes(agent,url,512*1024)
        except urllib.error.HTTPError as error:
            status=error.code
            error.close()
            if status!=404:raise
            return dict(remote_source_tree(agent,revision,{}),missing_source=url,
                        message='Source path not found. Choose the actual definition from these source paths; url+pattern can filter this same fixed repository tree.')
        if len(raw)>512*1024 or b'\0' in raw:
            raise ValueError('Reference must be a text source file no larger than 512 KiB')
        value=raw.decode('utf-8-sig')
        path.parent.mkdir(exist_ok=True,parents=True)
        path.write_text(value,encoding='utf-8')
        agent.api['atomic_json'](path.with_suffix('.json'),dict(url=url,sha256=hashlib.sha256(raw).hexdigest(),bytes=len(raw)))
    value=path.read_text('utf-8')
    offset=max(0,int(args.get('offset',0)))
    if args.get('search') and 'offset' not in args:
        found=value.lower().find(args['search'].lower())
        if found<0:return dict(member=url,matches=[],message='Search text absent from this source')
        offset=max(0,found-400)
    length=min(24000,max(1,int(args.get('length',24000))))
    result=dict(member=url,text=value[offset:offset+length],cached_path=relative,native_bindings=primitive_bindings(value),
                symbols=[dict(offset=m.start(),declaration=m.group().strip()) for m in re.finditer(
                    r'(?m)^\s*(?:(?:public|private|protected|internal|static|override|async)\s+)+[^\n;{}=]+\([^\n]*|^\s*(?:def|fn)\s+[^\n]+',value)][:100],
                next_offset=offset+length if offset+length<len(value) else None)
    if include_dependencies:
        try:result['dependencies']=remote_dependencies(agent,revision,url,value)
        except (OSError,ValueError,KeyError) as error:
            result['dependencies_error']='Auxiliary source lookup failed; read the required definitions explicitly: '+str(error)[:400]
    return result


def pe_resources(data):
    """静态读取 PE 资源树，不加载或执行游戏程序。"""
    import pefile
    if len(data) > MAX_INPUT or data[:2] != b'MZ':
        raise ValueError('pe_resources expects bounded PE bytes')
    pe = pefile.PE(data=data, fast_load=True)
    try:
        pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_RESOURCE']])
        result, total = [], 0
        def key(entry):
            return str(entry.name) if entry.name is not None else entry.id
        for kind in getattr(getattr(pe, 'DIRECTORY_ENTRY_RESOURCE', None), 'entries', []):
            for name in kind.directory.entries:
                for language in name.directory.entries:
                    item = language.data.struct
                    total += item.Size
                    if total > MAX_OUTPUT or len(result) >= 20000:
                        raise ValueError('PE resources exceed their budget')
                    value = pe.get_data(item.OffsetToData, item.Size)
                    if len(value) != item.Size:
                        raise ValueError('Truncated PE resource')
                    result.append(dict(type=key(kind), name=key(name), language=key(language), data=value))
        return result
    finally:
        pe.close()


def cipher(algorithm, data, key, mode='ecb', iv=None, encrypt=False):
    """标准块密码原语；不识别引擎、不寻找密钥、不猜字节序或填充方式。"""
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
    from cryptography.hazmat.decrepit.ciphers.algorithms import Blowfish
    if not isinstance(data, bytes) or len(data) > MAX_OUTPUT:
        raise ValueError('cipher needs bytes within the output budget')
    implementation = {'blowfish': Blowfish, 'aes': algorithms.AES}.get(algorithm)
    if implementation is None or mode not in ('ecb', 'cbc'):
        raise ValueError('Supported ciphers: blowfish/aes; modes: ecb/cbc. No implicit padding.')
    operation = Cipher(implementation(key), modes.ECB() if mode == 'ecb' else modes.CBC(iv))
    context = operation.encryptor() if encrypt else operation.decryptor()
    return context.update(data) + context.finalize()


def reference_primitive(algorithm, items, *, bridge=None):
    """批量复用随附库的历史算法实现，避免模型重复移植；不包含档案格式逻辑。"""
    if algorithm not in ('garbro_mt19937','garbro_blowfish','garbro_crc32normal_table') or not isinstance(items,list) or len(items) > 20000:
        raise ValueError('Choose garbro_mt19937, garbro_blowfish or garbro_crc32normal_table with 1..20000 items')
    if not items:return []
    if algorithm=='garbro_crc32normal_table':
        if items != [{}]:
            raise ValueError('garbro_crc32normal_table takes items=[{}] and returns one 256-entry uint32 table')
    values, total = [], 0
    for item in items:
        if algorithm == 'garbro_crc32normal_table':
            values.append({})
        elif algorithm == 'garbro_mt19937':
            seed, count = item['seed'], item.get('count',1)
            if type(seed) is not int or not 0 <= seed <= 0xffffffff or type(count) is not int or not 1 <= count <= 32:
                raise ValueError('PRNG needs a uint32 seed and count in 1..32')
            values.append(dict(seed=seed,count=count))
        else:
            key, data = item['key'], item['data']
            if not isinstance(key,bytes) or not 4 <= len(key) <= 56 or not isinstance(data,bytes) or len(data)%8:
                raise ValueError('Blowfish needs 4..56 key bytes and block-aligned data; process any tail separately')
            total += len(data)
            if total > MAX_OUTPUT:
                raise ValueError('Reference cipher batch exceeds 64 MiB')
            values.append(dict(key=base64.b64encode(key).decode(),data=base64.b64encode(data).decode(),encrypt=bool(item.get('encrypt',False))))
    if algorithm=='garbro_blowfish' and any(not item['data'] for item in items):
        # 空成员没有块可解密；某些旧原语拒绝零长度，保留原样空输出即可。
        decoded=iter(reference_primitive(algorithm,[item for item in items if item['data']],bridge=bridge))
        return [next(decoded) if item['data'] else b'' for item in items]
    payload=dict(algorithm=algorithm,items=values)
    if bridge is None:
        with NATIVE['ReferenceBridge']() as current:
            result=current.request(payload)
    else:
        result=bridge.request(payload)
    if len(result) != len(values):
        raise ValueError('Reference primitive returned the wrong batch size')
    return [base64.b64decode(value,validate=True) for value in result] if algorithm=='garbro_blowfish' else result


def install_primitives(api,bridge=None):
    module = types.ModuleType('galgear_binary')
    # 生命周期和总时限由外层研究工作进程管理，不因第十三次合法计算截断算法。
    module.pe_resources, module.cipher = pe_resources, cipher
    module.reference_primitive = reference_primitive if bridge is None else lambda algorithm,items:reference_primitive(algorithm,items,bridge=bridge)
    api['MODULES']['galgear_binary'] = module
    api['ATTRIBUTES'].update(('pe_resources', 'cipher', 'reference_primitive'))


def referenced_types(source):
    """提取源码显式引用但未在本文件定义的辅助类型。"""
    names = list(dict.fromkeys(re.findall(r'\b([A-Z]\w*)\.Table\b', source)
                             + re.findall(r'\bnew\s+([A-Z]\w*)(?:\.[A-Z]\w*)*\s*\(', source)))
    return [name for name in names if not re.search(r'\b(?:class|struct|enum)\s+'+re.escape(name)+r'\b',source)]


def dependency_excerpt(text,calls,remaining):
    """大型辅助类优先展示实际被调用的方法，避免常量表挤掉接口语义。"""
    if len(text)<=min(remaining,8000):return dict(text=text)
    snippets=[]
    if calls:
        signatures=re.compile(r'(?m)^\s*(?:public|private|protected|internal)[^\n;{}]*\b(?:'+ '|'.join(map(re.escape,calls))+r')\s*\(')
        for match in signatures.finditer(text):
            snippet=text[match.start():match.start()+1800]
            if sum(len(s) for s in snippets)+len(snippet)>min(remaining,5000):break
            snippets.append(snippet)
    return dict(text='\n// ... source excerpt ...\n'.join(snippets),excerpt=True,requires_read=True)


def source_dependencies(archive, member, source):
    """补齐源码显式引用的辅助类型，避免按类名猜测算法；只读同一 ZIP，预算固定。"""
    names=referenced_types(source)
    if not names:
        return []
    declaration = re.compile(r'\b(?:class|struct)\s+('+ '|'.join(map(re.escape,names))+r')\b')
    candidates = [i for i in archive.infolist() if i.filename != member and
                  Path(i.filename).suffix.lower() in ('.cs','.cpp','.h','.rs','.py') and i.file_size <= 2*1024*1024]
    candidates.sort(key=lambda i: not any(name.lower().startswith(Path(i.filename).stem.lower()) for name in names))
    result, scanned, remaining, found = [], 0, 32000, set()
    calls=set(re.findall(r'\.\s*(\w+)\s*\(',source))
    for info in candidates:
        scanned += info.file_size
        if scanned > 32*1024*1024:
            break
        text = archive.read(info).decode('utf-8-sig','replace')
        symbols = set(declaration.findall(text)) - found
        if not symbols:
            continue
        excerpt=dependency_excerpt(text,calls,remaining)
        result.append(dict(member=info.filename,symbols=sorted(symbols),**excerpt))
        remaining-=len(excerpt['text'])
        found.update(symbols)
        if len(result)>=6 or found.issuperset(names):
            break
    return result


def primitive_bindings(source):
    """将已封装的库符号绑定到宿主入口，不包含任何引擎密钥或格式参数。"""
    result={}
    if 'GameRes.Cryptography' in source:
        if re.search(r'\bBlowfish\b',source):
            result['GameRes.Cryptography.Blowfish.Decipher']='reference_primitive("garbro_blowfish", [{"key": key_bytes, "data": aligned_bytes, "encrypt": False}])[0]. Use this for EVERY call to that reference implementation, including wrapped key/passphrase bytes as well as index/member data. It uses little-endian word pairs; cipher("blowfish", ...) is a DIFFERENT standard byte convention and is not a drop-in replacement. Source still determines keys, ranges and tail handling.'
        if re.search(r'\bMersenneTwister\b',source):
            result['GameRes.Cryptography.MersenneTwister.Rand']='reference_primitive("garbro_mt19937", [{"seed": native_uint32_seed, "count": 1}])[0][0]; batch items in one call.'
    if 'GameRes.Utility' in source and re.search(r'\bCrc32Normal\b',source):
        result['GameRes.Utility.Crc32Normal.Table']='reference_primitive("garbro_crc32normal_table", [{}])[0]; apply the actual caller arithmetic, not zlib.crc32.'
    return result


def read_reference(agent, args):
    """按成员名或搜索词读取随 Skill 安装的源码 ZIP；不解压到文件系统。"""
    if not args.get('url') and str(args.get('path','')).startswith('https://'):
        args=dict(args,url=args['path'])
    if args.get('url'):
        return read_remote_reference(agent,args)
    root=args.get('root','skill')
    requested=str(args.get('path','')).replace('\\','/')
    directory=agent.host.roots[root]
    absolute=Path(requested)
    if absolute.is_absolute() and absolute.resolve().is_relative_to(directory.resolve()):
        requested=absolute.resolve().relative_to(directory.resolve()).as_posix()
    for prefix in (root+'/',directory.name+'/'):
        if requested.startswith(prefix):requested=requested[len(prefix):]
    args=dict(args,root=root,path=requested)
    path = agent.host.path(root,requested)
    if path.is_dir():
        return agent.host._dispatch('list_files',dict(root=root,path=requested))
    if path.suffix.lower() != '.zip':
        return agent.host._dispatch('read_text',dict(root=args.get('root','skill'),path=args['path'],
            offset=args.get('offset',0),length=args.get('length',8192)))
    offset = max(0, int(args.get('offset', 0)))
    length = min(24000, max(1, int(args.get('length', 24000))))
    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        if len(entries) > 100000:
            raise ValueError('Source ZIP has too many entries')
        if args.get('member'):
            member=args['member'].replace('\\','/')
            candidates=([entry for entry in entries if entry.filename==member] or
                        [entry for entry in entries if entry.filename.endswith('/'+member)])
            if len(candidates)!=1:
                return dict(matches=[dict(member=entry.filename) for entry in entries if Path(entry.filename).name==Path(member).name],
                    message='Select the unique full member path from these actual source entries')
            info = candidates[0]
            if info.file_size > 2 * 1024 * 1024:
                raise ValueError('Select a source member smaller than 2 MiB')
            value = archive.read(info).decode('utf-8-sig', 'replace')
            if args.get('search') and 'offset' not in args:
                found = value.lower().find(args['search'].lower())
                if found < 0:
                    return dict(member=info.filename,matches=[],message='Search text absent from this member')
                offset = max(0,found-400)
            dependencies=source_dependencies(archive,info.filename,value)
            supplied=getattr(agent,'reference_dependencies',set())
            for dependency in dependencies:
                identity=(str(path),dependency['member'])
                if identity in supplied:
                    dependency['text']=''
                    dependency['already_supplied']=True
                elif dependency.get('text'):
                    supplied.add(identity)
            agent.reference_dependencies=supplied
            return dict(member=info.filename, text=value[offset:offset+length],
                        dependencies=dependencies,
                        native_bindings=primitive_bindings(value),
                        symbols=[dict(offset=m.start(), declaration=m.group().strip()) for m in re.finditer(
                            r'(?m)^\s*(?:(?:public|private|protected|internal|static|override|async)\s+)+[^\n;{}=]+\([^\n]*|^\s*(?:def|fn)\s+[^\n]+',value)][:100],
                        next_offset=offset+length if offset+length < len(value) else None)
        candidates = [i for i in entries if not i.is_dir() and
                      fnmatch.fnmatchcase(i.filename.lower(), args.get('pattern', '*').lower())]
        if not args.get('search'):
            return dict(members=[i.filename for i in candidates[offset:offset+200]], total=len(candidates),
                        next_offset=offset+200 if offset+200 < len(candidates) else None)
        result, total = [], 0
        for info in candidates:
            if info.file_size > 2 * 1024 * 1024 or Path(info.filename).suffix.lower() not in ('.py','.cs','.cpp','.h','.rs','.md','.txt','.c'):
                continue
            total += info.file_size
            if total > 32 * 1024 * 1024:
                raise ValueError('Narrow source search using pattern; scan exceeds 32 MiB')
            value = archive.read(info).decode('utf-8-sig', 'replace')
            found = value.lower().find(args['search'].lower())
            if found >= 0:
                result.append(dict(member=info.filename, offset=max(0,found-400), text=value[max(0,found-400):found+1600]))
                if len(result) >= 8:
                    break
        return dict(matches=result)


def files_checked(files, api):
    if not isinstance(files, dict) or not 1 <= len(files) <= 20000:
        raise ValueError('files must contain 1..20000 member names mapped to bytes')
    names = api['ARCHIVE_TOOLS']['safe_members'](files)
    binary_types=(bytes,bytearray,memoryview,mmap.mmap)
    if sum(len(value) for value in files.values() if isinstance(value, binary_types)) > MAX_OUTPUT:
        raise ValueError('Selected output exceeds 64 MiB; extract only scripts or a smaller set')
    if any(not isinstance(value, binary_types) for value in files.values()):
        raise ValueError('Each member must be bytes, not base64 or decoded text')
    if not any(files.values()):
        raise ValueError('Every emitted member is empty. No script was decoded. Validate the record/table offsets and decode one nonempty sample before bulk conversion; use read_resources with a custom reader on the original emitted binary scripts.')
    folded = {name.casefold() for name in names}
    for name in names:
        parts = name.casefold().split('/')
        if any('/'.join(parts[:i]) in folded for i in range(1,len(parts))):
            raise ValueError('A member cannot also be a directory')
    return {name:bytes(value) for name,value in zip(names,files.values())}


def worker(config, api):
    with NATIVE['ReferenceBridge']() as bridge, contextlib.ExitStack() as resources:
        install_primitives(api,bridge)
        return _worker(config,api,resources)


def _worker(config,api,resources):
    inputs, receipts, stamps, total = {}, [], [], 0
    for item in config['inputs']:
        path = Path(item['absolute'])
        before=path.stat()
        offset=item.get('offset',0)
        length=item.get('length',before.st_size-offset)
        if (type(offset) is not int or type(length) is not int or offset<0 or length<=0
                or offset+length>before.st_size):
            raise ValueError('Input range must have nonnegative offset and positive length within the original file')
        mapped=total+length>MAX_INPUT
        if mapped:
            # 大包保持只读映射，struct 和切片可直接复用现有解析程序；无需为工具容量重写范围调度。
            # ACCESS_READ 同时阻止模型通过下标赋值修改用户文件；句柄由整次程序作用域回收。
            stream=resources.enter_context(path.open('rb'))
            aligned=offset-offset%mmap.ALLOCATIONGRANULARITY
            view=resources.enter_context(mmap.mmap(stream.fileno(),length+offset-aligned,
                                                  access=mmap.ACCESS_READ,offset=aligned))
            value=view if offset==aligned else resources.enter_context(memoryview(view)[offset-aligned:])
        else:
            total += length
            with path.open('rb') as stream:
                stream.seek(offset)
                value=stream.read(length)
        after=path.stat()
        if len(value)!=length or (before.st_size,before.st_mtime_ns)!=(after.st_size,after.st_mtime_ns):
            raise ValueError('Input changed during read')
        inputs[item['id']] = value
        stamps.append((path,before.st_size,before.st_mtime_ns))
        receipts.append(dict(id=item['id'], root=item['root'], path=item['path'], size=len(value),
                             offset=offset,file_size=before.st_size,
                             access='mapped_read_only' if mapped else 'bytes',
                             head_hex=value[:96].hex(),
                             sha256=hashlib.sha256(value).hexdigest()))
    source = config['source']
    metadata = dict(config.get('metadata', {}))
    metadata['input_ranges']={item['id']:dict(offset=item['offset'],length=item['size'],file_size=item['file_size']) for item in receipts}
    value = api['execute_pure'](source, 'analyze', inputs, metadata)
    for path,size,modified in stamps:
        after=path.stat()
        if (after.st_size,after.st_mtime_ns)!=(size,modified):raise ValueError('Input changed during format program')
    if not isinstance(value, dict):
        value=dict(result=value)
    files = files_checked(value['files'], api) if value.get('files') else {}
    # 探查代码自然返回普通 JSON；不能把有效观察静默丢成 None。
    result = value.get('result',{key:child for key,child in value.items() if key!='files'})
    if not files and result in (None,{}):
        raise ValueError('Program returned no probe result or emitted members')
    if len(json.dumps(result, ensure_ascii=False).encode()) > 1024 * 1024:
        raise ValueError('Probe result exceeds 1 MiB; summarize tables in result and emit full data as files')
    roundtrip = None
    if config.get('verify_roundtrip'):
        if not files:
            raise ValueError('Roundtrip requires extracted files')
        # 比较副本，防止模型通过修改传入字典伪造相等；只证明其自身编解码的一致性。
        packed = api['execute_pure'](source, 'pack', dict(files), dict(metadata))
        if not isinstance(packed, bytes) or len(packed) > MAX_OUTPUT:
            raise ValueError('pack must return bytes within 64 MiB')
        restored = files_checked(api['execute_pure'](source, 'unpack', packed, dict(metadata)), api)
        if files != restored:
            raise ValueError('Self roundtrip failed: names or complete member bytes differ')
        roundtrip = dict(level='self_roundtrip', sha256=hashlib.sha256(packed).hexdigest(),
                         size=len(packed), note='Does not prove independent tool or game compatibility')
    output = Path(config['output'])
    output.mkdir(exist_ok=False, parents=True)
    manifest = []
    for name, data in files.items():
        destination = output / name
        destination.parent.mkdir(exist_ok=True, parents=True)
        destination.write_bytes(data)
        manifest.append(dict(path=name, size=len(data), sha256=hashlib.sha256(data).hexdigest()))
    receipt = dict(inputs=receipts, program_sha256=hashlib.sha256(source.encode()).hexdigest(),
                   result=result, members=manifest, roundtrip=roundtrip,
                   validation='bounded_outputs_only; read_resources must verify script structure')
    api['atomic_json'](output.with_suffix('.json'), receipt)
    return dict(result=result, input_headers=receipts, count=len(files), sample_members=manifest[:8], roundtrip=roundtrip,
                validation=receipt['validation'])


def run_program(agent, args):
    args = dict(args)
    if not args.get('inputs') and args.get('program'):
        previous = agent.host.path('work',args['program']).with_suffix('.request.json')
        if previous.is_file():
            saved = json.loads(previous.read_text('utf-8'))
            args['inputs'] = saved['inputs']
            args.setdefault('metadata',saved.get('metadata',{}))
    items = args.get('inputs', [])
    if not 1 <= len(items) <= 16 or len({i['id'] for i in items}) != len(items):
        raise ValueError('Provide 1..16 inputs with unique ids')
    resolved = []
    for item in items:
        if item['root'] not in agent.host.roots or item['root'] in ('skill','reverse','pack'):
            raise ValueError('Format inputs must be game, work, or save roots')
        path = agent.host.path(item['root'], item['path'])
        if not path.is_file():
            raise ValueError('Input must be an existing regular file')
        agent.stamp(path)
        resolved.append(dict(item, absolute=str(path)))
    source = args.get('source')
    if not source:
        source = agent.host.path('work', args['program']).read_text('utf-8')
    for edit in args.get('edits', []):
        old, new = edit['old'], edit['new']
        if not old or source.count(old) != 1:
            raise ValueError('Each edit.old must match exactly once in the saved program; read_text the program before correcting it')
        source = source.replace(old,new,1)
    install_primitives(agent.api)
    identifier = uuid.uuid4().hex[:12]
    directory = agent.work / 'format-programs'
    if not directory.resolve().is_relative_to(agent.work):
        raise ValueError('Program directory escapes the current work root')
    directory.mkdir(exist_ok=True)
    program = directory / (identifier+'.py')
    program.write_text(source, encoding='utf-8')
    agent.api['atomic_json'](program.with_suffix('.request.json'),dict(inputs=items,metadata=args.get('metadata',{})))
    try:
        agent.api['validate_program'](source)
    except (ValueError,SyntaxError) as error:
        raise ValueError('Format program '+str(program.relative_to(agent.work))+': '+str(error)) from error
    output = agent.work / 'format-output' / identifier
    if not output.resolve().is_relative_to(agent.work):
        raise ValueError('Output directory escapes the current work root')
    payload = dict(source=source, inputs=resolved, output=str(output), metadata=args.get('metadata', {}),
                   verify_roundtrip=args.get('verify_roundtrip', False))
    # 开发解释器需要已安装的纯计算依赖；模型仍只能 import 白名单模块。
    command = [arg for arg in agent.host.reader_command if arg != '-S']
    completed = subprocess.run([*command, '--format-program'], input=json.dumps(payload).encode(),
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=min(90,max(1,agent.deadline-time.monotonic())),
        creationflags=0x08000000 if os.name == 'nt' else 0)
    relative = str(program.relative_to(agent.work))
    if completed.returncode:
        raise RuntimeError('Format program '+relative+': '+completed.stderr.decode('utf-8','replace')[-3500:])
    result = json.loads(completed.stdout)
    result.update(program=relative, output=str(output.relative_to(agent.work)),
                  inputs=items,
                  manifest=str(output.with_suffix('.json').relative_to(agent.work)),
                  next_step=('Reuse these emitted ORIGINAL members directly with read_resources(reader="custom", options={source:"def extract(data, metadata): ..."}) when no native reader exists. Keep unpacking separate from script decoding: return ordered records with native byte offsets, type and text; do not rerun the archive decryptor just to change a parser or replace binary members with a text dump. Read actual layout definitions: computed properties are not necessarily serialized fields.' if result.get('count') else
                             'Probe only: no files emitted. If scripts are already decoded, return their raw bytes in files and register them with read_resources NOW; do not keep the entire script set only in result JSON. Otherwise reject out-of-range offsets/unreadable names and compare code with reference source.'))
    return result


def tool_schemas(api):
    string = dict(type='string')
    return [api['tool']('read_reference', 'Read source evidence: provide path for an installed source ZIP, or url for a fixed-commit GitHub source link cited in the installed Skill. Related source files in that same repository/commit are allowed; url + pattern lists matching source paths from that fixed repository tree. Missing remote files also return the tree. Remote text is cached with provenance. For ZIP, omit member to list paths (pattern glob); search returns snippets; member + offset/length reads source and helper definitions. Follow next_offset. root is skill/reverse/pack.',
        dict(root=string,path=string,url=string,member=string,pattern=string,search=string,offset=dict(type='integer'),length=dict(type='integer')), []),
        api['tool']('run_format_program', 'Author/reuse a pure Python unpacker or format probe. def analyze(inputs, metadata) gets {id: bytes-like} for explicit read-only inputs and returns {result: JSON, files: {relative_name: bytes}} (files optional). Original game/saves cannot be written. Source and output receipts are persisted. Use installed reverse/pack Skill and tool source, not guessed keys. Allowed pure imports as inspect_data PLUS galgear_binary: pe_resources(data)->list[{type,name,language,data:bytes}], cipher(algorithm,data,key,mode="ecb",iv=None,encrypt=False) for standard blowfish/aes, no padding/endian conversion. Implement format-specific indexing/decryption in source; filter scripts in program. Large archives are automatically memory-mapped READ ONLY: len, indexing, slicing and struct.unpack_from work without splitting the program into range requests. Slices of a whole-file mapping are bytes; convert a ranged buffer slice with bytes() when needed. Explicit offset/length remains available; metadata.input_ranges[id]={offset,length,file_size}. Heap-read inputs are bounded to 192 MiB; larger inputs use mapping. Emitted members stay within 64 MiB and the program within 90 seconds. Select only referenced scripts/UI members before decrypting payloads, not unrelated art. Subsequent calls reuse returned program path; correct failed source before retry. Optional verify_roundtrip requires def pack(files,metadata)->bytes and def unpack(data,metadata)->{name:bytes}; compares ALL output bytes but proves only self-consistency, not runtime compatibility. Then use read_resources/query_data for actual script validation.',
        dict(inputs=dict(type='array', minItems=1,maxItems=16,items=dict(type='object',properties=dict(id=string,root=string,path=string,
             offset=dict(type='integer',minimum=0,description='Optional file byte offset; combine with length for large archive index/member reads.'),
             length=dict(type='integer',minimum=1,description='Optional byte count. metadata.input_ranges[id] supplies original file_size, offset and length.')),
             required=['id','root','path'])),
             source=string,program=dict(type='string',description='Reuse returned saved program path. Optional edits performs exact single-match replacements, avoiding a full rewrite.'),
             edits=dict(type='array',maxItems=20,items=dict(type='object',properties=dict(old=string,new=string),required=['old','new'])),
             metadata=dict(type='object'),verify_roundtrip=dict(type='boolean')), [])]
